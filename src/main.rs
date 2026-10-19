mod api; mod auth; mod backups; mod config; mod console; mod content; mod db; mod download; mod error; mod files; mod health;
mod installer; mod java; mod lifecycle; mod mcping; mod metrics; mod ops; mod runtimes; mod sched; mod server; mod state; mod undo;

use clap::{Parser, Subcommand};
use std::{collections::HashMap, sync::{Arc, Mutex}, time::Instant};

#[derive(Parser)]
#[command(name = "moss", version, about = "Moss — a fast, local-first game server controller")]
struct Cli { #[command(subcommand)] cmd: Cmd }

#[derive(Subcommand)]
enum Cmd {
    /// Run the controller (web panel + API)
    Serve(config::Config),
    /// Check the environment: data directory, ports, Java, auth configuration
    Doctor(config::Config),
    /// A tiny fake Minecraft server, used by the `sim` runtime for demos and tests
    SimServer { #[arg(long, default_value_t = 25565)] port: u16 },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "moss=info,tower_http=warn".into())).with_target(false).init();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    match cli.cmd {
        Cmd::Serve(cfg) => rt.block_on(serve(cfg)),
        Cmd::Doctor(cfg) => rt.block_on(doctor(cfg)),
        Cmd::SimServer { port } => rt.block_on(sim(port)),
    }
}

async fn build_state(cfg: config::Config) -> anyhow::Result<state::St> {
    for d in [cfg.data_dir.clone(), cfg.servers_dir(), cfg.backups_dir(), cfg.cache_dir(), cfg.tmp_dir(), cfg.java_dir()] { std::fs::create_dir_all(&d)?; }
    let _ = std::fs::remove_dir_all(cfg.tmp_dir()); // controller-owned scratch space: safe to clear on start
    std::fs::create_dir_all(cfg.tmp_dir())?;
    let db = db::Db::open(&cfg.data_dir.join("moss.sqlite"))?;
    let contact = db.call(|c| c.query_row("SELECT v FROM meta WHERE k='modrinth_contact'", [], |r| r.get::<_, String>(0))).await.unwrap_or_default();
    let (bus, _) = tokio::sync::broadcast::channel(512);
    Ok(Arc::new(state::Inner {
        http: download::client(&contact), cfg, db, bus, started: Instant::now(),
        live: Mutex::new(HashMap::new()), hubs: Mutex::new(HashMap::new()), stats: Mutex::new(HashMap::new()), tickets: Mutex::new(HashMap::new()),
        ops: Mutex::new(ops::Registry::default()), locks: Mutex::new(HashMap::new()), cache: Mutex::new(HashMap::new()), users: Mutex::new(HashMap::new()),
        jwks: Mutex::new(None), host: Mutex::new(metrics::HostSampler::default()),
    }))
}

async fn serve(cfg: config::Config) -> anyhow::Result<()> {
    cfg.validate()?;
    let st = build_state(cfg.clone()).await?;
    ops::recover(&st).await?;
    {
        let st = st.clone();
        tokio::spawn(async move { loop { let _ = st.db.call(crate::db::prune_history).await; tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await; } });
    }
    st.db.call(|c| c.execute("UPDATE backups SET status='failed' WHERE status IN ('running','verifying')", [])).await?;
    st.db.call(|c| c.execute("UPDATE servers SET installed=0, install_error='installation was interrupted by a controller restart — delete and recreate this server' WHERE installed=0 AND install_error IS NULL AND created_at < ?", [db::now()])).await.ok();
    { let s2 = st.clone(); tokio::task::spawn_blocking(move || { java::installs(&s2, true); }); }
    lifecycle::reconcile(&st).await?;
    lifecycle::spawn_background(&st);
    sched::spawn_background(&st);
    let addr = format!("{}:{}", cfg.bind, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}"))?;
    if cfg.dev() { tracing::warn!("AUTH DISABLED (dev mode) — anyone who can reach {addr} has full control"); }
    else if cfg.owner_email_list().is_empty() { tracing::warn!("MOSS_OWNER_EMAILS is not set: the first verified account to sign in becomes the owner. Set it to pin ownership."); }
    if !cfg.dev() && cfg.bind != "127.0.0.1" && cfg.bind != "::1" { tracing::warn!("listening on {addr}: put Moss behind a TLS reverse proxy before exposing it to a network"); }
    tracing::info!("Moss {} listening on http://{addr}  (data: {})", env!("CARGO_PKG_VERSION"), cfg.data_dir.display());
    axum::serve(listener, api::router(st)).with_graceful_shutdown(async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
        tracing::info!("shutting down the controller — game servers keep running");
    }).await?;
    Ok(())
}

