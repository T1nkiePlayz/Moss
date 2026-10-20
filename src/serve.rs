//! The `serve` command: builds the shared application state, runs startup recovery, spawns background
//! loops and serves the HTTP API until a shutdown signal.
use crate::{api, config, db, download, java, lifecycle, ops, platform, providers::curseforge, sched, state};
use std::{collections::HashMap, sync::{Arc, Mutex}, time::Instant};

pub async fn build_state(cfg: config::Config) -> anyhow::Result<state::St> {
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
        jwks: Mutex::new(None), guard: Default::default(), host: Mutex::new(platform::HostSampler::default()),
        cf: Mutex::new(Default::default()), cf_sem: tokio::sync::Semaphore::new(curseforge::CONCURRENCY),
    }))
}

pub async fn serve(cfg: config::Config) -> anyhow::Result<()> {
    cfg.validate()?;
    // held until the process exits; `moss recover` uses it to tell that a controller is running
    let lock = crate::lock::try_acquire(&cfg.data_dir)?.ok_or_else(|| anyhow::anyhow!("another Moss controller is already using {}", cfg.data_dir.display()))?;
    std::mem::forget(lock);
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
    crate::notify::spawn_background(&st);
    crate::history::spawn_background(&st);
    let addr = format!("{}:{}", cfg.bind, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}"))?;
    if cfg.dev() { tracing::warn!("AUTH DISABLED (dev mode) — anyone who can reach {addr} has full control"); }
    else { crate::auth::setup::ensure_token(&st).await?; }
    if !cfg.dev() && cfg.bind != "127.0.0.1" && cfg.bind != "::1" { tracing::warn!("listening on {addr}: put Moss behind a TLS reverse proxy before exposing it to a network"); }
    tracing::info!("Moss {} listening on http://{addr}  (data: {})", env!("CARGO_PKG_VERSION"), cfg.data_dir.display());
    let st_exit = st.clone();
    axum::serve(listener, api::router(st).into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(async move {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
        tracing::info!("shutting down the controller — game servers keep running");
        crate::history::flush(&st_exit, true).await;
    }).await?;
    Ok(())
}
