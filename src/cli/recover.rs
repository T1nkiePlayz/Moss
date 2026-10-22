//! `moss recover`: repair tools that run offline against the data directory. They refuse to run while a controller
//! holds the data-directory lock (see `lock`) unless `--force` is given. Each action is a small function over a
//! `Connection` that returns the text to print, so it is unit tested without a process.
use crate::{auth::setup, backups, db};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Args, Clone, Debug)]
pub struct Recover {
    /// Data directory (the same one `moss serve` uses).
    #[arg(long, env = "MOSS_DATA_DIR", default_value_os_t = crate::config::default_data_dir())]
    pub data_dir: PathBuf,
    /// Run even though a controller appears to be running. Changes can then be lost or overwritten; stop Moss first if you can.
    #[arg(long)]
    pub force: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Clone, Debug)]
pub enum Cmd {
    /// Change who the owner is (use after moving to a different Supabase project)
    Owner { #[command(subcommand)] cmd: OwnerCmd },
    /// Mark operations that are stuck as queued or running as failed
    Operations { #[command(subcommand)] cmd: OpsCmd },
    /// Create a fresh setup token for claiming ownership
    SetupToken { #[command(subcommand)] cmd: TokenCmd },
    /// List the servers in the database
    Servers { #[command(subcommand)] cmd: ServersCmd },
    /// Check a backup archive against its recorded checksum
    Backup { #[command(subcommand)] cmd: BackupCmd },
    /// Check the database file
    Db { #[command(subcommand)] cmd: DbCmd },
    /// Revoke every API token
    Tokens { #[command(subcommand)] cmd: TokensCmd },
    /// Print a JSON summary of the configuration (no secrets) for moving or documenting an installation
    ExportConfig {
        /// Write to this file instead of the terminal
        #[arg(long)]
        out: Option<PathBuf>,
    },
}
#[derive(Subcommand, Clone, Debug)]
pub enum OwnerCmd {
    /// Make this e-mail the owner. They link to their new account the next time they sign in.
    Set { email: String },
    /// Demote every owner to admin so the instance can be claimed again with a setup token
    Clear,
}
#[derive(Subcommand, Clone, Debug)]
pub enum OpsCmd { /// Mark queued and running operations (and backups in progress) as failed
    Reset }
#[derive(Subcommand, Clone, Debug)]
pub enum TokenCmd {
    /// Write a new setup token file
    New {
        /// Also demote the current owners, so the new token can be used right away
        #[arg(long)]
        unclaim: bool,
    },
}
#[derive(Subcommand, Clone, Debug)]
pub enum ServersCmd { /// One line per server
    List }
#[derive(Subcommand, Clone, Debug)]
pub enum BackupCmd {
    /// Verify one backup by its id, or every backup with `all`
    Verify { id: String },
}
#[derive(Subcommand, Clone, Debug)]
pub enum DbCmd { /// SQLite integrity check and schema version
    Check }
#[derive(Subcommand, Clone, Debug)]
pub enum TokensCmd { /// Delete all personal API tokens
    RevokeAll }

pub fn run(a: Recover) -> Result<()> {
    // Hold the lock for the whole run: it keeps a controller from starting halfway through a repair.
    let _lock = match crate::lock::try_acquire(&a.data_dir).context("checking whether Moss is running")? {
        Some(l) => Some(l),
        None if a.force => { eprintln!("warning: a Moss controller is running and --force was given; continuing anyway."); None }
        None => bail!("Moss appears to be running for {}. Stop it first (for example `systemctl stop moss`), or pass --force if you are sure it is not.", a.data_dir.display()),
    };
    let dbp = a.data_dir.join("moss.sqlite");
    let out = match &a.cmd {
        Cmd::Db { cmd: DbCmd::Check } => db_check(&db::open_existing(&dbp)?)?,
        Cmd::Owner { cmd: OwnerCmd::Set { email } } => owner_set(&db::open_existing(&dbp)?, email)?,
        Cmd::Owner { cmd: OwnerCmd::Clear } => owner_clear(&db::open_existing(&dbp)?)?,
        Cmd::Operations { cmd: OpsCmd::Reset } => operations_reset(&db::open_existing(&dbp)?)?,
        Cmd::SetupToken { cmd: TokenCmd::New { unclaim } } => setup_token_new(&db::open_existing(&dbp)?, &a.data_dir, *unclaim)?,
        Cmd::Servers { cmd: ServersCmd::List } => servers_list(&db::open_existing(&dbp)?, &a.data_dir)?,
        Cmd::Backup { cmd: BackupCmd::Verify { id } } => backup_verify(&db::open_existing(&dbp)?, &a.data_dir, id)?,
        Cmd::Tokens { cmd: TokensCmd::RevokeAll } => tokens_revoke_all(&db::open_existing(&dbp)?)?,
        Cmd::ExportConfig { out } => {
            let v = serde_json::to_string_pretty(&export_config(&db::open_existing(&dbp)?)?)?;
            match out {
                Some(p) => { std::fs::write(p, v + "\n").with_context(|| format!("writing {}", p.display()))?; format!("Configuration written to {}", p.display()) }
                None => v,
            }
        }
    };
    println!("{out}");
    Ok(())
}

fn valid_email(e: &str) -> bool { e.len() <= 254 && e.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !e.chars().any(|c| c.is_whitespace() || c.is_control())) }

pub fn owner_set(c: &Connection, email: &str) -> Result<String> {
    let email = email.trim().to_lowercase();
    if !valid_email(&email) { bail!("`{email}` does not look like an e-mail address"); }
    let tx = c.unchecked_transaction()?;
    // linked=0 means "not yet tied to a sign-in": the next sign-in with this e-mail takes over the row and keeps its grants
    let n = tx.execute("UPDATE users SET role='owner', linked=0 WHERE email=?", [&email])?;
    if n == 0 { tx.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,'owner',0,?)", params![format!("recovered-{}", uuid::Uuid::new_v4()), email, db::now()])?; }
    let others: Vec<String> = { let mut q = tx.prepare("SELECT email FROM users WHERE role='owner' AND email<>? ORDER BY email")?; q.query_map([&email], |r| r.get(0))?.collect::<Result<_, _>>()? };
    tx.commit()?;
    let mut s = format!("{email} is now the owner. They are linked to their account the next time they sign in with this e-mail address (it must be verified in the new project).");
    if !others.is_empty() { s += &format!("\nOther owners still exist: {}. Use `moss recover owner clear` first if you want only {email}.", others.join(", ")); }
    Ok(s)
}

pub fn owner_clear(c: &Connection) -> Result<String> {
    let n = c.execute("UPDATE users SET role='admin' WHERE role='owner'", [])?;
    Ok(if n == 0 { "There was no owner.".into() } else { format!("{n} owner(s) are now administrators. Start Moss and enter the setup token (see `moss recover setup-token new`) to claim ownership.") })
}

pub fn operations_reset(c: &Connection) -> Result<String> {
    let now = db::now();
    let ops = c.execute("UPDATE operations SET state='failed', stage='Failed', error='marked failed by moss recover', ended_at=? WHERE state IN ('queued','running')", [&now])?;
    let backups = c.execute("UPDATE backups SET status='failed' WHERE status IN ('running','verifying')", [])?;
    Ok(format!("{ops} stuck operation(s) and {backups} unfinished backup(s) marked failed."))
}

pub fn setup_token_new(c: &Connection, data_dir: &Path, unclaim: bool) -> Result<String> {
    let mut msg = String::new();
    if unclaim { msg += &owner_clear(c)?; msg.push('\n'); }
    let owners: i64 = c.query_row("SELECT COUNT(*) FROM users WHERE role='owner'", [], |r| r.get(0))?;
    let token = crate::auth::secret::random_string(setup::TOKEN_LEN);
    let path = data_dir.join("setup-token");
    setup::write_token_file(&path, &token).with_context(|| format!("writing {}", path.display()))?;
    msg += &format!("New setup token written to {} (readable only by you).", path.display());
    if owners > 0 { msg += "\nNote: an owner still exists, so Moss will delete this file on start. Use --unclaim to remove the current owner(s) first."; }
    Ok(msg)
}

pub fn servers_list(c: &Connection, data_dir: &Path) -> Result<String> {
    let mut q = c.prepare("SELECT id,name,runtime,game_version,port,desired,installed FROM servers ORDER BY created_at")?;
    let rows: Vec<(String, String, String, String, i64, String, bool)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?.collect::<Result<_, _>>()?;
    if rows.is_empty() { return Ok("No servers.".into()); }
    let mut s = String::from("ID                                    NAME                  RUNTIME    VERSION    PORT   WANTED   FILES\n");
    for (id, name, rt, gv, port, desired, installed) in rows {
        let dir = if !installed { "not installed" } else if data_dir.join("servers").join(&id).is_dir() { "ok" } else { "MISSING" };
        s += &format!("{id:<37} {:<21} {rt:<10} {gv:<10} {port:<6} {desired:<8} {dir}\n", name.chars().take(21).collect::<String>());
    }
    Ok(s.trim_end().into())
}

pub fn backup_verify(c: &Connection, data_dir: &Path, id: &str) -> Result<String> {
    let mut q = c.prepare("SELECT id,server_id,sha256,status FROM backups WHERE (id=?1 OR ?1='all') AND status='ok' AND local_present=1 ORDER BY created_at")?;
    let rows: Vec<(String, String, String, String)> = q.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    if rows.is_empty() { bail!("no completed backup with id `{id}` (use `moss recover servers list` and the Backups page to find ids)"); }
    let (mut out, mut bad) = (String::new(), 0);
    for (bid, sid, sha, _) in rows {
        let p = data_dir.join("backups").join(&sid).join(format!("{bid}.tar.zst"));
        match backups::verify_archive(&p, &sha) {
            Ok(n) => { out += &format!("ok      {bid}: {n} entries, checksum matches\n"); c.execute("UPDATE backups SET verified_at=? WHERE id=?", params![db::now(), bid])?; }
            Err(e) => { bad += 1; out += &format!("FAILED  {bid}: {e:#}\n"); }
        }
    }
    if bad > 0 { bail!("{out}{bad} backup(s) failed verification"); }
    Ok(out.trim_end().into())
}

pub fn db_check(c: &Connection) -> Result<String> {
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let latest = db::latest_migration() as i64;
    let mut q = c.prepare("PRAGMA integrity_check")?;
    let msgs: Vec<String> = q.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let fk: i64 = c.prepare("PRAGMA foreign_key_check")?.query_map([], |_| Ok(()))?.count() as i64;
    let mut s = format!("schema version {v} (this Moss expects {latest})\nintegrity check: {}\nforeign key violations: {fk}", msgs.join("; "));
    let mut problems = vec![];
    if msgs != ["ok"] { problems.push("the database file is damaged; restore moss.sqlite from a copy"); }
    if v > latest { problems.push("the database was written by a newer Moss; upgrade Moss"); }
    if fk > 0 { problems.push("some rows point at things that no longer exist"); }
    if v < latest { s += "\nnote: the schema is older than this Moss; it is upgraded automatically on the next start."; }
    if !problems.is_empty() { bail!("{s}\nproblems: {}", problems.join("; ")); }
    Ok(s)
}

pub fn tokens_revoke_all(c: &Connection) -> Result<String> {
    Ok(format!("{} API token(s) revoked.", c.execute("DELETE FROM api_tokens", [])?))
}

/// Everything needed to rebuild the setup on another machine, minus secrets (token hashes, webhook URL values).
pub fn export_config(c: &Connection) -> Result<Value> {
    let rows = |sql: &str, cols: &[&str]| -> Result<Vec<Value>> {
        let mut q = c.prepare(sql)?;
        let n = cols.len();
        let v = q.query_map([], |r| { let mut o = serde_json::Map::new(); for (i, k) in cols.iter().enumerate().take(n) { o.insert((*k).into(), match r.get_ref(i)? { rusqlite::types::ValueRef::Null => Value::Null, rusqlite::types::ValueRef::Integer(x) => json!(x), rusqlite::types::ValueRef::Real(x) => json!(x), rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)), rusqlite::types::ValueRef::Blob(_) => Value::Null }); } Ok(Value::Object(o)) })?.collect::<Result<_, _>>()?;
        Ok(v)
    };
    let mut settings = rows("SELECT k,v FROM meta WHERE k NOT IN ('notify_webhook_url') ORDER BY k", &["key", "value"])?;
    settings.retain(|s| !s["key"].as_str().unwrap_or_default().contains("secret"));
    let webhook_set = c.query_row("SELECT COUNT(*) FROM meta WHERE k='notify_webhook_url' AND v<>''", [], |r| r.get::<_, i64>(0))? > 0;
    Ok(json!({
        "moss_version": env!("CARGO_PKG_VERSION"),
        "exported_at": db::now(),
        "schema_version": c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        "note": "No secrets: API token hashes, the webhook URL and Supabase keys are not included.",
        "settings": settings,
        "notification_webhook_configured": webhook_set,
        "users": rows("SELECT email,role,linked,created_at FROM users ORDER BY email", &["email", "role", "linked", "created_at"])?,
        "servers": rows("SELECT id,name,runtime,game_version,loader_version,java_major,port,memory_mb,flags_profile,autostart,restart_enabled,restart_max,tags,notes,memory_limit_mb,cpu_limit_pct,pids_limit,created_at FROM servers ORDER BY created_at",
            &["id", "name", "runtime", "game_version", "loader_version", "java_major", "port", "memory_mb", "flags_profile", "autostart", "restart_enabled", "restart_max", "tags", "notes", "memory_limit_mb", "cpu_limit_pct", "pids_limit", "created_at"])?,
        "grants": rows("SELECT g.server_id,u.email,g.bundle FROM grants g JOIN users u ON u.id=g.user_id ORDER BY g.server_id,u.email", &["server_id", "email", "bundle"])?,
        "schedules": rows("SELECT server_id,kind,command,every_minutes,daily_at,tz,enabled,keep FROM schedules ORDER BY server_id,kind", &["server_id", "kind", "command", "every_minutes", "daily_at", "tz", "enabled", "keep"])?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mem() -> Connection { let c = Connection::open_in_memory().unwrap(); c.execute_batch("PRAGMA foreign_keys=ON").unwrap(); db::migrate_for_tests(&c); c }
    fn tmp() -> PathBuf { let d = std::env::temp_dir().join(format!("moss-rec-{}", uuid::Uuid::new_v4())); std::fs::create_dir_all(&d).unwrap(); d }
    fn user(c: &Connection, id: &str, email: &str, role: &str) { c.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,?,1,'t')", [id, email, role]).unwrap(); }

    #[test]
    fn owner_set_relinks_existing_and_creates_missing() {
        let c = mem();
        user(&c, "old", "a@x.io", "user");
        owner_set(&c, " A@X.io ").unwrap();
        let (role, linked): (String, i64) = c.query_row("SELECT role,linked FROM users WHERE email='a@x.io'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((role.as_str(), linked), ("owner", 0));
        let msg = owner_set(&c, "new@x.io").unwrap();
        assert!(msg.contains("Other owners still exist: a@x.io"));
        assert_eq!(c.query_row("SELECT COUNT(*) FROM users WHERE role='owner'", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert!(owner_set(&c, "not an email").is_err());
    }
    #[test]
    fn owner_clear_makes_setup_pending_again() {
        let c = mem();
        user(&c, "u", "a@x.io", "owner");
        assert!(!setup::pending_db(&c, false).unwrap());
        owner_clear(&c).unwrap();
        assert!(setup::pending_db(&c, false).unwrap());
        assert_eq!(c.query_row("SELECT role FROM users", [], |r| r.get::<_, String>(0)).unwrap(), "admin");
    }
    #[test]
    fn operations_reset_only_touches_unfinished_work() {
        let c = mem();
        for (id, st) in [("1", "running"), ("2", "queued"), ("3", "completed")] {
            c.execute("INSERT INTO operations(id,kind,title,state,created_at) VALUES (?,?,?,?,'t')", [id, "k", "t", st]).unwrap();
        }
        c.execute("INSERT INTO backups(id,server_id,created_at,status,kind) VALUES ('b','s','t','running','manual')", []).unwrap();
        assert!(operations_reset(&c).unwrap().starts_with("2 stuck operation(s) and 1 unfinished"));
        let states: Vec<String> = c.prepare("SELECT state FROM operations ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(states, ["failed", "failed", "completed"]);
    }
    #[cfg(unix)]
    #[test]
    fn setup_token_is_private_and_unclaim_demotes() {
        use std::os::unix::fs::PermissionsExt;
        let (c, d) = (mem(), tmp());
        user(&c, "u", "a@x.io", "owner");
        assert!(setup_token_new(&c, &d, false).unwrap().contains("an owner still exists"));
        assert!(!setup_token_new(&c, &d, true).unwrap().contains("still exists"));
        let p = d.join("setup-token");
        assert_eq!(std::fs::read_to_string(&p).unwrap().len(), setup::TOKEN_LEN);
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(setup::pending_db(&c, false).unwrap());
        let _ = std::fs::remove_dir_all(d);
    }
    #[test]
    fn servers_list_flags_missing_directories() {
        let (c, d) = (mem(), tmp());
        c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,installed,created_at) VALUES ('s1','Survival','paper','1.21',25565,2048,1,'t')", []).unwrap();
        assert!(servers_list(&c, &d).unwrap().contains("MISSING"));
        std::fs::create_dir_all(d.join("servers/s1")).unwrap();
        assert!(servers_list(&c, &d).unwrap().trim_end().ends_with("ok"));
        let _ = std::fs::remove_dir_all(d);
    }
    #[test]
    fn backup_verify_detects_corruption() {
        let (c, d) = (mem(), tmp());
        let sid = "s1";
        std::fs::create_dir_all(d.join("backups").join(sid)).unwrap();
        std::fs::write(d.join("servers_dummy"), b"x").unwrap();
        // a real archive from a small directory
        let src = d.join("src"); std::fs::create_dir_all(&src).unwrap(); std::fs::write(src.join("a.txt"), b"hello").unwrap();
        let out = d.join("backups").join(sid).join("b1.tar.zst");
        let (sha, ..) = backups::build_archive_for_tests(src, out.clone());
        c.execute("INSERT INTO backups(id,server_id,created_at,status,kind,sha256) VALUES ('b1',?,'t','ok','manual',?)", [sid, &sha]).unwrap();
        assert!(backup_verify(&c, &d, "b1").unwrap().contains("checksum matches"));
        assert!(backup_verify(&c, &d, "all").is_ok());
        { use std::io::Write; std::fs::OpenOptions::new().append(true).open(&out).unwrap().write_all(b"garbage").unwrap(); }
        let e = backup_verify(&c, &d, "b1").unwrap_err().to_string();
        assert!(e.contains("FAILED") && e.contains("checksum mismatch"), "{e}");
        assert!(backup_verify(&c, &d, "nope").is_err());
        let _ = std::fs::remove_dir_all(d);
    }
    #[test]
    fn db_check_reports_version_and_integrity() {
        let c = mem();
        let s = db_check(&c).unwrap();
        assert!(s.contains("integrity check: ok") && s.contains(&format!("schema version {}", db::latest_migration())));
        c.execute_batch("PRAGMA user_version = 9999").unwrap();
        assert!(db_check(&c).unwrap_err().to_string().contains("newer Moss"));
    }
    #[test]
    fn tokens_revoke_all_and_export_has_no_secrets() {
        let c = mem();
        user(&c, "u", "a@x.io", "owner");
        c.execute("INSERT INTO api_tokens(id,user_id,name,scope,hash,prefix,created_at) VALUES ('t','u','ci','full','SECRETHASH','moss_pat','t')", []).unwrap();
        c.execute("INSERT INTO meta(k,v) VALUES ('notify_webhook_url','https://hooks.example/SECRET'),('backup_keep','5')", []).unwrap();
        let j = export_config(&c).unwrap().to_string();
        assert!(!j.contains("SECRET") && j.contains("backup_keep") && j.contains("\"notification_webhook_configured\":true"));
        assert_eq!(tokens_revoke_all(&c).unwrap(), "1 API token(s) revoked.");
    }
}
