//! `GET /support-bundle`: a zip of diagnostics for bug reports, assembled in memory (all inputs are small and
//! capped), written to a temp file and streamed back. Everything passes through `redact::redact_text`.
use super::*;
use crate::platform;
use std::io::Write;

mod redact;
pub use redact::redact_text;

const MAX_ENTRY: usize = 4 << 20;
const MAX_TOTAL: usize = 32 << 20;
const CONSOLE_LINES: usize = 300;

fn cap(mut v: Vec<u8>) -> Vec<u8> {
    if v.len() > MAX_ENTRY { v.truncate(MAX_ENTRY); v.extend_from_slice(b"\n[truncated by Moss: size cap]\n"); }
    v
}

async fn collect(st: &St, user: &User) -> ApiResult<Vec<(String, Vec<u8>)>> {
    let webhook = st.setting("notify_webhook_url", "").await;
    let mut secrets = crate::backups::remote::secrets::all_values(&st.cfg.data_dir);
    secrets.extend(crate::webhooks::secrets::all_values(&st.cfg.data_dir));
    for e in crate::webhooks::list(st).await.unwrap_or_default() { secrets.push(e.url); }
    let lits: Vec<&str> = std::iter::once(webhook.as_str()).chain(secrets.iter().map(String::as_str)).collect();
    let red = |s: &str| redact_text(s, &lits).into_bytes();
    let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
    let mut out: Vec<(String, Vec<u8>)> = vec![];

    let (total, free) = platform::disk(&st.cfg.data_dir);
    let st2 = st.clone();
    let java = tokio::task::spawn_blocking(move || crate::java::installs(&st2, false)).await.unwrap_or_default();
    let mut txt = format!("Moss {}\nOS: {} {}\nUptime: {}s\nData directory: {}\nDisk: {:.1} GiB free of {:.1} GiB\nAuth: {}\n\nDoctor:\n",
        env!("CARGO_PKG_VERSION"), platform::OS_LABEL, std::env::consts::ARCH, st.started.elapsed().as_secs(), st.cfg.data_dir.display(),
        free as f64 / (1u64 << 30) as f64, total as f64 / (1u64 << 30) as f64, if st.cfg.dev() { "dev (no login)" } else { "supabase" });
    txt.push_str(&format!("  [{}] data directory writable\n", if std::fs::create_dir_all(&st.cfg.data_dir).is_ok() { "ok" } else { "FAIL" }));
    txt.push_str(&format!("  [{}] disk space (>2 GiB free)\n", if free > (2 << 30) { "ok" } else { "warn" }));
    txt.push_str(&format!("  [{}] config valid{}\n", if st.cfg.validate().is_ok() { "ok" } else { "FAIL" }, st.cfg.validate().err().map_or(String::new(), |e| format!(": {e}"))));
    if java.is_empty() { txt.push_str("  [warn] no Java found\n"); }
    for j in java { txt.push_str(&format!("  [ok] java {} ({}){}\n", j.major, j.version, if j.managed { " [managed]" } else { "" })); }
    out.push(("moss.txt".into(), red(&txt)));

    let c = &st.cfg;
    let secret = |s: &str| if s.is_empty() { "" } else { "[redacted]" };
    let cfg = json!({"bind": c.bind, "port": c.port, "data_dir": c.data_dir, "auth": c.auth, "supabase_url": c.supabase_url, "supabase_anon_key": secret(&c.supabase_anon_key),
        "supabase_jwt_secret": secret(&c.supabase_jwt_secret), "curseforge_api_key": secret(&c.curseforge_api_key), "curseforge_api_url": c.curseforge_api_url,
        "curseforge_proxy": c.curseforge_proxy, "owner_emails": c.owner_email_list().iter().map(|e| redact::hash_email(e)).collect::<Vec<_>>(),
        "require_verified_email": c.require_verified_email, "allow_private_webhooks": c.allow_private_webhooks, "allow_private_backup_hosts": c.allow_private_backup_hosts, "dev_runtimes": c.dev_runtimes,
        "settings": settings_json(st, false).await});
    out.push(("config.json".into(), red(&pretty(&cfg))));

    // destinations without credentials (those are only ever reported as set / not set)
    let dests: Vec<Value> = crate::backups::remote::dest::list(st).await.unwrap_or_default().iter().map(|d| d.json(st)).collect();
    out.push(("backup-destinations.json".into(), red(&pretty(&Value::Array(dests)))));

    let servers = server::list(st).await?;
    out.push(("servers.json".into(), red(&pretty(&Value::Array(servers.iter().map(|s| server::view(s, st)).collect())))));
    out.push(("health.json".into(), red(&pretty(&Value::Array(crate::health::collect(st, user).await?)))));
    let acts: Vec<Value> = st.db.call(|c| {
        let mut q = c.prepare("SELECT id,time,actor,action,target,detail,server_id FROM activity ORDER BY id DESC LIMIT 500")?;
        q.query_map([], |r| Ok(json!({"id": r.get::<_, i64>(0)?, "time": r.get::<_, String>(1)?, "actor": r.get::<_, String>(2)?, "action": r.get::<_, String>(3)?, "target": r.get::<_, String>(4)?, "detail": r.get::<_, String>(5)?, "server_id": r.get::<_, Option<String>>(6)?})))?.collect()
    }).await?;
    out.push(("activity.json".into(), red(&pretty(&Value::Array(acts)))));
    out.push(("operations.json".into(), red(&pretty(&json!(ops::list(st, None, 300))))));
    for s in &servers {
        let p = s.moss_dir(st).join("console.log");
        let lines = tokio::task::spawn_blocking(move || crate::console::read_tail(&p, 256 * 1024)).await.unwrap_or_default();
        let tail: Vec<String> = lines.into_iter().rev().take(CONSOLE_LINES).rev().map(|l| l.chars().take(2000).collect()).collect();
        if !tail.is_empty() { out.push((format!("console/{}-{}.log", s.name.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect::<String>(), &s.id[..8.min(s.id.len())]), red(&tail.join("\n")))); }
    }
    let mut total = 0;
    Ok(out.into_iter().map(|(n, v)| (n, cap(v))).take_while(|(_, v)| { total += v.len(); total <= MAX_TOTAL }).collect())
}

pub(super) async fn support_bundle(State(st): State<St>, user: User, Query(q): Q) -> ApiResult<Response> {
    user.require_admin()?;
    let files = collect(&st, &user).await?;
    if q.contains_key("preview") {
        return Ok(Json(json!({"files": files.iter().map(|(n, v)| json!({"name": n, "size": v.len()})).collect::<Vec<_>>(), "redactions": redact::REDACTIONS})).into_response());
    }
    let path = st.cfg.tmp_dir().join(format!("support-{}.zip", uuid::Uuid::new_v4()));
    let p2 = path.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(o.open(&p2)?));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in files { zip.start_file(name, opts)?; zip.write_all(&data)?; }
        zip.finish()?.flush()?;
        Ok(())
    }).await.map_err(|e| anyhow::anyhow!(e))??;
    let f = tokio::fs::File::open(&path).await?;
    let _ = tokio::fs::remove_file(&path).await; // the open handle keeps the data alive until the download ends
    let len = f.metadata().await?.len();
    st.audit(&user.actor(), "support.bundle", "instance", "", None).await;
    let mut r = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(f)).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("attachment; filename=\"moss-support-{}.zip\"", chrono::Utc::now().format("%Y%m%d-%H%M%S"))).unwrap());
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    Ok(r)
}
