//! Update checks and in-place updates for non-Minecraft game servers: "installed vs latest" per install method, a
//! one-hour cache of the upstream lookups, and the update flow (pre-change backup, stop, reinstall over the files,
//! record the version, start again if it was running) as a tracked operation.
use super::{Game, Install, versions};
use crate::{error::{ApiError, ApiResult}, lifecycle, ops::{self, OpCtx, Operation}, server::{self, Srv}, state::St};
use anyhow::{Result, anyhow};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, future::Future, path::Path, sync::{LazyLock, Mutex}, time::{Duration, Instant}};

/// How long an upstream lookup is reused.
const TTL: Duration = Duration::from_secs(3600);

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Status {
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub available: bool,
    pub checked_at: String,
    pub source: String,
}

// ---------- parsing ----------

static ACF_BUILDID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""buildid"\s+"(\d+)""#).unwrap());

/// `"buildid"` of a Steam `appmanifest_<appid>.acf`.
pub fn parse_acf_buildid(text: &str) -> Option<String> { ACF_BUILDID.captures(text).map(|c| c[1].to_string()) }

/// `"version"` of Factorio's `data/base/info.json`.
pub fn parse_factorio_info(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    v["version"].as_str().filter(|v| !v.is_empty()).map(String::from)
}

/// True when `latest` is a newer version (or build number) than `installed`. Unparseable values never count as an
/// update: a server whose version is unknown must not be nagged about.
pub fn is_newer(installed: &str, latest: &str) -> bool {
    let (a, b) = (versions::version_key(installed), versions::version_key(latest));
    !a.is_empty() && !b.is_empty() && b > a
}

fn source(g: &Game) -> &'static str {
    match g.install { Install::Bedrock => "minecraft.net", Install::Terraria => "terraria.org", Install::Factorio => "factorio.com", Install::Steam(_) => "steamcmd.net" }
}

// ---------- lookups ----------

type Entry = (Instant, String, Value);
static CACHE: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

/// `(value, checked_at, fresh)`: the cached value while it is younger than an hour (unless `force`), else a new lookup.
async fn cached<F, Fut>(key: &str, force: bool, f: F) -> Result<(Value, String, bool)>
where F: FnOnce() -> Fut, Fut: Future<Output = Result<Value>> {
    if !force && let Some((t, at, v)) = CACHE.lock().unwrap().get(key) && t.elapsed() < TTL { return Ok((v.clone(), at.clone(), false)); }
    let v = f().await?;
    let at = crate::db::now();
    CACHE.lock().unwrap().insert(key.to_string(), (Instant::now(), at.clone(), v.clone()));
    Ok((v, at, true))
}

async fn installed_version(st: &St, s: &Srv, g: &Game) -> Option<String> {
    let dir = s.dir(st);
    match g.install {
        Install::Steam(app) => parse_acf_buildid(&tokio::fs::read_to_string(dir.join("steamapps").join(format!("appmanifest_{app}.acf"))).await.ok()?),
        Install::Factorio => match tokio::fs::read_to_string(dir.join("data/base/info.json")).await.ok().and_then(|t| parse_factorio_info(&t)) {
            Some(v) => Some(v),
            None => Some(s.game_version.clone()).filter(|v| !v.is_empty()),
        },
        Install::Bedrock | Install::Terraria => Some(s.game_version.clone()).filter(|v| !v.is_empty()),
    }
}

/// Compare a server's installed version with upstream's. Notifies admins when a fresh lookup finds an update.
pub async fn check(st: &St, s: &Srv, force: bool) -> Result<Status> {
    let g = s.game().ok_or_else(|| anyhow!("{} is not a game server", s.name))?;
    let installed = installed_version(st, s, g).await;
    let (latest, checked_at, fresh) = match g.install {
        Install::Steam(app) => {
            let (v, at, fresh) = cached(&format!("steam:{app}"), force, || async { Ok(Value::String(versions::steam_latest_buildid(st, app).await?)) }).await?;
            (v.as_str().map(String::from), at, fresh)
        }
        _ => {
            let (doc, at, fresh) = cached(&format!("doc:{}", g.id), force, || versions::release_doc(st, g)).await?;
            (versions::latest_in_doc(g, &doc, installed.as_deref().unwrap_or_default()), at, fresh)
        }
    };
    let available = matches!((&installed, &latest), (Some(i), Some(l)) if is_newer(i, l));
    if available && fresh {
        let body = format!("{} runs {}; {} is available.", s.name, installed.as_deref().unwrap_or("?"), latest.as_deref().unwrap_or("?"));
        crate::notify::send(st, "update_available", "info", &format!("{} update available for {}", g.name, s.name), &body, Some(&s.id)).await;
    }
    Ok(Status { installed, latest, available, checked_at, source: source(g).into() })
}

