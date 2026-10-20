//! Backup destinations API (administrators only, except reading a server's own setting): add, edit, test and remove
//! destinations, and choose the destination a server uses. Credentials are write-only; responses say "set" or "not set".
use super::*;
use crate::backups::remote::{self, Dest, dest::{self, DestConfig}, probe, secrets, validate};

#[derive(Deserialize, Default)]
pub(super) struct DestBody {
    name: Option<String>, kind: Option<String>, path: Option<String>,
    endpoint: Option<String>, region: Option<String>, bucket: Option<String>, prefix: Option<String>, path_style: Option<bool>,
    access_key: Option<String>, secret_key: Option<String>,
    is_default: Option<bool>, keep_local: Option<i64>, keep_remote: Option<i64>,
}

/// Build and validate the configuration from a request body, falling back to `cur` for omitted fields.
fn config_from(st: &St, b: &DestBody, cur: Option<&DestConfig>) -> ApiResult<DestConfig> {
    let kind = match (&b.kind, cur) {
        (_, Some(DestConfig::Folder { .. })) => "folder",
        (_, Some(DestConfig::S3 { .. })) => "s3",
        (Some(k), None) if k == "folder" || k == "s3" => k.as_str(),
        _ => return Err(ApiError::bad("validation", "kind must be folder or s3").field("kind", "folder | s3")),
    };
    let raw = if kind == "folder" {
        let path = b.path.clone().or_else(|| match cur { Some(DestConfig::Folder { path }) => Some(path.clone()), _ => None }).unwrap_or_default();
        DestConfig::Folder { path }
    } else {
        let (ce, cr, cb, cp, cs) = match cur { Some(DestConfig::S3 { endpoint, region, bucket, prefix, path_style }) => (endpoint.clone(), region.clone(), bucket.clone(), prefix.clone(), *path_style), _ => Default::default() };
        DestConfig::S3 { endpoint: b.endpoint.clone().unwrap_or(ce), region: b.region.clone().unwrap_or(cr), bucket: b.bucket.clone().unwrap_or(cb), prefix: b.prefix.clone().unwrap_or(cp), path_style: b.path_style.unwrap_or(cs) }
    };
    Ok(validate::config_of(&raw, &st.cfg.data_dir, st.cfg.allow_private_backup_hosts)?)
}

async fn view(st: &St, id: &str) -> ApiResult<Value> {
    let d = dest::get(st, id).await?.ok_or_else(|| ApiError::not_found("destination"))?;
    Ok(d.json(st))
}

pub(super) async fn dest_list(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let list = dest::list(&st).await?;
    let counts: HashMap<String, i64> = st.db.call(|c| c.prepare("SELECT remote_dest, COUNT(*) FROM backups WHERE remote='stored' AND remote_dest IS NOT NULL GROUP BY remote_dest")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()).await?;
    Ok(Json(Value::Array(list.iter().map(|d| { let mut v = d.json(&st); v["stored_backups"] = json!(counts.get(&d.id).copied().unwrap_or(0)); v }).collect())))
}

pub(super) async fn dest_create(State(st): State<St>, user: User, Json(b): Json<DestBody>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let name = validate::name(b.name.as_deref().unwrap_or(""))?;
    let cfg = config_from(&st, &b, None)?;
    let creds = match &cfg {
        DestConfig::S3 { .. } => Some(secrets::Creds { access_key: validate::credential("access_key", b.access_key.as_deref().unwrap_or(""))?, secret_key: validate::credential("secret_key", b.secret_key.as_deref().unwrap_or(""))? }),
        DestConfig::Folder { .. } => None,
    };
    let (keep_local, keep_remote) = (validate::keep("keep_local", b.keep_local.unwrap_or(0), 0)?, validate::keep("keep_remote", b.keep_remote.unwrap_or(14), 1)?);
    let first = dest::list(&st).await?.is_empty();
    let d = Dest { id: uuid::Uuid::now_v7().to_string(), name, cfg, is_default: b.is_default.unwrap_or(first), keep_local, keep_remote, created_at: crate::db::now() };
    if let Some(c) = creds { secrets::set(&st.cfg.data_dir, &d.id, c)?; }
    let d2 = d.clone();
    if let Err(e) = st.db.call(move |c| dest::save_sync(c, &d2, true)).await { let _ = secrets::remove(&st.cfg.data_dir, &d.id); return Err(e.into()); }
    st.audit(&user.actor(), "backup.destination.add", &d.name, d.kind(), None).await;
    Ok((StatusCode::CREATED, Json(d.json(&st))))
}

pub(super) async fn dest_patch(State(st): State<St>, user: User, Path(id): Path<String>, Json(b): Json<DestBody>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let mut d = dest::get(&st, &id).await?.ok_or_else(|| ApiError::not_found("destination"))?;
    if let Some(n) = &b.name { d.name = validate::name(n)?; }
    d.cfg = config_from(&st, &b, Some(&d.cfg))?;
    if let Some(n) = b.keep_local { d.keep_local = validate::keep("keep_local", n, 0)?; }
    if let Some(n) = b.keep_remote { d.keep_remote = validate::keep("keep_remote", n, 1)?; }
    if let Some(v) = b.is_default { d.is_default = v; }
    if matches!(d.cfg, DestConfig::S3 { .. }) && (b.access_key.is_some() || b.secret_key.is_some()) {
        let cur = secrets::get(&st.cfg.data_dir, &id).unwrap_or_default();
        let c = secrets::Creds {
            access_key: match &b.access_key { Some(a) => validate::credential("access_key", a)?, None => cur.access_key },
            secret_key: match &b.secret_key { Some(a) => validate::credential("secret_key", a)?, None => cur.secret_key },
        };
        secrets::set(&st.cfg.data_dir, &id, c)?;
    }
    let d2 = d.clone();
    st.db.call(move |c| dest::save_sync(c, &d2, false)).await?;
    st.audit(&user.actor(), "backup.destination.update", &d.name, "", None).await;
    Ok(Json(view(&st, &id).await?))
}

