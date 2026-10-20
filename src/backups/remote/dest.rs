//! The `backup_destinations` table: loading, saving, resolving which destination a server uses, and opening a driver.
use super::{Store, folder::Folder, s3::{S3, S3Cfg}, secrets};
use crate::state::St;
use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DestConfig {
    Folder { path: String },
    S3 { endpoint: String, region: String, bucket: String, #[serde(default)] prefix: String, #[serde(default)] path_style: bool },
}

#[derive(Clone, Debug)]
pub struct Dest { pub id: String, pub name: String, pub cfg: DestConfig, pub is_default: bool, pub keep_local: i64, pub keep_remote: i64, pub created_at: String }

impl Dest {
    pub fn kind(&self) -> &'static str { match self.cfg { DestConfig::Folder { .. } => "folder", DestConfig::S3 { .. } => "s3" } }
    /// API view. Credentials are only ever reported as set / not set.
    pub fn json(&self, st: &St) -> Value {
        let mut v = json!({"id": self.id, "name": self.name, "kind": self.kind(), "is_default": self.is_default, "keep_local": self.keep_local, "keep_remote": self.keep_remote, "created_at": self.created_at});
        match &self.cfg {
            DestConfig::Folder { path } => v["path"] = json!(path),
            DestConfig::S3 { endpoint, region, bucket, prefix, path_style } => {
                v["endpoint"] = json!(endpoint); v["region"] = json!(region); v["bucket"] = json!(bucket); v["prefix"] = json!(prefix); v["path_style"] = json!(path_style);
                v["access_key"] = json!(if secrets::is_set(&st.cfg.data_dir, &self.id) { "set" } else { "not set" });
                v["secret_key"] = v["access_key"].clone();
            }
        }
        v
    }
}

const SEL: &str = "SELECT id,name,kind,config,is_default,keep_local,keep_remote,created_at FROM backup_destinations";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Dest> {
    let cfg: DestConfig = serde_json::from_str(&r.get::<_, String>(3)?).map_err(|e| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e)))?;
    Ok(Dest { id: r.get(0)?, name: r.get(1)?, cfg, is_default: r.get(4)?, keep_local: r.get(5)?, keep_remote: r.get(6)?, created_at: r.get(7)? })
}

pub fn list_sync(c: &Connection) -> rusqlite::Result<Vec<Dest>> { c.prepare(&format!("{SEL} ORDER BY created_at"))?.query_map([], from_row)?.collect() }
pub fn get_sync(c: &Connection, id: &str) -> rusqlite::Result<Option<Dest>> { c.query_row(&format!("{SEL} WHERE id=?"), [id], from_row).optional() }

pub async fn list(st: &St) -> Result<Vec<Dest>> { st.db.call(list_sync).await }
pub async fn get(st: &St, id: &str) -> Result<Option<Dest>> { let i = id.to_string(); st.db.call(move |c| get_sync(c, &i)).await }

pub fn save_sync(c: &Connection, d: &Dest, create: bool) -> rusqlite::Result<()> {
    let cfg = serde_json::to_string(&d.cfg).unwrap_or_default();
    if d.is_default { c.execute("UPDATE backup_destinations SET is_default=0 WHERE id<>?", [&d.id])?; }
    if create {
        c.execute("INSERT INTO backup_destinations(id,name,kind,config,is_default,keep_local,keep_remote,created_at) VALUES (?,?,?,?,?,?,?,?)", params![d.id, d.name, d.kind(), cfg, d.is_default, d.keep_local, d.keep_remote, d.created_at])?;
    } else {
        c.execute("UPDATE backup_destinations SET name=?,config=?,is_default=?,keep_local=?,keep_remote=? WHERE id=?", params![d.name, cfg, d.is_default, d.keep_local, d.keep_remote, d.id])?;
    }
    Ok(())
}

/// The destination a server's backups go to: its override, else the default. `None` = backups stay local.
pub async fn effective(st: &St, server_id: &str) -> Result<Option<Dest>> {
    let sid = server_id.to_string();
    st.db.call(move |c| {
        let over: Option<String> = c.query_row("SELECT destination_id FROM backup_dest_overrides WHERE server_id=?", [&sid], |r| r.get(0)).optional()?;
        match over.as_deref() {
            Some("") => Ok(None),
            Some(id) => get_sync(c, id),
            None => c.query_row(&format!("{SEL} WHERE is_default=1"), [], from_row).optional(),
        }
    }).await
}

/// Open a driver for a destination (resolving and pinning the host for S3).
pub async fn open(st: &St, d: &Dest) -> Result<Store> {
    match &d.cfg {
        DestConfig::Folder { path } => {
            if !std::path::Path::new(path).is_dir() { return Err(anyhow!("the folder {path} is not available; is the disk mounted?")); }
            Ok(Store::Folder(Folder::new(path)))
        }
        DestConfig::S3 { endpoint, region, bucket, prefix, path_style } => {
            let c = secrets::get(&st.cfg.data_dir, &d.id).filter(|c| !c.access_key.is_empty() && !c.secret_key.is_empty()).ok_or_else(|| anyhow!("no access key and secret key are saved for this destination"))?;
            let cfg = S3Cfg { endpoint: endpoint.clone(), region: region.clone(), bucket: bucket.clone(), prefix: prefix.clone(), path_style: *path_style };
            Ok(Store::S3(Box::new(S3::connect(cfg, c.access_key, c.secret_key, st.cfg.allow_private_backup_hosts).await?)))
        }
    }
}
