//! The `doctor` command: checks data directory, disk, port, Java, auth and CurseForge configuration.
//! The wording of each check is documented in `docs/troubleshooting.md`; keep the two in step when a message changes.
use std::path::Path;
use crate::{config, providers::curseforge, java, platform, serve::build_state};

pub async fn doctor(cfg: config::Config) -> anyhow::Result<()> {
    let ok = |b: bool| if b { "ok  " } else { "FAIL" };
    println!("Moss {} doctor ({} {})\n", env!("CARGO_PKG_VERSION"), platform::OS_LABEL, std::env::consts::ARCH);
    println!("[{}] config: {}", ok(cfg.validate().is_ok()), cfg.validate().err().map_or("valid".into(), |e| e.to_string()));
    // create the data directory only when one was named (--data-dir or MOSS_DATA_DIR): checking the setup must not
    // leave an empty Moss instance at the default location
    let named = std::env::var_os("MOSS_DATA_DIR").is_some_and(|v| !v.is_empty())
        || std::env::args_os().any(|a| a == "--data-dir" || a.to_string_lossy().starts_with("--data-dir="));
    let exists = named || cfg.data_dir.join("moss.sqlite").is_file();
    let (probe, label) = if exists { (cfg.data_dir.clone(), String::new()) } else {
        let p = cfg.data_dir.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf).unwrap_or_else(|| ".".into());
        (p, " (not created yet; Moss creates it on first start)".to_string())
    };
    let marker = probe.join(format!(".moss-doctor-{}", std::process::id()));
    let writable = std::fs::write(&marker, b"x").is_ok();
    let _ = std::fs::remove_file(&marker);
    println!("[{}] data directory {}{label} writable", ok(writable), cfg.data_dir.display());
    let (t, f) = platform::disk(&probe);
    println!("[{}] disk: {:.1} GiB free of {:.1} GiB", ok(f > (2 << 30)), f as f64 / (1u64 << 30) as f64, t as f64 / (1u64 << 30) as f64);
    println!("[{}] port {} available", ok(tokio::net::TcpListener::bind((cfg.bind.as_str(), cfg.port)).await.is_ok()), cfg.port);
    match platform::cgroup::backend() {
        Ok(_) => println!("[ok  ] resource limits: cgroup v2 (memory, CPU and process limits are enforced)"),
        Err(e) => println!("[info] resource limits are not enforced: {e}"),
    }
    // an existing data directory is used as is; otherwise a throwaway one, removed again below
    let scratch = if exists { None } else {
        let p = std::env::temp_dir().join(format!("moss-doctor-{}", uuid::Uuid::new_v4()));
        { use std::os::unix::fs::DirBuilderExt; std::fs::DirBuilder::new().mode(0o700).create(&p)?; } // fresh, never someone else's
        Some(p)
    };
    let st = build_state(config::Config { data_dir: scratch.clone().unwrap_or_else(|| cfg.data_dir.clone()), ..cfg.clone() }).await?;
    match crate::container::engine(&st).await {
        Ok(e) => { let i = e.info(); println!("[ok  ] container engine: {} {}{} (servers can use container isolation)", i.kind.name(), i.version, if i.rootless { ", rootless" } else { ", rootful" }); }
        Err(m) => println!("[info] container engine: {m}"),
    }
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
    if let Some(s) = scratch { drop(st); let _ = std::fs::remove_dir_all(s); }
    Ok(())
}
