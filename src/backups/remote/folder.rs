//! Folder destination: a directory the administrator chose (a mounted NAS or USB disk). Objects are copied to a
//! `.part` file, flushed, and renamed into place, so a half-copied backup never looks complete.
use super::{ObjInfo, Xfer};
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::{io::{Read, Write}, path::{Path, PathBuf}};

pub const SUBDIR: &str = "moss-backups";

pub struct Folder { root: PathBuf }

/// Keys are `<server-id>/<file>`: plain components only.
pub fn safe_key(key: &str) -> Result<()> {
    if key.is_empty() || key.split('/').any(|c| c.is_empty() || c == "." || c == ".." || c.contains(['\\', '\0'])) { bail!("invalid object name"); }
    Ok(())
}

fn hash_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let (mut h, mut b) = (Sha256::new(), vec![0u8; 1 << 20]);
    loop { let n = f.read(&mut b)?; if n == 0 { break; } h.update(&b[..n]); }
    Ok(hex::encode(h.finalize()))
}

impl Folder {
    pub fn new(path: &str) -> Self { Self { root: Path::new(path).join(SUBDIR) } }
    fn path(&self, key: &str) -> Result<PathBuf> { safe_key(key)?; Ok(self.root.join(key)) }

    pub async fn put(&self, src: &Path, key: &str, x: &Xfer) -> Result<()> {
        let (dest, src, x) = (self.path(key)?, src.to_path_buf(), x.clone());
        tokio::task::spawn_blocking(move || copy(&src, &dest, &x).map(|_| ())).await?
    }
    pub async fn head(&self, key: &str) -> Result<Option<ObjInfo>> {
        let p = self.path(key)?;
        let key = key.to_string();
        tokio::task::spawn_blocking(move || match std::fs::metadata(&p) {
            Ok(m) if m.is_file() => Ok(Some(ObjInfo { key, size: m.len(), etag: None, sha256: Some(hash_file(&p)?), kms: false })),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }).await?
    }
    pub async fn get(&self, key: &str, dest: &Path, x: &Xfer) -> Result<(String, u64)> {
        let (src, dest, x) = (self.path(key)?, dest.to_path_buf(), x.clone());
        tokio::task::spawn_blocking(move || copy(&src, &dest, &x)).await?
    }
    pub async fn list(&self, prefix: &str) -> Result<Vec<ObjInfo>> {
        let root = self.root.clone();
        let prefix = prefix.to_string();
        tokio::task::spawn_blocking(move || {
            let mut out = vec![];
            let Ok(top) = std::fs::read_dir(&root) else { return Ok(out) };
            for d in top.flatten().filter(|d| d.file_type().is_ok_and(|t| t.is_dir())) {
                for f in std::fs::read_dir(d.path())?.flatten().filter(|f| f.file_type().is_ok_and(|t| t.is_file())) {
                    let key = format!("{}/{}", d.file_name().to_string_lossy(), f.file_name().to_string_lossy());
                    if key.starts_with(&prefix) && !key.ends_with(".part") { out.push(ObjInfo { key, size: f.metadata()?.len(), etag: None, sha256: None, kms: false }); }
                }
            }
            Ok(out)
        }).await?
    }
    pub async fn delete(&self, key: &str) -> Result<()> {
        match tokio::fs::remove_file(self.path(key)?).await { Ok(()) => Ok(()), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), Err(e) => Err(e.into()) }
    }
}

/// Copy `src` to `dest` through a `.part` file; returns the SHA-256 and size of what was written.
fn copy(src: &Path, dest: &Path, x: &Xfer) -> Result<(String, u64)> {
    if let Some(d) = dest.parent() { std::fs::create_dir_all(d)?; }
    let part = dest.with_file_name(format!("{}.part", dest.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
    let r: Result<(String, u64)> = (|| {
        let (mut i, mut o) = (std::fs::File::open(src)?, std::fs::File::create(&part)?);
        let (mut h, mut b, mut n) = (Sha256::new(), vec![0u8; 1 << 20], 0u64);
        loop {
            if x.token.is_cancelled() { bail!("cancelled"); }
            let k = i.read(&mut b)?;
            if k == 0 { break; }
            o.write_all(&b[..k])?;
            h.update(&b[..k]);
            n += k as u64;
            x.add(k as u64);
        }
        o.sync_all()?;
        Ok((hex::encode(h.finalize()), n))
    })();
    match r {
        Ok(v) => { std::fs::rename(&part, dest)?; Ok(v) }
        Err(e) => { let _ = std::fs::remove_file(&part); Err(e) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(n: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("moss-folder-{n}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }
    fn sha(b: &[u8]) -> String { hex::encode(Sha256::digest(b)) }

    #[test]
    fn keys_must_be_plain_components() {
        assert!(safe_key("srv/b.tar.zst").is_ok());
        for k in ["", "../x", "a/../b", "/abs", "a//b", "a/./b", "a\\b", "a/\0"] { assert!(safe_key(k).is_err(), "{k:?}"); }
    }

    #[tokio::test]
    async fn put_head_get_list_delete_round_trip() {
        let d = tmp("rt");
        let (f, src) = (Folder::new(d.to_str().unwrap()), d.join("src.bin"));
        let body: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &body).unwrap();
        let x = Xfer::new();
        f.put(&src, "s1/b1.tar.zst", &x).await.unwrap();
        assert_eq!(x.done(), body.len() as u64);
        let info = f.head("s1/b1.tar.zst").await.unwrap().unwrap();
        assert_eq!((info.size, info.sha256), (body.len() as u64, Some(sha(&body))));
        assert!(f.head("s1/nope").await.unwrap().is_none());
        let out = d.join("out.bin");
        let (h, n) = f.get("s1/b1.tar.zst", &out, &Xfer::new()).await.unwrap();
        assert_eq!((h, n, std::fs::read(&out).unwrap()), (sha(&body), body.len() as u64, body));
        f.put(&src, "s2/b2.tar.zst", &Xfer::new()).await.unwrap();
        assert_eq!(f.list("s1/").await.unwrap().len(), 1);
        assert_eq!(f.list("").await.unwrap().len(), 2);
        f.delete("s1/b1.tar.zst").await.unwrap();
        f.delete("s1/b1.tar.zst").await.unwrap();
        assert!(f.head("s1/b1.tar.zst").await.unwrap().is_none());
        assert!(f.put(&src, "../escape", &Xfer::new()).await.is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[tokio::test]
    async fn a_cancelled_copy_leaves_no_partial_object() {
        let d = tmp("cancel");
        let (f, src) = (Folder::new(d.to_str().unwrap()), d.join("src.bin"));
        std::fs::write(&src, vec![1u8; 100_000]).unwrap();
        let x = Xfer::new();
        x.token.cancel();
        assert!(f.put(&src, "s1/b.tar.zst", &x).await.is_err());
        assert!(f.head("s1/b.tar.zst").await.unwrap().is_none());
        let left: Vec<_> = std::fs::read_dir(d.join(SUBDIR).join("s1")).map(|r| r.flatten().collect()).unwrap_or_default();
        assert!(left.is_empty(), "no .part file left behind");
        let _ = std::fs::remove_dir_all(d);
    }
}