// ---------- the update flow ----------

/// Small operator-owned files an update must leave exactly as they are (worlds and saves are never written by the
/// installers; these are the files an installer rewrites or could rewrite).
pub const PROTECTED: &[&str] = &[
    "server.properties", "allowlist.json", "permissions.json", "serverconfig.txt", "banlist.txt", "moss-game.cfg",
    "config/server-settings.json", "config/server-whitelist.json", "config/server-adminlist.json", "config/server-banlist.json",
];
const PROTECTED_MAX: u64 = 1 << 20;

/// Runs `install` and afterwards puts every `PROTECTED` file back to the content it had before (also when the
/// installer failed). A defence on top of the installers' own keep rules.
pub async fn preserving<F, Fut>(dir: &Path, install: F) -> Result<()>
where F: FnOnce() -> Fut, Fut: Future<Output = Result<()>> {
    let mut saved: Vec<(&str, Vec<u8>)> = vec![];
    for n in PROTECTED {
        let p = dir.join(n);
        if tokio::fs::metadata(&p).await.is_ok_and(|m| m.is_file() && m.len() <= PROTECTED_MAX) && let Ok(b) = tokio::fs::read(&p).await { saved.push((n, b)); }
    }
    let res = install().await;
    for (n, bytes) in saved {
        let p = dir.join(n);
        if tokio::fs::read(&p).await.ok().as_deref() != Some(bytes.as_slice()) {
            if let Some(parent) = p.parent() { let _ = tokio::fs::create_dir_all(parent).await; }
            let _ = tokio::fs::write(&p, bytes).await;
        }
    }
    res
}

/// Start an update. `Ok(None)` when the server is already current. Refuses while installing or while another
/// operation holds the server.
pub async fn begin(st: &St, s: &Srv, actor: &str) -> ApiResult<Option<Operation>> {
    if s.game().is_none() { return Err(ApiError::bad("game_update.not_a_game", "only game servers can be updated this way")); }
    let (state, _) = server::state_of(s, st);
    if !s.installed || state == "installing" { return Err(ApiError::conflict("server.installing", "the server is still installing")); }
    let guard = st.lock_for(&s.id).try_lock_owned().map_err(|_| ApiError::conflict("server.locked", "another operation is changing this server"))?;
    let status = check(st, s, true).await.map_err(|e| ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "game_update.lookup_failed", format!("could not look up the latest version: {e:#}")))?;
    let Some(latest) = status.latest.clone().filter(|_| status.available) else { return Ok(None) };
    st.audit(actor, "server.game_update", &s.name, &format!("{} -> {latest}", status.installed.as_deref().unwrap_or("?")), Some(&s.id)).await;
    let (id, actor) = (s.id.clone(), actor.to_string());
    let op = ops::spawn_op(st, "server.game_update", format!("Update {}", s.name), Some(&s.id), true, move |ctx| async move {
        let _guard = guard;
        run(ctx, id, latest, actor).await
    });
    Ok(Some(op))
}

async fn run(ctx: OpCtx, id: String, latest: String, actor: String) -> Result<()> {
    let st = ctx.st.clone();
    let s = server::get(&st, &id).await.map_err(|e| anyhow!(e.message))?;
    let g = s.game().ok_or_else(|| anyhow!("not a game server"))?;
    ctx.stage("Taking a pre-change backup");
    match crate::backups::pre_change_forced_locked(&st, &s, "a game update", &ctx).await? {
        Some(b) => ctx.log(format!("Backup {b} holds the previous state; restore it to roll back.")),
        None => ctx.log("No pre-change backup was taken."),
    }
    ctx.check()?;
    let was_running = lifecycle::process_alive(&s);
    if was_running {
        ctx.stage("Stopping the server");
        lifecycle::halt(&st, &s).await;
    }
    let mut target = s.clone();
    // Steam games always follow the public branch; the others move to the concrete version found.
    if !matches!(g.install, Install::Steam(_)) { target.game_version = latest.clone(); }
    let dir = s.dir(&st);
    let res = preserving(&dir, || async {
        ctx.stage(&format!("Updating {} to {latest}", g.name));
        let r = crate::runtimes::resolve(&st, &s.runtime, &target.game_version, None).await?;
        crate::installer::install_files(&ctx, &st, &target, &r, &dir).await
    }).await;
    if let Err(e) = res {
        if was_running { ctx.log("The server was left stopped because the update did not finish; restore the pre-change backup or retry the update."); }
        return Err(e.context("update failed"));
    }
    if target.game_version != s.game_version {
        let (i, v) = (id.clone(), target.game_version.clone());
        st.db.call(move |c| c.execute("UPDATE servers SET game_version=? WHERE id=?", rusqlite::params![v, i])).await?;
    }
    st.timeline(&id, "update", &format!("{} {latest}", g.name)).await;
    server::emit(&st, &id).await;
    if was_running {
        ctx.stage("Starting the server");
        let fresh = server::get(&st, &id).await.map_err(|e| anyhow!(e.message))?;
        if let Err(e) = lifecycle::start(&st, &fresh, &actor).await { ctx.log(format!("The server did not start again: {}", e.message)); }
    }
    ctx.progress(Some(1.0));
    Ok(())
}

