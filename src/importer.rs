//! "Import an existing server": bring a server directory that was not created by Moss under Moss's management.
//! Sources are an uploaded zip or tar.zst (extracted into a private staging directory) or an absolute path on the
//! host (administrators only; the directory is copied, never moved or modified). The runtime, version, port and a
//! suitable Java are detected from the files; anything that cannot be detected must be chosen by the person importing.
use crate::{auth::User, error::{ApiError, ApiResult}, ops::OpCtx, state::St};
use anyhow::{Result, anyhow, bail};
use axum::{Json, extract::{Multipart, State}, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};

const MAX_BYTES: u64 = 200 << 30;
/// What an uploaded archive may expand to: the upload itself is capped at 512 MiB, so a larger result is a decompression bomb.
const UPLOAD_EXPANSION: u64 = 64 << 30;
const MAX_ENTRIES: usize = 1_000_000;
const STAGE_TTL_SECS: u64 = 6 * 3600;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Detection {
    pub runtime: Option<String>,
    pub game_version: Option<String>,
    pub loader_version: Option<String>,
    pub port: Option<i64>,
    pub jar: Option<String>,
    pub has_world: bool,
    pub notes: Vec<String>,
}

fn read(p: &Path) -> Option<String> { std::fs::read_to_string(p).ok() }
fn exists(root: &Path, rel: &str) -> bool { root.join(rel).exists() }

/// First `1.x[.y]` style version found in a file name.
fn version_in_name(n: &str) -> Option<String> {
    let b = n.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'1' && b.get(i + 1) == Some(&b'.') && (i == 0 || !b[i - 1].is_ascii_digit()) {
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || (b[j] == b'.' && b.get(j + 1).is_some_and(|c| c.is_ascii_digit()))) { j += 1; }
            if j > i + 2 { return Some(n[i..j].to_string()); }
        }
        i += 1;
    }
    None
}

/// Facts read from inside a server jar (a zip file).
#[derive(Default, Debug)]
struct JarFacts { mc_version: Option<String>, velocity: bool, fabric: bool, quilt: bool, paperclip: bool, vanilla: bool }

fn inspect_jar(p: &Path) -> JarFacts {
    use std::io::Read;
    let mut f = JarFacts::default();
    let Ok(file) = std::fs::File::open(p) else { return f };
    let Ok(mut z) = zip::ZipArchive::new(file) else { return f };
    let names: Vec<String> = z.file_names().take(20000).flatten().map(|c| c.into_owned()).collect();
    let has = |pre: &str| names.iter().any(|n| n.starts_with(pre));
    f.velocity = has("com/velocitypowered/proxy/");
    f.fabric = has("net/fabricmc/");
    f.quilt = has("org/quiltmc/");
    f.paperclip = names.iter().any(|n| n == "patch.properties" || n.starts_with("io/papermc/paperclip/"));
    f.vanilla = names.iter().any(|n| n == "net/minecraft/server/MinecraftServer.class" || n == "net/minecraft/bundler/Main.class" || n == "version.json");
    let mut text = |name: &str| -> Option<String> { let mut s = String::new(); z.by_name(name).ok()?.take(1 << 20).read_to_string(&mut s).ok()?; Some(s) };
    if let Some(v) = text("version.json").and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        f.mc_version = v["id"].as_str().or(v["name"].as_str()).filter(|s| version_in_name(s).is_some() || s.contains('w')).map(String::from);
    }
    if f.mc_version.is_none() && let Some(t) = text("patch.properties") { f.mc_version = t.lines().find_map(|l| l.strip_prefix("version=")).map(|v| v.trim().to_string()); }
    if f.mc_version.is_none() && let Some(t) = text("META-INF/versions.list") {
        f.mc_version = t.lines().filter_map(|l| l.split('\t').nth(2)).find_map(|path| path.split('/').next().and_then(version_in_name));
    }
    f
}

fn properties_port(root: &Path) -> Option<i64> {
    read(&root.join("server.properties"))?.lines().find_map(|l| l.trim().strip_prefix("server-port=")?.trim().parse::<i64>().ok()).filter(|p| (1..=65535).contains(p))
}
fn velocity_port(root: &Path) -> Option<i64> {
    let v: toml::Value = toml::from_str(&read(&root.join("velocity.toml"))?).ok()?;
    v.get("bind")?.as_str()?.rsplit(':').next()?.parse().ok().filter(|p| (1..=65535).contains(p))
}
fn log_version(root: &Path) -> Option<String> {
    let l = read(&root.join("logs/latest.log"))?;
    l.lines().take(200).find_map(|x| x.split("Starting minecraft server version ").nth(1)).map(|v| v.trim().to_string()).filter(|v| version_in_name(v).is_some())
}

