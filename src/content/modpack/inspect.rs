//! `POST /modpacks/inspect`: parse a staged pack file (format detected from its contents, never its name), work out
//! what an import would do, and hand back a token for it.
use super::*;
use std::path::{Path as FsPath, PathBuf};

const FORMATS: &str = "this is neither a Modrinth .mrpack nor a CurseForge modpack .zip";

/// Parse the archive on a blocking thread.
pub(super) async fn scan(path: PathBuf) -> Result<Scanned> {
    tokio::task::spawn_blocking(move || -> Result<Scanned> {
        let mut z = zip::ZipArchive::new(std::fs::File::open(&path)?).context("this is not a valid zip archive")?;
        if z.by_name(mrpack::INDEX).is_ok() { mrpack::parse(&mut z) } else if z.by_name("manifest.json").is_ok() { cfpack::parse(&mut z) } else { bail!(FORMATS) }
    }).await?
}

/// The inspect response body (without the token). Counts: `server` = files an import will install,
/// `client_only` = files the pack marks as not for servers, `unavailable` = files that cannot be fetched automatically.
pub(super) fn info(token: &str, p: &Pack, files: &[PackFile], unavailable: &[Unavailable], overrides: (usize, u64), size: u64, warnings: &[String]) -> Value {
    let client_only = files.iter().filter(|f| f.server == Side::Unsupported).count();
    json!({
        "token": token, "format": p.format, "name": p.name, "version": p.version, "summary": p.summary, "game_version": p.game_version,
        "loader": {"id": p.runtime(), "version": p.loader_version},
        "files": {"total": files.len() + unavailable.len(), "server": files.len() - client_only, "client_only": client_only, "unavailable": unavailable.len()},
        "unavailable": unavailable.iter().map(|u| json!({"name": u.name, "reason": u.reason, "url": u.url})).collect::<Vec<_>>(),
        "overrides": {"files": overrides.0, "size": overrides.1}, "size": size, "warnings": warnings,
    })
}

pub(super) async fn run(st: &St, user: &User, path: &FsPath, size: u64) -> ApiResult<Value> {
    let sc = scan(path.to_path_buf()).await.map_err(|e| ApiError::bad("modpack.invalid", format!("{e:#}")))?;
    let mut warnings: Vec<String> = vec![];
    let (files, unavailable) = if sc.pack.format == "curseforge" {
        if cf::availability(st, user.token.as_deref()).0 {
            cfpack::resolve(st, user.token.as_deref(), &sc.pack.cf_refs).await.map_err(upstream)?
        } else {
            warnings.push("CurseForge is not available on this Moss instance, so the files could not be checked; importing this pack will fail until it is.".into());
            (vec![], vec![])
        }
    } else { (sc.pack.files.clone(), sc.pack.unavailable.clone()) };
    if !unavailable.is_empty() { warnings.push(format!("{} file(s) cannot be downloaded automatically and will be skipped.", unavailable.len())); }
    if files.iter().any(|f| f.server == Side::Optional) { warnings.push("Optional files are installed too.".into()); }
    if !sc.skipped.is_empty() { warnings.push(format!("{} file(s) that would replace server software are ignored ({}).", sc.skipped.len(), sc.skipped.iter().take(3).cloned().collect::<Vec<_>>().join(", "))); }
    if sc.pack.format == "curseforge" && sc.pack.cf_refs.len() > files.len() + unavailable.len() { warnings.push("Some referenced files were not returned by CurseForge.".into()); }
    let stats = (sc.overrides.len(), sc.overrides.iter().map(|o| o.size).sum::<u64>());
    let token = store::put(st, store::Entry { token: String::new(), path: path.to_path_buf(), user: user.id.clone(), created: std::time::Instant::now(), runtime: sc.pack.runtime().to_string(),
        game_version: sc.pack.game_version.clone(), loader_version: sc.pack.loader_version.clone(), name: sc.pack.name.clone() });
    Ok(info(&token, &sc.pack, &files, &unavailable, stats, size, &warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zipsafe::tests::build;

    #[tokio::test]
    async fn detects_formats_from_content_and_reports_counts() {
        let idx = serde_json::to_vec(&json!({"formatVersion": 1, "name": "P", "versionId": "1", "dependencies": {"minecraft": "1.21", "fabric-loader": "0.16"},
            "files": [{"path": "mods/a.jar", "hashes": {"sha1": "aa"}, "downloads": ["https://cdn.modrinth.com/data/a/versions/b/a.jar"], "fileSize": 1},
                      {"path": "mods/c.jar", "hashes": {"sha1": "aa"}, "env": {"server": "unsupported"}, "downloads": ["https://cdn.modrinth.com/data/c/versions/d/c.jar"], "fileSize": 1},
                      {"path": "mods/x.jar", "hashes": {"sha1": "aa"}, "downloads": ["https://example.org/x.jar"], "fileSize": 1}]})).unwrap();
        let dir = std::env::temp_dir().join(format!("moss-insp-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_bytes = |entries: &[(&str, &[u8])]| { let z = build(entries); z.into_inner().into_inner() };
        let mr = dir.join("a.zip");
        std::fs::write(&mr, zip_bytes(&[(mrpack::INDEX, &idx), ("overrides/config/a.toml", b"abc")])).unwrap();
        let sc = scan(mr).await.unwrap();
        let v = info("tok", &sc.pack, &sc.pack.files, &sc.pack.unavailable, (sc.overrides.len(), 3), 99, &[]);
        assert_eq!(v["format"], "mrpack");
        assert_eq!(v["loader"], json!({"id": "fabric", "version": "0.16"}));
        assert_eq!(v["files"], json!({"total": 3, "server": 1, "client_only": 1, "unavailable": 1}));
        assert_eq!(v["overrides"], json!({"files": 1, "size": 3}));
        assert_eq!(v["unavailable"][0]["name"], "mods/x.jar");
        let bad = dir.join("b.mrpack");
        std::fs::write(&bad, zip_bytes(&[("readme.txt", b"hi")])).unwrap();
        assert!(scan(bad).await.unwrap_err().to_string().contains("neither"));
        let notzip = dir.join("c.mrpack");
        std::fs::write(&notzip, b"not a zip").unwrap();
        assert!(scan(notzip).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
