//! Owner claim with a setup token. When `MOSS_OWNER_EMAILS` is empty and no owner exists, Moss writes a random
//! token to `<data>/setup-token` (0600); the operator proves control of the host by posting it to `/setup/claim`.
use super::secret;
use crate::state::St;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::PathBuf;

pub const TOKEN_LEN: usize = 32;

pub fn token_path(st: &St) -> PathBuf { st.cfg.data_dir.join("setup-token") }

/// Setup is pending when nobody can become owner by email and no owner row exists yet.
pub fn pending_db(c: &Connection, owner_emails_configured: bool) -> rusqlite::Result<bool> {
    if owner_emails_configured { return Ok(false); }
    Ok(c.query_row("SELECT COUNT(*) FROM users WHERE role='owner'", [], |r| r.get::<_, i64>(0))? == 0)
}

pub async fn pending(st: &St) -> bool {
    if st.cfg.dev() { return false; }
    let configured = !st.cfg.owner_email_list().is_empty();
    st.db.call(move |c| pending_db(c, configured)).await.unwrap_or(false)
}

/// Write the token file (replacing any old one) readable by the owner only.
pub fn write_token_file(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(path);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; o.mode(0o600); }
    o.open(path)?.write_all(token.as_bytes())
}

/// Startup hook: create the setup token when setup is pending (an existing valid file is kept across restarts).
pub async fn ensure_token(st: &St) -> anyhow::Result<()> {
    let path = token_path(st);
    if !pending(st).await {
        if path.exists() { let _ = std::fs::remove_file(&path); }
        return Ok(());
    }
    let valid = std::fs::read_to_string(&path).map(|t| t.trim().len() == TOKEN_LEN).unwrap_or(false);
    if !valid { write_token_file(&path, &secret::random_string(TOKEN_LEN))?; }
    tracing::warn!("NO OWNER YET: to claim this instance, sign in and enter the setup token stored in {} (or set MOSS_OWNER_EMAILS). Anyone who can read that file can become the owner.", path.display());
    Ok(())
}

/// Does `given` match the token on disk? `None` when there is no usable token file.
pub fn check_token(st: &St, given: &str) -> Option<bool> {
    compare(&std::fs::read_to_string(token_path(st)).ok()?, given)
}

/// Constant-time comparison of the stored token with the submitted one; `None` if the stored value is malformed.
pub fn compare(stored: &str, given: &str) -> Option<bool> {
    let stored = stored.trim();
    if stored.len() != TOKEN_LEN { return None; }
    Some(secret::ct_eq(stored.as_bytes(), given.trim().as_bytes()))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Claim { Claimed, AlreadyClaimed }

/// Make `id` the owner in one transaction, only if no owner exists. The database connection serialises writers
/// and the check plus write share one transaction, so exactly one concurrent claimant can win.
pub fn claim_db(c: &Connection, id: &str, email: &str) -> rusqlite::Result<Claim> {
    let tx = c.unchecked_transaction()?;
    if tx.query_row("SELECT COUNT(*) FROM users WHERE role='owner'", [], |r| r.get::<_, i64>(0))? > 0 { return Ok(Claim::AlreadyClaimed); }
    let exists = tx.query_row("SELECT 1 FROM users WHERE id=?", [id], |_| Ok(())).optional()?.is_some();
    if exists { tx.execute("UPDATE users SET role='owner' WHERE id=?", [id])?; }
    else { tx.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,'owner',1,?)", params![id, email, crate::db::now()])?; }
    tx.commit()?;
    Ok(Claim::Claimed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> Connection { let c = Connection::open_in_memory().unwrap(); crate::db::migrate_for_tests(&c); c }

    #[test]
    fn state_machine() {
        let c = db();
        assert!(pending_db(&c, false).unwrap());
        assert!(!pending_db(&c, true).unwrap(), "configured owner emails disable setup");
        assert_eq!(claim_db(&c, "u1", "a@x.io").unwrap(), Claim::Claimed);
        assert!(!pending_db(&c, false).unwrap());
        assert_eq!(claim_db(&c, "u2", "b@x.io").unwrap(), Claim::AlreadyClaimed);
        let role: String = c.query_row("SELECT role FROM users WHERE id='u1'", [], |r| r.get(0)).unwrap();
        assert_eq!(role, "owner");
        assert_eq!(c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }
    #[test]
    fn token_comparison() {
        let t = secret::random_string(TOKEN_LEN);
        assert_eq!(compare(&format!("{t}\n"), &t), Some(true));
        assert_eq!(compare(&t, &format!(" {t} ")), Some(true));
        assert_eq!(compare(&t, &secret::random_string(TOKEN_LEN)), Some(false));
        assert_eq!(compare(&t, "short"), Some(false));
        assert_eq!(compare("short", "short"), None);
        assert_eq!(compare("", ""), None);
    }
    #[test]
    fn invited_user_is_promoted_not_duplicated() {
        let c = db();
        c.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES ('u1','a@x.io','user',1,'t')", []).unwrap();
        assert_eq!(claim_db(&c, "u1", "a@x.io").unwrap(), Claim::Claimed);
        assert_eq!(c.query_row("SELECT role FROM users WHERE id='u1'", [], |r| r.get::<_, String>(0)).unwrap(), "owner");
    }
    #[tokio::test]
    async fn only_one_concurrent_claimant_wins() {
        let dir = std::env::temp_dir().join(format!("moss-setup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Db::open(&dir.join("t.sqlite")).unwrap();
        let tasks: Vec<_> = (0..8).map(|i| { let db = db.clone(); tokio::spawn(async move { db.call(move |c| claim_db(c, &format!("u{i}"), &format!("u{i}@x.io"))).await.unwrap() }) }).collect();
        let mut wins = 0;
        for t in tasks { if t.await.unwrap() == Claim::Claimed { wins += 1; } }
        assert_eq!(wins, 1);
        let owners = db.call(|c| c.query_row("SELECT COUNT(*) FROM users WHERE role='owner'", [], |r| r.get::<_, i64>(0))).await.unwrap();
        assert_eq!(owners, 1);
        let _ = std::fs::remove_dir_all(dir);
    }
    #[cfg(unix)]
    #[test]
    fn token_file_is_private_and_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir().join(format!("moss-tok-{}", uuid::Uuid::new_v4()));
        write_token_file(&p, "old").unwrap();
        write_token_file(&p, "new-token").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new-token");
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_file(p).unwrap();
    }
}
