//! Real-game end to end, run by the nightly matrix (.github/workflows/games-e2e.yml) and never by normal CI.
//! Every test is `#[ignore]`. `MOSS_E2E_GAME=<id>` picks the game (`all` loops over every SteamCMD game); the steps are
//! install, start, players, backup, stop and no_orphan. One JSON line per step goes to `$MOSS_E2E_RESULTS`:
//! `{"game","step","ok","seconds","error"}`. A failed step ends that game's run (later steps depend on it), and the
//! test fails at the end if any game failed. Time limits in seconds (defaults in brackets): `MOSS_E2E_INSTALL_S` [3600],
//! `MOSS_E2E_START_S` [1200], `MOSS_E2E_PLAYERS_S` [120], `MOSS_E2E_BACKUP_S` [1800], `MOSS_E2E_STOP_S` [300].
//! Run one: `MOSS_E2E_GAME=valheim cargo nextest run --run-ignored only --test games_e2e --no-capture`.
mod suite;
use serde_json::{Value, json};
use std::{io::Write, time::{Duration, Instant}};

/// Games that are not SteamCMD games; `all` skips them (name one in `MOSS_E2E_GAME` to run it anyway).
const NOT_STEAM: &[&str] = &["bedrock", "terraria", "factorio"];

fn secs(var: &str, default: u64) -> u64 { std::env::var(var).ok().and_then(|v| v.parse().ok()).unwrap_or(default) }

/// Appends one result line to `$MOSS_E2E_RESULTS` (and echoes it to stderr).
fn record(game: &str, step: &str, started: Instant, res: Result<(), &str>) {
    let line = json!({"game": game, "step": step, "ok": res.is_ok(), "seconds": started.elapsed().as_secs(), "error": res.err()});
    eprintln!("games_e2e: {line}");
    if let Some(path) = std::env::var_os("MOSS_E2E_RESULTS") {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).expect("open MOSS_E2E_RESULTS");
        writeln!(f, "{line}").expect("write result");
    }
}

