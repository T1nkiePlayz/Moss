//! Installing game servers: direct downloads (Bedrock zip, Terraria zip, Factorio tar.xz) and the generic SteamCMD
//! route (anonymous login, streamed output, progress parsed from SteamCMD's own lines, cancellable).
use super::{Game, Install, Launch, SETTINGS_FILE, read_settings};
use crate::{download, ops::OpCtx, platform, runtimes::Resolved, server::Srv, state::St};
use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use std::{
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    sync::{LazyLock, OnceLock},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;

const STEAMCMD_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz";
/// Hosts the game downloads may use (exact or subdomain), on top of the general download allow-list.
const GAME_HOSTS: &[&str] = &["terraria.org", "factorio.com", "akamaihd.net", "steamstatic.com", "steampowered.com"];

fn game_host(h: &str) -> bool { GAME_HOSTS.iter().any(|d| h == *d || h.ends_with(&format!(".{d}"))) || download::host_allowed(h) }

/// HTTP client whose redirects stay on the game download hosts (the shared client only follows the Minecraft ones).
fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| reqwest::Client::builder()
        // minecraft.net (Bedrock) never answers clients that do not present a browser-style "Mozilla/5.0" token.
        .user_agent(format!("Mozilla/5.0 (compatible; moss/{})", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() > 5 { return a.error("too many redirects"); }
            if a.url().scheme() == "https" && a.url().host_str().is_some_and(game_host) { a.follow() } else { a.error("redirect to a host that is not on the allow-list") }
        }))
        .connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(60)).build().expect("http client"))
}

async fn fetch(ctx: &OpCtx, url: &str, dest: &Path, scale: f64) -> Result<()> {
    let host = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(String::from)).filter(|h| game_host(h)).ok_or_else(|| anyhow!("refusing to download from {url}"))?;
    let c2 = ctx.clone();
    let sum = download::fetch_ext(client(), url, dest, None, &ctx.token, move |p| c2.progress(Some(p * scale)), download::Opts { extra_host: Some(host), ..Default::default() }).await?;
    ctx.log(format!("sha256 {sum} (upstream publishes no checksum we can verify)"));
    Ok(())
}

/// Install (or reinstall over) a game server's files in `dir`, then write first-run configuration.
pub async fn install(ctx: &OpCtx, st: &St, s: &Srv, r: &Resolved, dir: &Path) -> Result<()> {
    let g = super::get(&s.runtime).ok_or_else(|| anyhow!("unknown game {}", s.runtime))?;
    if !g.supported_here() { bail!("{} is not supported on this host ({})", g.name, g.platforms.join(" / ")); }
    let tmp = dir.join(".moss");
    tokio::fs::create_dir_all(&tmp).await?;
    match g.install {
        Install::Bedrock => {
            ctx.stage("Downloading Bedrock Dedicated Server");
            let zip = tmp.join("bedrock.zip");
            fetch(ctx, &r.url, &zip, 0.9).await?;
            ctx.check()?;
            ctx.stage("Extracting");
            let (z, d) = (zip.clone(), dir.to_path_buf());
            tokio::task::spawn_blocking(move || extract_zip(&z, &d, &|p| Some(p.to_path_buf()), &bedrock_keep)).await??;
            let _ = tokio::fs::remove_file(&zip).await;
        }
        Install::Terraria => {
            ctx.stage("Downloading Terraria dedicated server");
            let zip = tmp.join("terraria.zip");
            fetch(ctx, &r.url, &zip, 0.9).await?;
            ctx.check()?;
            ctx.stage("Extracting");
            let (z, d) = (zip.clone(), dir.to_path_buf());
            let n = tokio::task::spawn_blocking(move || extract_zip(&z, &d, &terraria_map, &|_| false)).await??;
            if n == 0 { bail!("the Terraria archive has no {} server folder", terraria_platform_dir()); }
            let _ = tokio::fs::remove_file(&zip).await;
        }
        Install::Factorio => {
            ctx.stage("Downloading Factorio headless");
            let xz = tmp.join("factorio.tar.xz");
            fetch(ctx, &r.url, &xz, 0.6).await?;
            ctx.check()?;
            ctx.stage("Unpacking (xz)");
            let (x, d) = (xz.clone(), dir.to_path_buf());
            tokio::task::spawn_blocking(move || unpack_tar_xz(&x, &d)).await??;
            let _ = tokio::fs::remove_file(&xz).await;
            ctx.progress(Some(0.85));
        }
        Install::Steam(app) => steam_install(ctx, st, app, dir).await?,
    }
    ctx.check()?;
    ctx.stage("Writing configuration");
    write_config(ctx, g, s, dir).await?;
    if g.install == Install::Factorio { create_factorio_save(ctx, dir).await?; }
    make_executable(g, s, dir);
    ctx.progress(Some(1.0));
    Ok(())
}