/// NeoForge versions encode the Minecraft version: 21.4.x is 1.21.4, 20.2.x is 1.20.2, 21.0.x is 1.21.
fn neoforge_game(v: &str) -> Option<String> {
    let mut p = v.split('.');
    let (a, b) = (p.next()?.parse::<u32>().ok()?, p.next()?.parse::<u32>().ok()?);
    Some(if b == 0 { format!("1.{a}") } else { format!("1.{a}.{b}") })
}

fn first_dir_entry(p: &Path) -> Option<String> {
    let mut v: Vec<String> = std::fs::read_dir(p).ok()?.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    v.sort();
    v.pop()
}

/// Works out what a server directory is. Never writes anything.
pub fn detect(root: &Path) -> Detection {
    let mut d = Detection { has_world: exists(root, "world/level.dat") || std::fs::read_dir(root).ok().is_some_and(|rd| rd.flatten().any(|e| e.path().join("level.dat").is_file())), ..Default::default() };
    let jars: Vec<String> = std::fs::read_dir(root).map(|rd| rd.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.to_ascii_lowercase().ends_with(".jar")).collect()).unwrap_or_default();
    let mut jars = jars;
    jars.sort();
    let pick = |pred: &dyn Fn(&str) -> bool| jars.iter().find(|n| pred(&n.to_ascii_lowercase())).cloned();
    // Forge and NeoForge are launched through generated argument files
    if let Some(v) = first_dir_entry(&root.join("libraries/net/neoforged/neoforge")).filter(|v| exists(root, &format!("libraries/net/neoforged/neoforge/{v}/unix_args.txt"))) {
        d.game_version = neoforge_game(&v);
        d.runtime = Some("neoforge".into());
        d.loader_version = Some(v);
    } else if let Some(v) = first_dir_entry(&root.join("libraries/net/minecraftforge/forge")).filter(|v| exists(root, &format!("libraries/net/minecraftforge/forge/{v}/unix_args.txt"))) {
        let (mc, lv) = v.split_once('-').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((String::new(), v.clone()));
        d.runtime = Some("forge".into());
        d.game_version = (!mc.is_empty()).then_some(mc);
        d.loader_version = Some(lv);
    }
    if d.runtime.is_none() {
        let launch = pick(&|n| n == "fabric-server-launch.jar").or_else(|| pick(&|n| n == "quilt-server-launch.jar"));
        let candidates: Vec<String> = launch.iter().cloned().chain(pick(&|n| n == "server.jar")).chain(jars.iter().filter(|n| !["server.jar"].contains(&n.as_str())).cloned()).collect();
        let jar = candidates.first().cloned();
        let facts = jar.as_ref().map(|j| inspect_jar(&root.join(j))).unwrap_or_default();
        let lname = jar.as_deref().unwrap_or_default().to_ascii_lowercase();
        let rt = if exists(root, "velocity.toml") || lname.contains("velocity") || facts.velocity { Some("velocity") }
            else if lname == "quilt-server-launch.jar" || facts.quilt { Some("quilt") }
            else if lname == "fabric-server-launch.jar" || exists(root, "fabric-server-launcher.properties") || exists(root, ".fabric") || facts.fabric { Some("fabric") }
            else if exists(root, "purpur.yml") || lname.contains("purpur") { Some("purpur") }
            else if lname.contains("folia") { Some("folia") }
            else if exists(root, "config/paper-global.yml") || exists(root, "paper.yml") || exists(root, "config/paper-world-defaults.yml") || lname.contains("paper") || facts.paperclip { Some("paper") }
            else if exists(root, "spigot.yml") || lname.contains("spigot") || lname.contains("craftbukkit") { d.notes.push("This looks like Spigot or CraftBukkit, which Moss cannot run yet. Paper runs Spigot plugins; pick Paper to switch.".into()); None }
            else if facts.vanilla || lname.contains("minecraft_server") || lname == "server.jar" && exists(root, "server.properties") { Some("vanilla") }
            else { None };
        d.runtime = rt.map(String::from);
        d.jar = jar.clone();
        d.game_version = facts.mc_version.clone();
        if rt == Some("velocity") {
            d.game_version = jar.as_deref().and_then(|j| { let l = j.to_ascii_lowercase(); let i = l.find("velocity-")? + 9; let v: String = l[i..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect(); (!v.is_empty()).then_some(v.trim_end_matches('.').to_string()) });
        }
        if matches!(rt, Some("fabric") | Some("quilt")) {
            d.loader_version = read(&root.join(".fabric-installer/install.properties")).and_then(|t| t.lines().find_map(|l| l.strip_prefix("fabric-loader-version=").map(|v| v.trim().to_string())));
            if let Some(t) = read(&root.join(".fabric-installer/launcher.json")).and_then(|t| serde_json::from_str::<Value>(&t).ok()) && let Some(id) = t["id"].as_str() {
                let parts: Vec<&str> = id.splitn(4, '-').collect(); // fabric-loader-<loader>-<mc>
                if parts.len() == 4 { d.loader_version.get_or_insert(parts[2].to_string()); d.game_version.get_or_insert(parts[3].to_string()); }
            }
            if d.game_version.is_none() { d.game_version = first_dir_entry(&root.join(".fabric/server")).filter(|v| version_in_name(v).is_some()); }
        }
        if d.game_version.is_none() && rt != Some("velocity") { d.game_version = jar.as_deref().and_then(version_in_name); }
    }
    if d.game_version.is_none() && d.runtime.as_deref() != Some("velocity") { d.game_version = log_version(root); }
    if d.runtime.as_deref() == Some("velocity") { d.port = velocity_port(root); } else { d.port = properties_port(root); }
    if d.runtime.is_none() && d.notes.is_empty() { d.notes.push("Moss could not tell what kind of server this is. Choose the runtime below.".into()); }
    if d.runtime.is_some() && d.game_version.is_none() { d.notes.push("The Minecraft version could not be detected. Enter it below.".into()); }
    d
}