/// The scheduler's entry: check, and update only when something newer exists. Returns the status line.
pub async fn scheduled(st: &St, s: &Srv, actor: &str) -> std::result::Result<String, String> {
    match begin(st, s, actor).await {
        Ok(Some(_)) => Ok("update started".into()),
        Ok(None) => Ok("up to date".into()),
        Err(e) => Err(e.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACF: &str = "\"AppState\"\n{\n\t\"appid\"\t\t\"896660\"\n\t\"Universe\"\t\t\"1\"\n\t\"name\"\t\t\"Valheim Dedicated Server\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"buildid\"\t\t\"25909431\"\n\t\"TargetBuildID\"\t\t\"25909431\"\n}\n";

    #[test]
    fn acf_buildid() {
        assert_eq!(parse_acf_buildid(ACF).as_deref(), Some("25909431"));
        assert_eq!(parse_acf_buildid("\"AppState\" { \"TargetBuildID\" \"7\" }"), None);
        assert_eq!(parse_acf_buildid(""), None);
    }

    #[test]
    fn factorio_info() {
        assert_eq!(parse_factorio_info(r#"{"name":"base","version":"2.0.55","title":"Base Mod"}"#).as_deref(), Some("2.0.55"));
        assert_eq!(parse_factorio_info(r#"{"version":""}"#), None);
        assert_eq!(parse_factorio_info("not json"), None);
    }

    #[test]
    fn comparison() {
        assert!(is_newer("1.4.4.9", "1.4.5.0"));
        assert!(is_newer("2.0.9", "2.0.10"));
        assert!(is_newer("25909431", "25909432"));
        assert!(!is_newer("1.4.5.0", "1.4.5.0"));
        assert!(!is_newer("1.4.5.1", "1.4.5.0"));
        assert!(!is_newer("latest", "1.4.5.0"));
        assert!(!is_newer("1.4.5.0", "latest"));
    }

    #[tokio::test]
    async fn lookups_are_cached_until_forced() {
        let calls = std::sync::atomic::AtomicU32::new(0);
        let f = || async { calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst); Ok(Value::from("1")) };
        let key = format!("test:{}", uuid::Uuid::new_v4());
        assert!(cached(&key, false, f).await.unwrap().2);
        assert!(!cached(&key, false, f).await.unwrap().2);
        assert!(cached(&key, true, f).await.unwrap().2);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn update_keeps_worlds_and_operator_files() {
        let d = std::env::temp_dir().join(format!("moss-update-{}", uuid::Uuid::new_v4()));
        for sub in ["worlds/w", "config", "saves", "bin"] { std::fs::create_dir_all(d.join(sub)).unwrap(); }
        let files = [("worlds/w/level.dat", "world"), ("saves/moss.zip", "save"), ("server.properties", "motd=mine"), ("allowlist.json", "[1]"), ("permissions.json", "[2]"),
            ("serverconfig.txt", "port=7778"), ("config/server-settings.json", "{\"name\":\"mine\"}"), ("moss-game.cfg", "password=secret")];
        for (n, body) in files { std::fs::write(d.join(n), body).unwrap(); }
        // A fake installer that behaves badly: overwrites and deletes the operator's files, replaces the binary.
        let res = preserving(&d, || async {
            std::fs::write(d.join("server.properties"), "motd=default")?;
            std::fs::write(d.join("serverconfig.txt"), "default")?;
            std::fs::remove_file(d.join("config/server-settings.json"))?;
            std::fs::remove_file(d.join("moss-game.cfg"))?;
            std::fs::write(d.join("bin/server"), "v2")?;
            Ok(())
        }).await;
        assert!(res.is_ok());
        for (n, body) in files { assert_eq!(std::fs::read_to_string(d.join(n)).unwrap(), body, "{n}"); }
        assert_eq!(std::fs::read_to_string(d.join("bin/server")).unwrap(), "v2");
        // Restored even when the installer fails.
        let failed = preserving(&d, || async { std::fs::write(d.join("server.properties"), "x")?; Err(anyhow!("boom")) }).await;
        assert!(failed.is_err());
        assert_eq!(std::fs::read_to_string(d.join("server.properties")).unwrap(), "motd=mine");
        let _ = std::fs::remove_dir_all(&d);
    }
}