// ---------- archives ----------

/// Files that belong to the operator once the server has run; a reinstall must not overwrite them.
fn bedrock_keep(rel: &Path) -> bool {
    matches!(rel.to_str(), Some("server.properties" | "allowlist.json" | "permissions.json")) || rel.starts_with("worlds")
}

fn terraria_platform_dir() -> &'static str { if cfg!(target_os = "macos") { "Mac" } else { "Linux" } }

/// `1449/Linux/x` -> `x`; everything outside the platform folder is skipped.
pub fn terraria_map(p: &Path) -> Option<PathBuf> {
    let mut c = p.components();
    c.next()?;
    let plat = c.next()?;
    if !plat.as_os_str().eq_ignore_ascii_case(terraria_platform_dir()) { return None; }
    let rest: PathBuf = c.collect();
    (!rest.as_os_str().is_empty()).then_some(rest)
}

/// Extracts the entries `map` keeps into `dest` (entry names with `..` or absolute parts are dropped). Returns the
/// number of files written.
pub fn extract_zip(zip_path: &Path, dest: &Path, map: &dyn Fn(&Path) -> Option<PathBuf>, keep_existing: &dyn Fn(&Path) -> bool) -> Result<usize> {
    let mut ar = zip::ZipArchive::new(BufReader::new(std::fs::File::open(zip_path)?)).context("not a valid zip archive")?;
    let mut n = 0;
    for i in 0..ar.len() {
        let mut e = ar.by_index(i)?;
        let Some(rel) = e.enclosed_name().and_then(|p| map(&p)) else { continue };
        let out = dest.join(&rel);
        if e.is_dir() { std::fs::create_dir_all(&out)?; continue; }
        if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
        if keep_existing(&rel) && out.exists() { continue; }
        let mode = e.unix_mode();
        std::io::copy(&mut e, &mut BufWriter::new(std::fs::File::create(&out)?))?;
        if let Some(m) = mode { set_mode(&out, m & 0o777); }
        n += 1;
    }
    Ok(n)
}

fn set_mode(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));
}

/// Decompress `.tar.xz` (pure Rust, via a temporary `.tar`) and unpack it with the top-level folder stripped.
fn unpack_tar_xz(xz: &Path, dest: &Path) -> Result<()> {
    let tar_path = xz.with_extension("tar");
    {
        let mut input = BufReader::new(std::fs::File::open(xz)?);
        let mut out = BufWriter::new(std::fs::File::create(&tar_path)?);
        lzma_rs::xz_decompress(&mut input, &mut out).map_err(|e| anyhow!("could not decompress the xz archive: {e}"))?;
    }
    let res = (|| -> Result<()> {
        let mut ar = tar::Archive::new(BufReader::new(std::fs::File::open(&tar_path)?));
        for e in ar.entries()? {
            let mut e = e?;
            let path = e.path()?.into_owned();
            let rel: PathBuf = path.components().skip(1).collect();
            if rel.as_os_str().is_empty() || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) { continue; }
            let kind = e.header().entry_type();
            if !(kind.is_file() || kind.is_dir()) { continue; }
            let out = dest.join(&rel);
            if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
            e.unpack(&out)?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&tar_path);
    res
}

// ---------- processes ----------

fn kill_group(pid: Option<u32>) { if let Some(p) = pid { unsafe { libc::kill(-(p as i32), libc::SIGKILL); } } }

async fn pump(mut r: impl AsyncRead + Unpin, tx: tokio::sync::mpsc::UnboundedSender<String>) {
    let (mut buf, mut cur) = ([0u8; 4096], Vec::<u8>::new());
    while let Ok(n) = r.read(&mut buf).await {
        if n == 0 { break; }
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                if !cur.is_empty() { let _ = tx.send(String::from_utf8_lossy(&cur).into_owned()); cur.clear(); }
            } else { cur.push(b); }
        }
    }
    if !cur.is_empty() { let _ = tx.send(String::from_utf8_lossy(&cur).into_owned()); }
}