fn java_for(runtime: &str, game: &str) -> u32 { if runtime == "velocity" { 21 } else { crate::runtimes::fallback_java(game) } }

fn view(st: &St, d: &Detection, root_name: &str, size: u64) -> Value {
    let java = d.runtime.as_deref().map(|r| java_for(r, d.game_version.as_deref().unwrap_or("1.21")));
    json!({"runtime": d.runtime, "game_version": d.game_version, "loader_version": d.loader_version, "port": d.port, "jar": d.jar, "has_world": d.has_world,
        "java_major": java, "java_installed": java.is_none_or(|j| crate::java::find(st, j).is_some()), "name": root_name, "size": size, "notes": d.notes})
}

fn size_of(root: &Path) -> u64 {
    fn walk(p: &Path) -> u64 { std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| match e.file_type() { Ok(t) if t.is_dir() => walk(&e.path()), Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0), _ => 0 }).sum()).unwrap_or(0) }
    walk(root)
}

/// A host directory the importer may read: absolute, a real directory, not part of system locations or of Moss's own data.
pub fn check_host_path(st: &St, raw: &str) -> ApiResult<PathBuf> {
    let bad = |m: &str| ApiError::bad("import.path", m.to_string()).field("path", m);
    if raw.is_empty() || raw.contains('\0') || raw.len() > 4096 { return Err(bad("Enter the full path of the server folder.")); }
    let p = Path::new(raw);
    if !p.is_absolute() { return Err(bad("The path must be absolute, for example /srv/minecraft/survival.")); }
    if p.components().any(|c| matches!(c, Component::ParentDir)) { return Err(bad("The path must not contain \"..\".")); }
    let canon = std::fs::canonicalize(p).map_err(|_| bad("That folder does not exist or Moss cannot read it."))?;
    if !canon.is_dir() { return Err(bad("That path is a file, not a folder.")); }
    if canon.components().count() < 2 || ["/etc", "/proc", "/sys", "/dev", "/boot", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/run", "/var/lib", "/root/.ssh"].iter().any(|s| canon.starts_with(s)) { return Err(bad("That is a system folder, not a Minecraft server.")); }
    let data = std::fs::canonicalize(&st.cfg.data_dir).unwrap_or_else(|_| st.cfg.data_dir.clone());
    if canon.starts_with(&data) || data.starts_with(&canon) { return Err(bad("That folder is, or contains, Moss's own data folder.")); }
    Ok(canon)
}

fn stage_root(st: &St) -> PathBuf { st.cfg.tmp_dir() }
fn token_ok(t: &str) -> bool { t.len() == 36 && t.chars().all(|c| c.is_ascii_hexdigit() || c == '-') }
fn staged(st: &St, token: &str) -> ApiResult<PathBuf> {
    if !token_ok(token) { return Err(ApiError::bad("import.token", "That upload is not valid. Upload the file again.")); }
    let p = stage_root(st).join(format!("import-{token}")).join("root");
    if !p.is_dir() { return Err(ApiError::bad("import.token", "That upload has expired. Upload the file again.")); }
    Ok(p)
}
async fn sweep(st: &St) {
    let Ok(mut rd) = tokio::fs::read_dir(stage_root(st)).await else { return };
    while let Ok(Some(e)) = rd.next_entry().await {
        let old = e.metadata().await.ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > STAGE_TTL_SECS);
        if old && e.file_name().to_string_lossy().starts_with("import-") { let _ = tokio::fs::remove_dir_all(e.path()).await; }
    }
}