#[derive(Deserialize, Default)] pub(super) struct DelQ { force: Option<bool> }
pub(super) async fn dest_delete(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Query<DelQ>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    let d = dest::get(&st, &id).await?.ok_or_else(|| ApiError::not_found("destination"))?;
    let i = id.clone();
    let stored: i64 = st.db.call(move |c| c.query_row("SELECT COUNT(*) FROM backups WHERE remote='stored' AND remote_dest=?", [i], |r| r.get(0))).await?;
    if stored > 0 && q.force != Some(true) {
        return Err(ApiError::conflict("destination.in_use", format!("{stored} backup(s) are stored on {}. Removing it keeps the files there but Moss forgets them (backups that exist only there disappear from the list). Confirm to continue.", d.name)));
    }
    let i = id.clone();
    st.db.call(move |c| {
        c.execute("DELETE FROM backups WHERE remote='stored' AND remote_dest=?1 AND local_present=0", [&i])?;
        c.execute("UPDATE backups SET remote='none', remote_dest=NULL, remote_key=NULL, remote_error=NULL, remote_at=NULL WHERE remote_dest=?1", [&i])?;
        c.execute("DELETE FROM backup_dest_overrides WHERE destination_id=?1", [&i])?;
        c.execute("DELETE FROM backup_destinations WHERE id=?1", [&i])
    }).await?;
    secrets::remove(&st.cfg.data_dir, &id)?;
    st.audit(&user.actor(), "backup.destination.remove", &d.name, "", None).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn run_probe(st: &St, store: anyhow::Result<remote::Store>) -> Json<Value> {
    let (ok, steps) = match store {
        Err(e) => (false, vec![json!({"step": "Connect", "ok": false, "detail": format!("{e:#}")})]),
        Ok(s) => match tokio::time::timeout(Duration::from_secs(90), probe::run(&s, &st.cfg.tmp_dir())).await {
            Ok(r) => r,
            Err(_) => (false, vec![json!({"step": "Test", "ok": false, "detail": "the destination did not answer within 90 seconds"})]),
        },
    };
    Json(json!({"ok": ok, "steps": steps}))
}

/// `POST /backup-destinations/{id}/test`: write, read back, list and delete a small file.
pub(super) async fn dest_test(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let d = dest::get(&st, &id).await?.ok_or_else(|| ApiError::not_found("destination"))?;
    let r = run_probe(&st, dest::open(&st, &d).await).await;
    st.audit(&user.actor(), "backup.destination.test", &d.name, if r.0["ok"] == true { "ok" } else { "failed" }, None).await;
    Ok(r)
}

/// `POST /backup-destinations/test`: the same test for settings that are not saved yet.
pub(super) async fn dest_test_new(State(st): State<St>, user: User, Json(b): Json<DestBody>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let cfg = config_from(&st, &b, None)?;
    let creds = match &cfg {
        DestConfig::S3 { .. } => Some(secrets::Creds { access_key: validate::credential("access_key", b.access_key.as_deref().unwrap_or(""))?, secret_key: validate::credential("secret_key", b.secret_key.as_deref().unwrap_or(""))? }),
        DestConfig::Folder { .. } => None,
    };
    Ok(run_probe(&st, dest::connect(&st, &cfg, creds).await).await)
}

async fn server_setting(st: &St, sid: &str) -> ApiResult<Value> {
    let s = sid.to_string();
    let over: Option<String> = st.db.call(move |c| c.query_row("SELECT destination_id FROM backup_dest_overrides WHERE server_id=?", [s], |r| r.get(0)).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await?;
    let eff = dest::effective(st, sid).await?;
    Ok(json!({"destination_id": match over.as_deref() { None => json!("default"), Some("") => json!("none"), Some(id) => json!(id) },
        "effective": eff.map(|d| json!({"id": d.id, "name": d.name, "kind": d.kind()}))}))
}

/// `GET /servers/{id}/backup-destination`
pub(super) async fn server_dest_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    server_for(&st, &user, &id, "backup.manage").await?;
    Ok(Json(server_setting(&st, &id).await?))
}

#[derive(Deserialize)] pub(super) struct SrvDest { destination_id: String }
/// `PUT /servers/{id}/backup-destination {destination_id: "default" | "none" | <destination id>}`
pub(super) async fn server_dest_put(State(st): State<St>, user: User, Path(id): Path<String>, Json(b): Json<SrvDest>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let s = server_for(&st, &user, &id, "backup.manage").await?;
    match b.destination_id.as_str() {
        "default" => { let i = id.clone(); st.db.call(move |c| c.execute("DELETE FROM backup_dest_overrides WHERE server_id=?", [i])).await?; }
        v => {
            let target = if v == "none" { String::new() } else {
                dest::get(&st, v).await?.ok_or_else(|| ApiError::bad("validation", "unknown destination").field("destination_id", "not found"))?.id
            };
            let i = id.clone();
            st.db.call(move |c| c.execute("INSERT INTO backup_dest_overrides(server_id,destination_id) VALUES (?,?) ON CONFLICT(server_id) DO UPDATE SET destination_id=excluded.destination_id", [i, target])).await?;
        }
    }
    st.audit(&user.actor(), "backup.destination.assign", &s.name, &b.destination_id, Some(&s.id)).await;
    Ok(Json(server_setting(&st, &id).await?))
}