/// Runs a command with a clean environment, feeding every output line (split on CR or LF) to `on_line`.
/// Returns the exit code; fails on cancellation or timeout (the whole process group is killed).
pub async fn run_logged(token: &CancellationToken, argv: &[String], cwd: &Path, env: &[(&str, String)], limit: Duration, mut on_line: impl FnMut(&str)) -> Result<i32> {
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(cwd).env_clear().env("PATH", platform::GAME_PATH).env("LANG", platform::GAME_LANG).env("TERM", "dumb");
    for (k, v) in env { cmd.env(k, v); }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true).process_group(0);
    let mut child = cmd.spawn().with_context(|| format!("could not run {}", argv[0]))?;
    let pid = child.id();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(pump(child.stdout.take().unwrap(), tx.clone()));
    tokio::spawn(pump(child.stderr.take().unwrap(), tx));
    let deadline = tokio::time::sleep(limit);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            _ = token.cancelled() => { kill_group(pid); let _ = child.wait().await; bail!("cancelled"); }
            _ = &mut deadline => { kill_group(pid); let _ = child.wait().await; bail!("{} timed out", argv[0]); }
            l = rx.recv() => match l { Some(l) => on_line(&l), None => break },
        }
    }
    Ok(child.wait().await?.code().unwrap_or(-1))
}

// ---------- SteamCMD ----------

static PROGRESS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"progress:\s*(\d+(?:\.\d+)?)").unwrap());

/// Percentage from a SteamCMD line such as `Update state (0x61) downloading, progress: 12.34 (1 / 8)`.
pub fn parse_progress(line: &str) -> Option<f64> { PROGRESS.captures(line)?[1].parse().ok().filter(|p: &f64| (0.0..=100.0).contains(p)) }

fn steamcmd_args(sh: &Path, install_dir: &Path, app: u32) -> Vec<String> {
    vec![sh.to_string_lossy().into(), "+force_install_dir".into(), install_dir.to_string_lossy().into(), "+login".into(), "anonymous".into(), "+app_update".into(), app.to_string(), "validate".into(), "+quit".into()]
}

async fn ensure_steamcmd(ctx: &OpCtx, st: &St) -> Result<PathBuf> {
    let root = st.cfg.data_dir.join("steamcmd");
    let sh = root.join("steamcmd.sh");
    if sh.exists() { return Ok(sh); }
    tokio::fs::create_dir_all(root.join("home")).await?;
    ctx.stage("Downloading SteamCMD");
    let tgz = root.join("steamcmd_linux.tar.gz");
    fetch(ctx, STEAMCMD_URL, &tgz, 0.05).await?;
    let (t, r) = (tgz.clone(), root.clone());
    tokio::task::spawn_blocking(move || -> Result<()> {
        tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(std::fs::File::open(&t)?))).unpack(&r)?;
        std::fs::remove_file(&t)?;
        Ok(())
    }).await??;
    if !sh.exists() { bail!("the SteamCMD archive did not contain steamcmd.sh"); }
    set_mode(&sh, 0o755);
    Ok(sh)
}