/// Unpacks a zip or tar.zst into `dest` with every entry confined to it. Symlinks and special files are refused.
pub fn extract(archive: &Path, dest: &Path, token: &tokio_util::sync::CancellationToken) -> Result<()> { extract_limited(archive, dest, token, UPLOAD_EXPANSION) }

fn extract_limited(archive: &Path, dest: &Path, token: &tokio_util::sync::CancellationToken, max_bytes: u64) -> Result<()> {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(archive)?.read_exact(&mut magic).map_err(|_| anyhow!("The file is empty or too small to be an archive."))?;
    std::fs::create_dir_all(dest)?;
    if &magic == b"PK\x03\x04" {
        let mut z = zip::ZipArchive::new(std::fs::File::open(archive)?).map_err(|e| anyhow!("The zip file could not be read: {e}"))?;
        let mut total = 0u64;
        if z.len() > MAX_ENTRIES { bail!("The archive has too many files."); }
        for i in 0..z.len() {
            if token.is_cancelled() { bail!("cancelled"); }
            let mut f = z.by_index(i)?;
            let rel = f.enclosed_name().ok_or_else(|| anyhow!("The archive contains an unsafe path ({}).", f.name().unwrap_or_default()))?;
            if f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) { bail!("The archive contains a symbolic link ({}), which Moss will not import.", rel.display()); }
            if rel.components().any(|c| !matches!(c, Component::Normal(_))) { bail!("The archive contains an unsafe path ({}).", rel.display()); }
            let out = dest.join(&rel);
            if f.is_dir() { std::fs::create_dir_all(&out)?; continue; }
            if total.saturating_add(f.size()) > max_bytes { bail!("The archive expands beyond the size limit."); }
            if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
            // count what is really written: the size in the header is attacker-controlled
            let room = max_bytes - total;
            let n = std::io::copy(&mut f.by_ref().take(room.saturating_add(1)), &mut std::fs::File::create(&out)?)?;
            total = total.saturating_add(n);
            if total > max_bytes { bail!("The archive expands beyond the size limit."); }
        }
        Ok(())
    } else if magic == [0x28, 0xB5, 0x2F, 0xFD] {
        crate::backups::extract_archive_limited(archive, dest, token, None, max_bytes)
    } else {
        bail!("Only .zip and .tar.zst files can be imported.")
    }
}

/// An archive must not bring its own controller metadata (`.moss`), like the folder import which never copies it.
fn strip_reserved(root: &Path) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        if n == ".moss" { let _ = if e.path().is_dir() { std::fs::remove_dir_all(e.path()) } else { std::fs::remove_file(e.path()) }; }
    }
}

/// If the archive had a single top-level folder (the usual case when zipping a server folder), use that folder.
fn effective_root(dir: &Path) -> PathBuf {
    let entries: Vec<_> = std::fs::read_dir(dir).map(|rd| rd.flatten().filter(|e| e.file_name() != "__MACOSX").collect()).unwrap_or_default();
    if entries.len() == 1 && entries[0].path().is_dir() { entries[0].path() } else { dir.to_path_buf() }
}

