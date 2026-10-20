//! The `doctor` command: checks data directory, disk, port, Java, auth and CurseForge configuration.
use crate::{config, providers::curseforge, java, platform, serve::build_state};

pub async fn doctor(cfg: config::Config) -> anyhow::Result<()> {
    let ok = |b: bool| if b { "ok  " } else { "FAIL" };
    println!("Moss {} doctor ({} {})\n", env!("CARGO_PKG_VERSION"), platform::OS_LABEL, std::env::consts::ARCH);
    println!("[{}] config: {}", ok(cfg.validate().is_ok()), cfg.validate().err().map_or("valid".into(), |e| e.to_string()));
    let writable = std::fs::create_dir_all(&cfg.data_dir).is_ok() && std::fs::write(cfg.data_dir.join(".doctor"), b"x").is_ok();
    let _ = std::fs::remove_file(cfg.data_dir.join(".doctor"));
    println!("[{}] data directory {} writable", ok(writable), cfg.data_dir.display());
    let (t, f) = platform::disk(&cfg.data_dir);
    println!("[{}] disk: {:.1} GiB free of {:.1} GiB", ok(f > (2 << 30)), f as f64 / (1u64 << 30) as f64, t as f64 / (1u64 << 30) as f64);
    println!("[{}] port {} available", ok(tokio::net::TcpListener::bind((cfg.bind.as_str(), cfg.port)).await.is_ok()), cfg.port);
    match platform::cgroup::backend() {
        Ok(_) => println!("[ok  ] resource limits: cgroup v2 (memory, CPU and process limits are enforced)"),
        Err(e) => println!("[info] resource limits are not enforced: {e}"),
    }
    let st = build_state(cfg.clone()).await?;
    let st2 = st.clone();
    let j = tokio::task::spawn_blocking(move || java::installs(&st2, true)).await?;
    if j.is_empty() { println!("[warn] no Java found — Moss can download Temurin from Settings → Java"); }
    for i in j { println!("[ok  ] java {} ({}) at {}{}", i.major, i.version, i.path.display(), if i.managed { " [managed]" } else { "" }); }
    println!("[{}] auth: {}", ok(cfg.dev() || !cfg.supabase_url.is_empty()), if cfg.dev() { "dev mode (no login, loopback only)" } else { "supabase" });
    let cf = match curseforge::mode(&st) {
        Some(curseforge::Mode::Key(_)) => "direct, with CURSEFORGE_API_KEY".to_string(),
        Some(curseforge::Mode::Proxy(u)) => format!("through the edge function at {u}"),
        None => "off (deploy the Supabase edge function or set CURSEFORGE_API_KEY)".to_string(),
    };
    println!("[info] curseforge: {cf}");
    Ok(())
}