static STEAM_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn steam_install(ctx: &OpCtx, st: &St, app: u32, dir: &Path) -> Result<()> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) { bail!("SteamCMD servers need Linux on x86_64"); }
    ctx.log("Waiting for any other SteamCMD install to finish");
    let _one_at_a_time = STEAM_LOCK.lock().await; // SteamCMD keeps shared state in its home directory
    let sh = ensure_steamcmd(ctx, st).await?;
    let abs = std::fs::canonicalize(dir)?;
    let home = st.cfg.data_dir.join("steamcmd").join("home");
    tokio::fs::create_dir_all(&home).await?;
    let argv = steamcmd_args(&sh, &abs, app);
    let mut last_err = String::new();
    // SteamCMD updates itself on first run and often exits non-zero after doing so; a second run then succeeds.
    for attempt in 1..=3 {
        ctx.check()?;
        ctx.stage(&format!("Downloading app {app} with SteamCMD (attempt {attempt})"));
        let (mut ok, mut logged) = (false, -10.0f64);
        let mut tail: Vec<String> = vec![];
        let c2 = ctx.clone();
        let code = run_logged(&ctx.token, &argv, sh.parent().unwrap_or(dir), &[("HOME", home.to_string_lossy().into_owned())], Duration::from_secs(4 * 3600), |l| {
            if let Some(p) = parse_progress(l) {
                c2.progress(Some(0.05 + p / 100.0 * 0.9));
                if p - logged >= 10.0 || p >= 100.0 { logged = p; c2.log(format!("progress {p:.0}%")); }
                return;
            }
            if l.contains("Success! App") { ok = true; }
            c2.log(l.to_string());
            tail.push(l.to_string());
            if tail.len() > 6 { tail.remove(0); }
        }).await?;
        if ok { return Ok(()); }
        last_err = format!("SteamCMD exited with code {code}: {}", tail.join(" | "));
        ctx.log(last_err.clone());
    }
    bail!("{last_err}. SteamCMD is a 32-bit program: on Debian/Ubuntu install lib32gcc-s1 and lib32stdc++6 (Fedora: glibc.i686 libstdc++.i686), then retry the install")
}

// ---------- configuration ----------

/// Replace or append `key=value` lines in a properties-style file.
pub fn set_properties(text: &str, kv: &[(&str, String)]) -> String {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    for (k, v) in kv {
        let pre = format!("{k}=");
        match lines.iter_mut().find(|l| l.trim_start().starts_with(&pre)) {
            Some(l) => *l = format!("{k}={v}"),
            None => lines.push(format!("{k}={v}")),
        }
    }
    lines.join("\n") + "\n"
}

/// Terraria `serverconfig.txt` for a fresh medium world.
pub fn terraria_config(dir: &Path, port: i64, max_players: u32, name: &str) -> String {
    let world_name: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-')).collect();
    let world_name = if world_name.trim().is_empty() { "Moss".to_string() } else { world_name.trim().to_string() };
    let worlds = dir.join("worlds");
    format!("# Written by Moss on first install; edit freely (changes apply on the next start).\nworld={}\nworldpath={}\nautocreate=2\nworldname={world_name}\ndifficulty=0\nmaxplayers={max_players}\nport={port}\npassword=\nmotd=Welcome!\nsecure=1\nlanguage=en-US\nupnp=0\n",
        worlds.join("moss.wld").display(), worlds.display())
}

/// Factorio `server-settings.json` from the example Factorio ships, kept off the public matchmaking list.
pub fn factorio_settings(example: &str, name: &str) -> Result<String> {
    let mut v: serde_json::Value = serde_json::from_str(example).context("server-settings.example.json is not valid JSON")?;
    v["name"] = name.into();
    v["description"] = "Hosted with Moss".into();
    v["visibility"] = serde_json::json!({"public": false, "lan": false});
    Ok(serde_json::to_string_pretty(&v)? + "\n")
}

fn settings_text(g: &Game, s: &Srv) -> Option<String> {
    let pw = crate::auth::secret::random_string(12);
    let head = format!("# Moss settings for this {} server. Edit and restart to apply. Optional: max_players=N\n", g.name);
    match g.id {
        "valheim" => {
            let name: String = s.name.chars().filter(|c| !c.is_control()).collect();
            Some(format!("{head}name={name}\nworld=Dedicated\npassword={pw}\n"))
        }
        "zomboid" => Some(format!("{head}servername=moss\npassword={pw}\n")),
        _ => None,
    }
}