pub async fn inspect(State(st): State<St>, user: User, req: axum::extract::Request) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    sweep(&st).await;
    let ct = req.headers().get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    if ct.starts_with("multipart/") {
        let mut mp = <Multipart as axum::extract::FromRequest<()>>::from_request(req, &()).await.map_err(|e| ApiError::bad("upload.invalid", e.to_string()))?;
        let token = uuid::Uuid::now_v7().to_string();
        let base = stage_root(&st).join(format!("import-{token}"));
        tokio::fs::create_dir_all(&base).await?;
        let res: ApiResult<(String, u64)> = async {
            let mut orig = String::from("server");
            while let Some(mut f) = mp.next_field().await.map_err(|e| ApiError::bad("upload.invalid", e.to_string()))? {
                let Some(name) = f.file_name().map(String::from) else { continue };
                orig = Path::new(&name).file_stem().map(|s| s.to_string_lossy().trim_end_matches(".tar").to_string()).unwrap_or(orig);
                let mut out = tokio::fs::File::create(base.join("upload.bin")).await?;
                use tokio::io::AsyncWriteExt;
                while let Some(c) = f.chunk().await.map_err(|e| ApiError::bad("upload.invalid", e.to_string()))? { out.write_all(&c).await?; }
                out.flush().await?;
                let (a, d) = (base.join("upload.bin"), base.join("extracted"));
                tokio::task::spawn_blocking(move || extract(&a, &d, &tokio_util::sync::CancellationToken::new())).await.map_err(|e| ApiError::bad("import.failed", e.to_string()))?
                    .map_err(|e| ApiError::bad("import.archive", e.to_string()))?;
                let _ = tokio::fs::remove_file(base.join("upload.bin")).await;
                let root = effective_root(&base.join("extracted"));
                strip_reserved(&root);
                tokio::fs::rename(&root, base.join("root")).await?;
                let r2 = base.join("root");
                return Ok((orig, tokio::task::spawn_blocking(move || size_of(&r2)).await.unwrap_or(0)));
            }
            Err(ApiError::bad("upload.invalid", "No file was uploaded."))
        }.await;
        match res {
            Ok((name, size)) => {
                let root = base.join("root");
                let d = { let r = root.clone(); tokio::task::spawn_blocking(move || detect(&r)).await.unwrap_or_default() };
                let mut v = view(&st, &d, &name, size);
                v["token"] = json!(token);
                Ok(Json(v))
            }
            Err(e) => { let _ = tokio::fs::remove_dir_all(&base).await; Err(e) }
        }
    } else {
        let body = axum::body::to_bytes(req.into_body(), 64 * 1024).await.map_err(|_| ApiError::bad("validation", "invalid request"))?;
        let r: PathReq = serde_json::from_slice(&body).map_err(|_| ApiError::bad("validation", "expected {\"path\": \"/absolute/path\"}"))?;
        let p = check_host_path(&st, r.path.trim())?;
        let (p2, name) = (p.clone(), p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        let (d, size) = tokio::task::spawn_blocking(move || (detect(&p2), size_of(&p2))).await.map_err(|e| ApiError::bad("import.failed", e.to_string()))?;
        if d.runtime.is_none() && d.port.is_none() && !p.join("server.properties").exists() && !d.notes.iter().any(|n| n.contains("Spigot")) && d.jar.is_none() {
            return Err(ApiError::bad("import.not_a_server", "That folder does not look like a Minecraft server: it has no server.properties and no .jar file.").field("path", "no server files found"));
        }
        Ok(Json(view(&st, &d, &name, size)))
    }
}

#[derive(Deserialize)]
pub struct PathReq { path: String }

#[derive(Deserialize)]
pub struct ImportReq {
    name: String, path: Option<String>, token: Option<String>, runtime: Option<String>, game_version: Option<String>, loader_version: Option<String>,
    memory_mb: Option<i64>, port: Option<i64>, accept_eula: Option<bool>,
}

fn copy_tree(src: &Path, dst: &Path, ctx: &OpCtx, done: &mut u64, total: u64, skipped: &mut u64, top: bool) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        ctx.check()?;
        let e = e?;
        let name = e.file_name();
        if top && (name == ".moss" || name == "session.lock") { continue; }
        let t = e.file_type()?;
        let to = dst.join(&name);
        if t.is_symlink() { *skipped += 1; continue; }
        if t.is_dir() { copy_tree(&e.path(), &to, ctx, done, total, skipped, false)?; }
        else if t.is_file() {
            std::fs::copy(e.path(), &to)?;
            *done += e.metadata().map(|m| m.len()).unwrap_or(0);
            ctx.progress(Some((*done as f64 / total.max(1) as f64).min(0.99)));
        }
    }
    Ok(())
}

/// Makes sure the launcher Moss starts (`server.jar`) exists, copying the detected jar to that name when needed.
fn normalise_launcher(dir: &Path, runtime: &str, jar: Option<&str>) -> Result<Option<String>> {
    if matches!(runtime, "forge" | "neoforge" | "quilt") || dir.join("server.jar").exists() { return Ok(None); }
    let Some(j) = jar.filter(|j| !j.contains(['/', '\\'])) else { bail!("No server .jar was found in this folder, so Moss would not be able to start it.") };
    std::fs::copy(dir.join(j), dir.join("server.jar"))?;
    Ok(Some(format!("Copied {j} to server.jar so Moss can start it (the original is still there).")))
}