/// Polls until `f` yields a value or the limit passes (never panics, unlike `suite::until`, so a step can fail cleanly).
async fn wait<T, Fut: std::future::Future<Output = Result<Option<T>, String>>>(what: &str, limit: u64, mut f: impl FnMut() -> Fut) -> Result<T, String> {
    let end = Instant::now() + Duration::from_secs(limit);
    loop {
        if let Some(v) = f().await? { return Ok(v); }
        if Instant::now() >= end { return Err(format!("timed out after {limit}s waiting for {what}")); }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn get(c: &reqwest::Client, url: String) -> Option<Value> { c.get(url).send().await.ok()?.json().await.ok() }

async fn post(c: &reqwest::Client, url: String, body: Value) -> Result<Value, String> {
    let r = c.post(&url).json(&body).send().await.map_err(|e| format!("POST {url}: {e}"))?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if status.is_success() { Ok(v) } else { Err(format!("POST {url}: {status} {v}")) }
}

/// Waits for an operation to end; the error carries the operation's own message unless it completed.
async fn finish_op(c: &reqwest::Client, base: &str, id: &str, limit: u64) -> Result<(), String> {
    let o = wait("the operation to finish", limit, || async { Ok(get(c, format!("{base}/operations/{id}")).await.filter(|o| matches!(o["state"].as_str(), Some("completed" | "failed" | "cancelled" | "interrupted")))) }).await?;
    if o["state"] == "completed" { Ok(()) } else { Err(format!("operation {}: {}", o["state"], o["error"].as_str().unwrap_or(""))) }
}

async fn wait_server(c: &reqwest::Client, base: &str, id: &str, what: &str, limit: u64, want: &str) -> Result<Value, String> {
    wait(what, limit, || async {
        match get(c, format!("{base}/servers/{id}")).await {
            Some(s) if s["state"] == want => Ok(Some(s)),
            Some(s) if want == "running" && s["state"] == "crashed" => Err(format!("the server crashed: {}", s["state_detail"].as_str().unwrap_or(""))),
            _ => Ok(None),
        }
    }).await
}

/// Whether the wrapper process or anything in its process group is still alive.
fn alive(pid: i64) -> bool { unsafe { libc::kill(-(pid as i32), 0) == 0 || libc::kill(pid as i32, 0) == 0 } }

/// Runs every step for one game, recording each. Returns whether all passed.
async fn run_game(game: &str) -> bool {
    let data = std::env::var_os("MOSS_E2E_DIR").map_or_else(std::env::temp_dir, Into::into).join(format!("moss-games-e2e-{game}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let port = suite::free_port();
    let moss = suite::Moss::start(&data, port, &[]);
    let c = reqwest::Client::new();
    let base = moss.base.clone();

    // Record the step; on failure the run for this game ends here (the Moss handle drops, which reaps the game).
    macro_rules! step { ($name:expr, $body:expr) => {{
        let t = Instant::now();
        let res: Result<_, String> = $body.await;
        record(game, $name, t, res.as_ref().map(|_| ()).map_err(String::as_str));
        match res { Ok(v) => v, Err(_) => return false }
    }}; }

    step!("controller", wait("the controller to answer", 30, || async { Ok(c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()).map(|_| ())) }));
    let cat = get(&c, format!("{base}/runtimes")).await.unwrap_or(Value::Null);
    let game_port = cat.as_array().and_then(|a| a.iter().find(|g| g["id"] == game)).and_then(|g| g["default_port"].as_u64()).map_or_else(suite::free_port, |p| p as u16);

    let id = step!("install", async {
        let created = post(&c, format!("{base}/servers"), json!({"name": format!("e2e-{game}"), "runtime": game, "game_version": "latest", "port": game_port})).await?;
        let id = created["server"]["id"].as_str().ok_or("no server id in the create response")?.to_string();
        finish_op(&c, &base, created["operation"]["id"].as_str().ok_or("no operation id")?, secs("MOSS_E2E_INSTALL_S", 3600)).await?;
        Ok::<_, String>(id)
    });
    let pid = step!("start", async {
        post(&c, format!("{base}/servers/{id}/actions/start"), json!({})).await?;
        // "running" is Moss's own readiness: the game's ready log rule, or its fallback timer, has fired
        let s = wait_server(&c, &base, &id, "the server to be running", secs("MOSS_E2E_START_S", 1200), "running").await?;
        s["pid"].as_i64().ok_or_else(|| "a running server reports no pid".to_string())
    });
    step!("players", async {
        let url = format!("{base}/servers/{id}/players/online");
        let last = std::cell::RefCell::new(String::from("no answer"));
        let r = wait("a player query that works", secs("MOSS_E2E_PLAYERS_S", 120), || async {
            match c.get(&url).send().await {
                Ok(r) if r.status().is_success() => Ok(r.json::<Value>().await.ok()),
                Ok(r) => { *last.borrow_mut() = format!("{} {}", r.status(), r.text().await.unwrap_or_default()); Ok(None) }
                Err(e) => { *last.borrow_mut() = e.to_string(); Ok(None) }
            }
        }).await.map_err(|e| format!("{e} (last answer: {})", last.borrow()))?;
        let empty = r["players"].as_array().is_some_and(Vec::is_empty) && r["online"].as_u64().unwrap_or(0) == 0;
        if empty { Ok(()) } else { Err(format!("expected no players online, got {r}")) }
    });
    step!("backup", async {
        let limit = secs("MOSS_E2E_BACKUP_S", 1800);
        let bk = post(&c, format!("{base}/servers/{id}/backups"), json!({"note": "games-e2e"})).await?;
        finish_op(&c, &base, bk["operation"]["id"].as_str().ok_or("no operation id")?, limit).await?;
        let list = get(&c, format!("{base}/servers/{id}/backups")).await.ok_or("could not list backups")?;
        let b = list.as_array().and_then(|l| l.first()).ok_or("the backup is not listed")?;
        if b["status"] != "ok" { return Err(format!("backup status is {}", b["status"])); }
        let vf = post(&c, format!("{base}/backups/{}/verify", b["id"].as_str().unwrap_or_default()), json!({})).await?;
        finish_op(&c, &base, vf["operation"]["id"].as_str().ok_or("no operation id")?, limit).await
    });
    step!("stop", async {
        post(&c, format!("{base}/servers/{id}/actions/stop"), json!({})).await?;
        wait_server(&c, &base, &id, "the server to stop", secs("MOSS_E2E_STOP_S", 300), "stopped").await.map(|_| ())
    });
    step!("no_orphan", wait("the game process to be gone", 30, || async { Ok((!alive(pid)).then_some(())) }));
    drop(moss);
    true
}

/// The games to run: `MOSS_E2E_GAME` names one, or `all` for every SteamCMD game in the catalogue.
async fn games() -> Vec<String> {
    let want = std::env::var("MOSS_E2E_GAME").unwrap_or_default();
    match want.as_str() {
        "" => panic!("set MOSS_E2E_GAME=<game id> (or all)"),
        "all" => {
            let (data, port) = (std::env::temp_dir().join(format!("moss-games-e2e-list-{}", std::process::id())), suite::free_port());
            let moss = suite::Moss::start(&data, port, &[]);
            let c = reqwest::Client::new();
            let mut cat = None;
            for _ in 0..60 {
                if let Some(v) = get(&c, format!("{}/runtimes", moss.base)).await { cat = Some(v); break; }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let cat = cat.expect("the controller did not answer");
            cat.as_array().unwrap().iter().filter(|g| g["category"] == "game" && g["installable"] == true).filter_map(|g| g["id"].as_str())
                .filter(|id| !NOT_STEAM.contains(id)).map(String::from).collect()
        }
        one => vec![one.to_string()],
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "installs and runs a real game server; set MOSS_E2E_GAME (see the file header)"]
async fn game_end_to_end() {
    let mut failed = vec![];
    for g in games().await {
        eprintln!("games_e2e: === {g} ===");
        if !run_game(&g).await { failed.push(g); }
    }
    assert!(failed.is_empty(), "games that failed: {}", failed.join(", "));
}