async fn write_if_absent(path: &Path, text: String) -> Result<bool> {
    if tokio::fs::try_exists(path).await.unwrap_or(false) { return Ok(false); }
    if let Some(p) = path.parent() { tokio::fs::create_dir_all(p).await?; }
    tokio::fs::write(path, text).await?;
    Ok(true)
}

async fn write_config(ctx: &OpCtx, g: &Game, s: &Srv, dir: &Path) -> Result<()> {
    match g.install {
        Install::Bedrock => {
            let p = dir.join("server.properties");
            let old = tokio::fs::read_to_string(&p).await.unwrap_or_default();
            let mut kv = vec![("server-port", s.port.to_string())];
            if s.port < 65535 { kv.push(("server-portv6", (s.port + 1).to_string())); }
            tokio::fs::write(&p, set_properties(&old, &kv)).await?;
        }
        Install::Terraria => { write_if_absent(&dir.join("serverconfig.txt"), terraria_config(dir, s.port, g.max_players, &s.name)).await?; }
        Install::Factorio => {
            let example = tokio::fs::read_to_string(dir.join("data/server-settings.example.json")).await.context("Factorio's server-settings.example.json is missing")?;
            write_if_absent(&dir.join("config/server-settings.json"), factorio_settings(&example, &s.name)?).await?;
        }
        Install::Steam(_) => {
            if let Some(t) = settings_text(g, s) && write_if_absent(&dir.join(SETTINGS_FILE), t).await? {
                ctx.log(format!("Wrote {SETTINGS_FILE} with a generated password; edit it from the Files tab."));
            }
        }
    }
    Ok(())
}

async fn create_factorio_save(ctx: &OpCtx, dir: &Path) -> Result<()> {
    if tokio::fs::try_exists(dir.join("saves/moss.zip")).await.unwrap_or(false) { return Ok(()); }
    ctx.stage("Creating the first Factorio save");
    tokio::fs::create_dir_all(dir.join("saves")).await?;
    let exe = dir.join("bin/x64/factorio");
    set_mode(&exe, 0o755);
    let c2 = ctx.clone();
    let argv = vec![exe.to_string_lossy().into_owned(), "--create".into(), "saves/moss.zip".into()];
    let code = run_logged(&ctx.token, &argv, dir, &[("HOME", dir.to_string_lossy().into_owned())], Duration::from_secs(300), move |l| c2.log(l.to_string())).await?;
    if code != 0 || !dir.join("saves/moss.zip").exists() { bail!("Factorio could not create the first save (exit code {code})"); }
    Ok(())
}

