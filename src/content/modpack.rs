//! Modpacks: inspect an uploaded or provider-hosted `.mrpack` / CurseForge pack (`POST /modpacks/inspect`), import it
//! into a server (`POST /servers/{id}/modpacks`, or `modpack_token` on server creation) and export a server's
//! Modrinth-sourced content (`GET /servers/{id}/modpacks/export`). Format parsers: `mrpack`, `cfpack`; the rest is
//! split into token `store`, `remote` fetching, `inspect`, `import`, `export` and archive safety in `zipsafe`.
use super::*;
use axum::extract::{FromRequest, Multipart, Request};

mod diff;
mod export;
mod import;
mod inspect;
mod remote;
mod update;
pub(super) mod store;
pub mod zipsafe;

pub use export::export;
pub use update::{apply_update, check_update, install_row, list, pin, staged_plan, update_start};
pub use store::{Identity, get as entry};
pub use zipsafe::OverrideEntry;

/// Largest pack Moss accepts, uploaded or downloaded.
pub const MAX_PACK: u64 = 512 << 20;

/// Whether a file belongs on a server.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Side { #[default] Required, Optional, Unsupported }

#[derive(Clone, Debug, Default)]
pub struct PackFile {
    pub path: String,
    pub urls: Vec<String>,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    pub size: u64,
    pub server: Side,
    /// (provider, project id, version id) when known, so installed files can be updated later.
    pub provider: Option<(String, String, String)>,
    /// CurseForge (project id, file id): downloads go through the CurseForge download path.
    pub cf: Option<(u64, u64)>,
}

#[derive(Clone, Debug)]
pub struct CfRef { pub project_id: u64, pub file_id: u64, pub required: bool }

#[derive(Clone, Debug)]
pub struct Unavailable { pub name: String, pub reason: String, pub url: Option<String> }

#[derive(Clone, Debug, Default)]
pub struct Pack {
    pub format: &'static str,
    pub name: String,
    pub version: String,
    pub summary: Option<String>,
    pub game_version: String,
    /// `forge`, `neoforge`, `fabric`, `quilt` or empty for no loader.
    pub loader: String,
    pub loader_version: Option<String>,
    pub files: Vec<PackFile>,
    pub unavailable: Vec<Unavailable>,
    pub cf_refs: Vec<CfRef>,
}

impl Pack {
    /// The Moss runtime that runs this pack.
    pub fn runtime(&self) -> &str { if self.loader.is_empty() { "vanilla" } else { &self.loader } }
}

/// A parsed archive: the pack, its overrides and the protected paths that will be skipped.
#[derive(Debug)]
pub struct Scanned { pub pack: Pack, pub overrides: Vec<OverrideEntry>, pub skipped: Vec<String> }

/// Only administrators and per-server managers may stage packs (they can create servers or manage content).
pub(super) async fn may_stage(st: &St, user: &User) -> ApiResult<()> {
    if user.is_admin() { return Ok(()); }
    let uid = user.id.clone();
    let ok: bool = st.db.call(move |c| c.query_row("SELECT EXISTS(SELECT 1 FROM grants WHERE user_id=? AND bundle='manager')", [uid], |r| r.get(0))).await?;
    if ok { Ok(()) } else { Err(ApiError::forbidden("managing content on a server is required to import modpacks")) }
}

pub async fn inspect(State(st): State<St>, user: User, req: Request) -> ApiResult<Json<Value>> {
    may_stage(&st, &user).await?;
    let multipart = req.headers().get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("multipart/form-data"));
    let (path, size, source) = if multipart {
        let mp = Multipart::from_request(req, &st).await.map_err(|e| ApiError::bad("upload.invalid", e.to_string()))?;
        let (p, s) = remote::receive_upload(&st, mp).await?;
        (p, s, None)
    } else {
        let Json(body): Json<Value> = Json::from_request(req, &st).await.map_err(|e| ApiError::bad("validation", e.to_string()))?;
        let (p, s, id) = remote::fetch_remote(&st, &user, &body).await?;
        (p, s, Some(id))
    };
    match inspect::run(&st, &user, &path, size, source).await {
        Ok(v) => Ok(Json(v)),
        Err(e) => { let _ = tokio::fs::remove_file(&path).await; Err(e) }
    }
}

#[derive(Deserialize)]
pub struct ImportReq { token: String, mode: String }

pub async fn import_start(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<ImportReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    if !matches!(r.mode.as_str(), "add" | "replace") { return Err(ApiError::bad("validation", "mode must be add or replace").field("mode", "add | replace")); }
    let entry = store::get(&r.token, &user.id).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "modpack.token_expired", "this modpack token is unknown or has expired (tokens last 30 minutes); inspect the pack again"))?;
    if entry.runtime != s.runtime { return Err(ApiError::conflict("modpack.runtime_mismatch", format!("this pack needs {} but the server runs {}", entry.runtime, s.runtime))); }
    if r.mode == "replace" && (lifecycle_running(&st, &s) || crate::lifecycle::process_alive(&s)) { return Err(ApiError::conflict("server.running", "stop the server before replacing its content")); }
    let (title, actor, utoken) = (format!("Import {} into {}", entry.name, s.name), user.actor(), user.token.clone());
    let op = ops::spawn_op(&st, "modpack.import", title, Some(&id), true, move |ctx| import::import(ctx, s, c, entry, r.mode, utoken, actor));
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

/// `POST /servers` with a `modpack_token`: the runtime, game version and loader come from the pack. Values the
/// client also sent must agree. Returns (runtime, game_version, loader_version).
pub fn create_params(user: &User, token: &str, runtime: &str, game: &str, loader: Option<&str>) -> ApiResult<(String, String, Option<String>)> {
    let e = store::get(token, &user.id).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "modpack.token_expired", "this modpack token is unknown or has expired (tokens last 30 minutes); inspect the pack again"))?;
    if !runtime.is_empty() && runtime != e.runtime { return Err(ApiError::bad("runtime.unsupported", format!("this pack needs the {} runtime", e.runtime)).field("runtime", "must match the pack or be omitted")); }
    if !game.is_empty() && game != e.game_version { return Err(ApiError::bad("validation", format!("this pack is for Minecraft {}", e.game_version)).field("game_version", "must match the pack or be omitted")); }
    if let (Some(l), Some(p)) = (loader, e.loader_version.as_deref()) && l != p { return Err(ApiError::bad("validation", format!("this pack needs loader version {p}")).field("loader_version", "must match the pack or be omitted")); }
    Ok((e.runtime, e.game_version, e.loader_version))
}

/// The operation behind `POST /servers`: the normal install, then (when a pack token was given) the pack import.
pub async fn install_then_import(ctx: OpCtx, id: String, token: Option<String>, user: User) -> Result<()> {
    crate::installer::install(ctx.clone(), id.clone()).await?;
    let Some(token) = token else { return Ok(()) };
    let st = ctx.st.clone();
    let s = crate::server::get(&st, &id).await.map_err(|e| anyhow!(e.message))?;
    let c = compat(&st, &s).ok_or_else(|| anyhow!("this runtime has no managed content"))?;
    let entry = store::get(&token, &user.id).ok_or_else(|| anyhow!("the modpack token expired before the server finished installing"))?;
    import::import(ctx, s, c, entry, "add".into(), user.token.clone(), user.email.clone()).await
}