async fn doctor(cfg: config::Config) -> anyhow::Result<()> {
    let ok = |b: bool| if b { "ok  " } else { "FAIL" };
    println!("Moss {} doctor\n", env!("CARGO_PKG_VERSION"));
    println!("[{}] config: {}", ok(cfg.validate().is_ok()), cfg.validate().err().map_or("valid".into(), |e| e.to_string()));
    let writable = std::fs::create_dir_all(&cfg.data_dir).is_ok() && std::fs::write(cfg.data_dir.join(".doctor"), b"x").is_ok();
    let _ = std::fs::remove_file(cfg.data_dir.join(".doctor"));
    println!("[{}] data directory {} writable", ok(writable), cfg.data_dir.display());
    let (t, f) = metrics::disk(&cfg.data_dir);
    println!("[{}] disk: {:.1} GiB free of {:.1} GiB", ok(f > (2 << 30)), f as f64 / (1u64 << 30) as f64, t as f64 / (1u64 << 30) as f64);
    println!("[{}] port {} available", ok(tokio::net::TcpListener::bind((cfg.bind.as_str(), cfg.port)).await.is_ok()), cfg.port);
    let st = build_state(cfg.clone()).await?;
    let j = tokio::task::spawn_blocking(move || java::installs(&st, true)).await?;
    if j.is_empty() { println!("[warn] no Java found — Moss can download Temurin from Settings → Java"); }
    for i in j { println!("[ok  ] java {} ({}) at {}{}", i.major, i.version, i.path.display(), if i.managed { " [managed]" } else { "" }); }
    println!("[{}] auth: {}", ok(cfg.dev() || !cfg.supabase_url.is_empty()), if cfg.dev() { "dev mode (no login, loopback only)" } else { "supabase" });
    Ok(())
}

/// Fake Minecraft server: prints realistic logs, answers status pings, handles a few console commands.
async fn sim(port: u16) -> anyhow::Result<()> {
    use tokio::io::AsyncBufReadExt;
    let t = chrono::Local::now;
    let log = |lvl: &str, m: &str| println!("[{}] [Server thread/{lvl}]: {m}", t().format("%H:%M:%S"));
    log("INFO", "Starting minecraft server version sim-1.0");
    log("INFO", &format!("Starting Minecraft server on *:{port}"));
    tokio::spawn(mcping::serve_sim(port));
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    log("INFO", "Preparing level \"world\"");
    log("WARN", "Can't keep up! Is the server overloaded? (simulated)");
    log("INFO", "Done (1.204s)! For help, type \"help\"");
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(l)) = lines.next_line().await {
        match l.trim() {
            "stop" => { log("INFO", "Stopping the server"); log("INFO", "Saving worlds"); tokio::time::sleep(std::time::Duration::from_millis(400)).await; return Ok(()); }
            "crash" => { log("ERROR", "Simulated crash: java.lang.OutOfMemoryError: Java heap space"); std::process::exit(1); }
            "save-off" => log("INFO", "Automatic saving is now disabled"),
            "save-on" => log("INFO", "Automatic saving is now enabled"),
            "save-all flush" => log("INFO", "Saved the game"),
            "list" => log("INFO", "There are 2 of a max of 20 players online: Steve, Alex"),
            c if c.starts_with("say ") => log("INFO", &format!("[Server] {}", &c[4..])),
            "" => {}
            c => log("INFO", &format!("Unknown or incomplete command: {c}")),
        }
    }
    Ok(())
}