/// Zip archives and some Steam depots do not keep the executable bit; set it on whatever the launch command runs.
fn make_executable(g: &Game, s: &Srv, dir: &Path) {
    let cfg = read_settings(dir);
    let Ok(port) = u16::try_from(s.port) else { return };
    let Ok(argv) = (g.launch)(&Launch { port, max_players: g.max_players, name: &s.name, cfg: &cfg }) else { return };
    for exe in super::executables(&argv) {
        let p = dir.join(exe);
        if p.is_file() { set_mode(&p, 0o755); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn steam_progress_lines() {
        assert_eq!(parse_progress(" Update state (0x61) downloading, progress: 12.34 (123 / 1000)"), Some(12.34));
        assert_eq!(parse_progress("Update state (0x81) verifying update, progress: 100.00 (1 / 1)"), Some(100.0));
        assert_eq!(parse_progress("Update state (0x5) validating, progress: 250.00"), None);
        assert_eq!(parse_progress("Success! App '896660' fully installed."), None);
    }

    #[test]
    fn steamcmd_command_line() {
        let a = steamcmd_args(Path::new("/d/steamcmd.sh"), Path::new("/srv/x"), 896660);
        assert_eq!(a[1..], ["+force_install_dir", "/srv/x", "+login", "anonymous", "+app_update", "896660", "validate", "+quit"]);
    }

    #[test]
    fn properties_are_patched_in_place() {
        let out = set_properties("server-name=Dedicated\nserver-port=19132\nserver-portv6=19133\n", &[("server-port", "20000".into()), ("server-portv6", "20001".into()), ("new-key", "1".into())]);
        assert_eq!(out, "server-name=Dedicated\nserver-port=20000\nserver-portv6=20001\nnew-key=1\n");
    }

    #[test]
    fn terraria_files() {
        let t = terraria_config(Path::new("/srv/t"), 7778, 8, "My Server!");
        assert!(t.contains("port=7778\n") && t.contains("maxplayers=8\n") && t.contains("worldname=My Server\n") && t.contains("world=/srv/t/worlds/moss.wld\n") && t.contains("autocreate=2\n"));
        assert!(terraria_config(Path::new("/x"), 1, 1, "!!!").contains("worldname=Moss\n"));
        let dir = terraria_platform_dir();
        assert_eq!(terraria_map(&Path::new("1449").join(dir).join("lib64").join("x.so")), Some(PathBuf::from("lib64/x.so")));
        assert_eq!(terraria_map(Path::new("1449/Windows/x.exe")), None);
        assert_eq!(terraria_map(&Path::new("1449").join(dir)), None);
    }

    #[test]
    fn factorio_settings_are_private_and_named() {
        let out = factorio_settings(r#"{"name":"Name","visibility":{"public":true,"lan":true},"max_players":0}"#, "Mine").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!((v["name"].as_str(), v["visibility"]["public"].as_bool(), v["max_players"].as_i64()), (Some("Mine"), Some(false), Some(0)));
        assert!(factorio_settings("nope", "x").is_err());
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("moss-games-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn zip_extraction_maps_keeps_and_refuses_traversal() {
        let d = scratch("zip");
        let zp = d.join("a.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
            let o = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            for (n, body) in [("bedrock_server", "bin"), ("server.properties", "from-zip"), ("worlds/w/db", "x"), ("../escape.txt", "bad"), ("sub/a.txt", "a")] {
                w.start_file(n, o).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        let out = d.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("server.properties"), "mine").unwrap();
        let n = extract_zip(&zp, &out, &|p| Some(p.to_path_buf()), &bedrock_keep).unwrap();
        assert_eq!(n, 3, "bedrock_server, worlds/w/db, sub/a.txt");
        assert_eq!(std::fs::read_to_string(out.join("server.properties")).unwrap(), "mine");
        assert!(!d.join("escape.txt").exists() && out.join("sub/a.txt").exists());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(out.join("bedrock_server")).unwrap().permissions().mode() & 0o111, 0o111);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn run_logged_splits_lines_and_reports_exit_code() {
        let d = scratch("run");
        let mut lines = vec![];
        let argv: Vec<String> = ["sh", "-c", "printf 'a\\rb\\nc\\n'; echo err >&2; exit 3"].iter().map(|s| s.to_string()).collect();
        let code = run_logged(&CancellationToken::new(), &argv, &d, &[], Duration::from_secs(10), |l| lines.push(l.to_string())).await.unwrap();
        lines.sort();
        assert_eq!((code, lines), (3, vec!["a".to_string(), "b".into(), "c".into(), "err".into()]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn run_logged_can_be_cancelled() {
        let d = scratch("cancel");
        let token = CancellationToken::new();
        let t2 = token.clone();
        tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(200)).await; t2.cancel(); });
        let argv: Vec<String> = ["sleep", "30"].iter().map(|s| s.to_string()).collect();
        let started = std::time::Instant::now();
        let err = run_logged(&token, &argv, &d, &[], Duration::from_secs(60), |_| {}).await.unwrap_err();
        assert!(err.to_string().contains("cancelled") && started.elapsed().as_secs() < 10);
        let _ = std::fs::remove_dir_all(&d);
    }
}