pub async fn start(State(st): State<St>, user: User, Json(r): Json<ImportReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let name = r.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 48 || name.chars().any(|c| c.is_control()) { return Err(ApiError::bad("validation", "invalid name").field("name", "1–48 characters, no control characters")); }
    let (src, is_upload) = match (&r.path, &r.token) {
        (Some(p), None) => (check_host_path(&st, p.trim())?, false),
        (None, Some(t)) => (staged(&st, t)?, true),
        _ => return Err(ApiError::bad("validation", "give either a path or an upload token")),
    };
    let src2 = src.clone();
    let d = tokio::task::spawn_blocking(move || detect(&src2)).await.map_err(|e| ApiError::bad("import.failed", e.to_string()))?;
    let runtime = r.runtime.clone().or(d.runtime.clone()).ok_or_else(|| ApiError::bad("runtime.unsupported", "Choose the server type.").field("runtime", "required"))?;
    let cat = crate::runtimes::catalogue(st.cfg.dev_runtimes);
    if !cat.as_array().unwrap().iter().any(|x| x["id"] == runtime.as_str() && x["installable"] == true) || matches!(runtime.as_str(), "custom" | "sim") {
        return Err(ApiError::bad("runtime.unsupported", "unknown or unsupported runtime").field("runtime", "choose a supported runtime"));
    }
    let plain = |v: &str| !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
    let game = r.game_version.clone().or(d.game_version.clone()).filter(|v| plain(v)).ok_or_else(|| ApiError::bad("validation", "Enter the Minecraft version.").field("game_version", "required"))?;
    let loader = r.loader_version.clone().or(d.loader_version.clone()).filter(|v| plain(v));
    if matches!(runtime.as_str(), "forge" | "neoforge") && loader.is_none() { return Err(ApiError::bad("validation", "Enter the loader version.").field("loader_version", "required")); }
    if r.accept_eula != Some(true) { return Err(ApiError::bad("eula.required", "You must accept the Minecraft EULA to run a Minecraft server.").field("accept_eula", "required")); }
    let mem = r.memory_mb.unwrap_or(2048);
    if !(256..=262144).contains(&mem) { return Err(ApiError::bad("validation", "memory out of range").field("memory_mb", "256–262144")); }
    // every port another server uses, including a game's extra ports (Valheim takes port+1 and port+2)
    let existing: Vec<i64> = crate::server::list(&st).await?.iter().flat_map(|s| s.port..=s.port + s.port_span()).collect();
    let default_port = if runtime == "velocity" { 25577 } else { 25565 };
    let port = match r.port.or(d.port) {
        Some(p) if !(1..=65535).contains(&p) => return Err(ApiError::bad("validation", "invalid port").field("port", "1–65535")),
        Some(p) if existing.contains(&p) => return Err(ApiError::conflict("port.taken", format!("Port {p} is already used by another Moss server. Pick a different port.")).field("port", "already assigned")),
        Some(p) => p,
        None => crate::api::free_port_for(&st, default_port, &existing).await,
    };
    let java = java_for(&runtime, &game);
    let (id, now) = (uuid::Uuid::now_v7().to_string(), crate::db::now());
    let row = (id.clone(), name.clone(), runtime.clone(), game.clone(), loader.clone(), java as i64, port, mem, user.email.clone(), now.clone());
    st.db.call(move |c| c.execute("INSERT INTO servers(id,name,runtime,game_version,loader_version,java_major,port,memory_mb,eula_at,eula_by,created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.9.clone(), row.8, row.9])).await?;
    let s = crate::server::get(&st, &id).await?;
    st.audit(&user.actor(), "eula.accept", &name, "Minecraft EULA accepted at import", Some(&id)).await;
    st.audit(&user.actor(), "server.import", &name, &format!("{runtime} {game} from {}", if is_upload { "an uploaded archive".to_string() } else { src.display().to_string() }), Some(&id)).await;
    let jar = d.jar.clone();
    let op = crate::ops::spawn_op(&st, "server.import", format!("Import {name}"), Some(&id), true, {
        let (id, src, rt) = (id.clone(), src.clone(), runtime.clone());
        move |ctx| async move {
            let st = ctx.st.clone();
            let _guard = st.lock_for(&id).lock_owned().await;
            let res: Result<()> = async {
                let s = crate::server::get(&st, &id).await.map_err(|e| anyhow!(e.message))?;
                let dir = s.dir(&st);
                if dir.exists() { tokio::fs::remove_dir_all(&dir).await?; }
                if is_upload {
                    ctx.stage("Moving files into place");
                    if let Some(p) = dir.parent() { tokio::fs::create_dir_all(p).await?; }
                    tokio::fs::rename(&src, &dir).await?;
                    if let Some(base) = src.parent() { let _ = tokio::fs::remove_dir_all(base).await; }
                } else {
                    ctx.stage("Copying files (the original is not changed)");
                    let (src, dir, c2) = (src.clone(), dir.clone(), ctx.clone());
                    let skipped = tokio::task::spawn_blocking(move || -> Result<u64> {
                        let total = size_of(&src);
                        if total > MAX_BYTES { bail!("The folder is larger than the import limit."); }
                        let (mut done, mut skipped) = (0, 0);
                        copy_tree(&src, &dir, &c2, &mut done, total, &mut skipped, true)?;
                        Ok(skipped)
                    }).await??;
                    if skipped > 0 { ctx.log(format!("Skipped {skipped} symbolic link(s); Moss never follows links out of a server folder.")); }
                }
                ctx.stage("Preparing to start");
                let (d2, rt2, jar2) = (dir.clone(), rt.clone(), jar.clone());
                if let Some(n) = tokio::task::spawn_blocking(move || normalise_launcher(&d2, &rt2, jar2.as_deref())).await?? { ctx.log(n); }
                tokio::fs::write(dir.join("eula.txt"), "# Accepted via Moss by the server owner (https://aka.ms/MinecraftEULA)\neula=true\n").await?;
                let i = id.clone();
                st.db.call(move |c| c.execute("UPDATE servers SET installed=1, install_error=NULL WHERE id=?", [i])).await?;
                if crate::java::find(&st, java).is_none() { ctx.log(format!("Java {java} is not installed. Install it from Settings, then Java, before starting.")); }
                Ok(())
            }.await;
            if let Err(e) = &res {
                let i = id.clone();
                let _ = st.db.call(move |c| c.execute("DELETE FROM servers WHERE id=?", [i])).await;
                let _ = tokio::fs::remove_dir_all(st.cfg.servers_dir().join(&id)).await;
                if let Some(base) = src.parent().filter(|_| is_upload) { let _ = tokio::fs::remove_dir_all(base).await; }
                ctx.log(format!("Import failed and was cleaned up: {e}"));
            }
            res
        }
    });
    let v = crate::server::view(&s, &st);
    st.emit("server", v.clone());
    Ok((StatusCode::ACCEPTED, Json(json!({"server": v, "operation": op}))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(n: &str) -> PathBuf { let p = std::env::temp_dir().join(format!("moss-imp-{n}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&p); std::fs::create_dir_all(&p).unwrap(); p }
    fn jar(path: &Path, files: &[(&str, &str)]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, c) in files { z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap(); z.write_all(c.as_bytes()).unwrap(); }
        z.finish().unwrap();
    }

    #[test]
    fn extraction_counts_real_bytes_against_the_limit_and_drops_reserved_metadata() {
        let d = tmp("lim");
        let z = d.join("a.zip");
        jar(&z, &[("a.bin", &"x".repeat(100)), ("b.bin", &"y".repeat(100))]);
        let tok = tokio_util::sync::CancellationToken::new();
        assert!(extract_limited(&z, &d.join("ok"), &tok, 1000).is_ok());
        let e = extract_limited(&z, &d.join("big"), &tok, 150).unwrap_err().to_string();
        assert!(e.contains("size limit"), "{e}");
        let z2 = d.join("m.zip");
        jar(&z2, &[(".moss/console.fifo", "x"), ("server.properties", "a=b")]);
        extract_limited(&z2, &d.join("m"), &tok, 1000).unwrap();
        strip_reserved(&d.join("m"));
        assert!(!d.join("m/.moss").exists() && d.join("m/server.properties").exists());
    }
    #[test]
    fn names_and_versions() {
        assert_eq!(version_in_name("paper-1.21.4-123.jar").as_deref(), Some("1.21.4"));
        assert_eq!(version_in_name("purpur-1.20.jar").as_deref(), Some("1.20"));
        assert_eq!(version_in_name("server.jar"), None);
        assert_eq!(neoforge_game("21.4.12").as_deref(), Some("1.21.4"));
        assert_eq!(neoforge_game("21.0.5").as_deref(), Some("1.21"));
    }
    #[test]
    fn detects_vanilla_from_version_json_and_port() {
        let d = tmp("van");
        jar(&d.join("server.jar"), &[("version.json", r#"{"id":"1.21.4","name":"1.21.4"}"#), ("net/minecraft/bundler/Main.class", "x")]);
        std::fs::write(d.join("server.properties"), "server-port=25570\n").unwrap();
        let x = detect(&d);
        assert_eq!((x.runtime.as_deref(), x.game_version.as_deref(), x.port), (Some("vanilla"), Some("1.21.4"), Some(25570)));
    }
    #[test]
    fn detects_paper_and_purpur() {
        let d = tmp("paper");
        jar(&d.join("paper-1.21.4-100.jar"), &[("patch.properties", "version=1.21.4\n"), ("io/papermc/paperclip/Main.class", "x")]);
        std::fs::create_dir_all(d.join("config")).unwrap();
        std::fs::write(d.join("config/paper-global.yml"), "x: 1\n").unwrap();
        let x = detect(&d);
        assert_eq!((x.runtime.as_deref(), x.game_version.as_deref()), (Some("paper"), Some("1.21.4")));
        std::fs::write(d.join("purpur.yml"), "x: 1\n").unwrap();
        assert_eq!(detect(&d).runtime.as_deref(), Some("purpur"));
    }
    #[test]
    fn detects_fabric_velocity_forge_neoforge() {
        let d = tmp("fab");
        jar(&d.join("fabric-server-launch.jar"), &[("net/fabricmc/loader/Foo.class", "x")]);
        std::fs::create_dir_all(d.join(".fabric-installer")).unwrap();
        std::fs::write(d.join(".fabric-installer/launcher.json"), r#"{"id":"fabric-loader-0.16.9-1.21.4"}"#).unwrap();
        let x = detect(&d);
        assert_eq!((x.runtime.as_deref(), x.game_version.as_deref(), x.loader_version.as_deref()), (Some("fabric"), Some("1.21.4"), Some("0.16.9")));

        let v = tmp("vel");
        jar(&v.join("velocity-3.4.0-SNAPSHOT-450.jar"), &[("com/velocitypowered/proxy/Velocity.class", "x")]);
        std::fs::write(v.join("velocity.toml"), "bind = \"0.0.0.0:25599\"\n").unwrap();
        let x = detect(&v);
        assert_eq!((x.runtime.as_deref(), x.game_version.as_deref(), x.port), (Some("velocity"), Some("3.4.0"), Some(25599)));

        let n = tmp("neo");
        std::fs::create_dir_all(n.join("libraries/net/neoforged/neoforge/21.4.12")).unwrap();
        std::fs::write(n.join("libraries/net/neoforged/neoforge/21.4.12/unix_args.txt"), "x").unwrap();
        let x = detect(&n);
        assert_eq!((x.runtime.as_deref(), x.game_version.as_deref(), x.loader_version.as_deref()), (Some("neoforge"), Some("1.21.4"), Some("21.4.12")));

        let f = tmp("forge");
        std::fs::create_dir_all(f.join("libraries/net/minecraftforge/forge/1.20.1-47.3.0")).unwrap();
        std::fs::write(f.join("libraries/net/minecraftforge/forge/1.20.1-47.3.0/unix_args.txt"), "x").unwrap();
        let x = detect(&f);
        assert_eq!((x.runtime.as_deref(), x.game_version.as_deref(), x.loader_version.as_deref()), (Some("forge"), Some("1.20.1"), Some("47.3.0")));
    }
    #[test]
    fn unknown_and_spigot_need_a_choice_and_log_gives_version() {
        let d = tmp("unk");
        std::fs::write(d.join("server.properties"), "motd=x\n").unwrap();
        let x = detect(&d);
        assert!(x.runtime.is_none() && !x.notes.is_empty());
        let s = tmp("spig");
        jar(&s.join("spigot-1.20.4.jar"), &[("a", "b")]);
        std::fs::write(s.join("spigot.yml"), "x: 1\n").unwrap();
        let x = detect(&s);
        assert!(x.runtime.is_none() && x.notes[0].contains("Spigot"));
        let l = tmp("log");
        std::fs::create_dir_all(l.join("logs")).unwrap();
        std::fs::write(l.join("logs/latest.log"), "[10:00:01] [ServerMain/INFO]: Starting minecraft server version 1.19.4\n").unwrap();
        jar(&l.join("server.jar"), &[("net/minecraft/server/MinecraftServer.class", "x")]);
        assert_eq!(detect(&l).game_version.as_deref(), Some("1.19.4"));
    }
    #[test]
    fn zip_extraction_refuses_traversal_and_symlinks() {
        let d = tmp("zip");
        let zp = d.join("a.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
        z.start_file("srv/server.properties", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"x=1").unwrap();
        z.finish().unwrap();
        let tok = tokio_util::sync::CancellationToken::new();
        extract(&zp, &d.join("out"), &tok).unwrap();
        assert!(d.join("out/srv/server.properties").is_file());
        assert_eq!(effective_root(&d.join("out")), d.join("out/srv"));
        // path traversal
        let evil = d.join("evil.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
        z.start_file("../escape.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        assert!(extract(&evil, &d.join("out2"), &tok).is_err());
        assert!(!d.join("escape.txt").exists());
        // not an archive
        std::fs::write(d.join("x.bin"), b"hello world").unwrap();
        assert!(extract(&d.join("x.bin"), &d.join("out3"), &tok).is_err());
    }
}
