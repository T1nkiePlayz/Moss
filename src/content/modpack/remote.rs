//! Getting a pack file into `tmp/`: a streamed multipart upload, or a download of a Modrinth / CurseForge pack
//! version. Both end with a temp file named `modpack-<uuid>.pack` that the caller owns.
use super::*;

fn tmp_pack(st: &St) -> std::path::PathBuf { st.cfg.tmp_dir().join(format!("modpack-{}.pack", uuid::Uuid::now_v7())) }

/// Stream the `file` field of a multipart body to disk, refusing more than `MAX_PACK` bytes.
pub(super) async fn receive_upload(st: &St, mut mp: Multipart) -> ApiResult<(std::path::PathBuf, u64)> {
    use tokio::io::AsyncWriteExt;
    tokio::fs::create_dir_all(st.cfg.tmp_dir()).await?;
    while let Some(mut f) = mp.next_field().await.map_err(|e| ApiError::bad("upload.invalid", e.to_string()))? {
        if f.name() != Some("file") { continue; }
        let path = tmp_pack(st);
        let mut out = tokio::fs::File::create(&path).await?;
        let mut size = 0u64;
        let res: ApiResult<()> = async {
            while let Some(chunk) = f.chunk().await.map_err(|e| ApiError::bad("upload.invalid", e.to_string()))? {
                size += chunk.len() as u64;
                if size > MAX_PACK { return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "upload.too_large", "modpacks can be at most 512 MiB")); }
                out.write_all(&chunk).await?;
            }
            out.flush().await?;
            Ok(())
        }.await;
        drop(out);
        if let Err(e) = res { let _ = tokio::fs::remove_file(&path).await; return Err(e); }
        return Ok((path, size));
    }
    Err(ApiError::bad("upload.invalid", "send the pack in a multipart field named \"file\"").field("file", "required"))
}

fn too_big(size: Option<u64>) -> bool { size.is_some_and(|s| s > MAX_PACK) }

/// `{provider, project_id, version_id?}` -> download that pack version into `tmp/`.
pub(super) async fn fetch_remote(st: &St, user: &User, body: &Value) -> ApiResult<(std::path::PathBuf, u64)> {
    let prov = Prov::parse(body["provider"].as_str())?;
    let pid = body["project_id"].as_str().map(String::from).or_else(|| body["project_id"].as_u64().map(|n| n.to_string())).unwrap_or_default();
    let vid = body["version_id"].as_str().map(String::from).or_else(|| body["version_id"].as_u64().map(|n| n.to_string())).filter(|v| !v.is_empty());
    if !valid_id(&pid) || vid.as_deref().is_some_and(|v| !valid_id(v)) { return Err(ApiError::bad("validation", "invalid project or version id").field("project_id", "letters, digits, - and _")); }
    tokio::fs::create_dir_all(st.cfg.tmp_dir()).await?;
    let path = tmp_pack(st);
    let r = match prov { Prov::Modrinth => modrinth_pack(st, &pid, vid.as_deref(), &path).await, Prov::Curseforge => curseforge_pack(st, user, &pid, vid.as_deref(), &path).await };
    match r {
        Ok(size) => Ok((path, size)),
        Err(e) => { let _ = tokio::fs::remove_file(&path).await; Err(match e.downcast_ref::<cf::CfError>() { Some(_) => upstream(e), None => ApiError::bad("modpack.fetch_failed", format!("{e:#}")) }) }
    }
}

async fn modrinth_pack(st: &St, pid: &str, vid: Option<&str>, dest: &std::path::Path) -> Result<u64> {
    let v = match vid {
        Some(v) => mr(st, &format!("/version/{v}"), &[]).await?,
        None => {
            let vs = mr(st, &format!("/project/{pid}/version"), &[]).await?;
            let vs = vs.as_array().cloned().unwrap_or_default();
            pick_latest(&vs).cloned().ok_or_else(|| anyhow!("this project has no versions"))?
        }
    };
    let files = v["files"].as_array().cloned().unwrap_or_default();
    let f = files.iter().find(|f| f["filename"].as_str().is_some_and(|n| n.ends_with(".mrpack"))).ok_or_else(|| anyhow!("this version has no .mrpack file"))?;
    if too_big(f["size"].as_u64()) { bail!("the pack is larger than 512 MiB"); }
    let hash = f["hashes"]["sha512"].as_str().map(|h| Hash::Sha512(h.into())).or_else(|| f["hashes"]["sha1"].as_str().map(|h| Hash::Sha1(h.into())));
    download::fetch(&st.http, f["url"].as_str().unwrap_or_default(), dest, hash, &tokio_util::sync::CancellationToken::new(), |_| {}).await?;
    Ok(tokio::fs::metadata(dest).await?.len())
}

async fn curseforge_pack(st: &St, user: &User, pid: &str, vid: Option<&str>, dest: &std::path::Path) -> Result<u64> {
    let token = user.token.as_deref();
    if let (false, Some(why)) = cf::availability(st, token) { bail!("{why}"); }
    let f = match vid {
        Some(v) => cf::get(st, token, &format!("/v1/mods/{pid}/files/{v}"), &[]).await?["data"].clone(),
        None => {
            let m = cf::get(st, token, &format!("/v1/mods/{pid}"), &[]).await?["data"].clone();
            if !cf::distributable(&m) { bail!("the author disabled third-party downloads for this pack"); }
            let files: Vec<Value> = m["latestFiles"].as_array().cloned().unwrap_or_default();
            cf::pick_latest(&files).cloned().ok_or_else(|| anyhow!("no downloadable file found for this pack"))?
        }
    };
    if !cf::downloadable(&f) { bail!("the author disabled automatic downloads for this file; download it from {} and upload it here", cf::project_url(pid.parse().unwrap_or(0))); }
    if too_big(f["fileLength"].as_u64()) { bail!("the pack is larger than 512 MiB"); }
    let fid = f["id"].as_u64().unwrap_or_default().to_string();
    let sha1 = f["hashes"].as_array().into_iter().flatten().find(|h| h["algo"] == 1).and_then(|h| h["value"].as_str()).map(|h| Hash::Sha1(h.into()));
    let (u, headers, extra_host) = cf::download_target(st, token, pid, &fid, f["downloadUrl"].as_str().unwrap_or_default())?;
    let _permit = st.cf_sem.acquire().await?;
    download::fetch_ext(cf::dl_client(), &u, dest, sha1, &tokio_util::sync::CancellationToken::new(), |_| {}, download::Opts { headers, extra_host }).await?;
    Ok(tokio::fs::metadata(dest).await?.len())
}
