//! Archive format: tar.zst writer with embedded manifest and SHA-256, verification and safe extraction
//! (no traversal, links or devices), plus directory sizing. Pure blocking code; callers use `spawn_blocking`.
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::{io::{Read, Write}, path::{Path as FsPath, PathBuf}};

pub const EXCLUDE_TOP: &[&str] = &[".moss", "logs", "cache"];
pub const MANIFEST: &str = ".moss-manifest.json";

struct HashW<W: Write> { inner: W, h: Sha256, n: u64 }
impl<W: Write> Write for HashW<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { let n = self.inner.write(b)?; self.h.update(&b[..n]); self.n += n as u64; Ok(n) }
    fn flush(&mut self) -> std::io::Result<()> { self.inner.flush() }
}

/// Reads exactly `len` bytes, zero-padding if the file shrinks while being archived (tar needs a fixed length).
struct Fixed { f: std::io::Take<std::fs::File>, left: u64 }
impl Read for Fixed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 { return Ok(0); }
        let want = buf.len().min(self.left as usize);
        let n = self.f.read(&mut buf[..want])?;
        let n = if n == 0 { buf[..want].fill(0); want } else { n };
        self.left -= n as u64;
        Ok(n)
    }
}

pub fn dir_size(root: &FsPath) -> u64 {
    fn walk(p: &FsPath, top: bool) -> u64 {
        let Ok(rd) = std::fs::read_dir(p) else { return 0 };
        rd.flatten().map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if top && EXCLUDE_TOP.contains(&name.as_str()) { return 0; }
            match e.file_type() { Ok(t) if t.is_dir() => walk(&e.path(), false), Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0), _ => 0 }
        }).sum()
    }
    walk(root, true)
}

pub fn build_archive(root: PathBuf, out: PathBuf, manifest: String, token: tokio_util::sync::CancellationToken) -> Result<(String, u64, u64)> {
    let f = std::fs::File::create(&out)?;
    let hw = HashW { inner: std::io::BufWriter::with_capacity(1 << 20, f), h: Sha256::new(), n: 0 };
    let mut enc = zstd::Encoder::new(hw, 3)?;
    enc.multithread(2).ok();
    let mut tar = tar::Builder::new(enc);
    tar.follow_symlinks(false);
    let mut h = tar::Header::new_gnu();
    h.set_size(manifest.len() as u64); h.set_mode(0o644); h.set_entry_type(tar::EntryType::Regular); h.set_cksum();
    tar.append_data(&mut h, MANIFEST, manifest.as_bytes())?;
    let mut files = 0u64;
    let mut stack = vec![(root.clone(), true)];
    while let Some((dir, top)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            if token.is_cancelled() { bail!("cancelled"); }
            let name = e.file_name().to_string_lossy().to_string();
            if top && EXCLUDE_TOP.contains(&name.as_str()) { continue; }
            let Ok(t) = e.file_type() else { continue };
            let rel = e.path().strip_prefix(&root)?.to_path_buf();
            if t.is_dir() {
                tar.append_dir(&rel, e.path()).ok();
                stack.push((e.path(), false));
            } else if t.is_file() {
                let Ok(file) = std::fs::File::open(e.path()) else { continue };
                let Ok(m) = file.metadata() else { continue };
                let mut hd = tar::Header::new_gnu();
                hd.set_metadata(&m);
                hd.set_size(m.len());
                hd.set_cksum();
                tar.append_data(&mut hd, &rel, Fixed { f: file.take(m.len()), left: m.len() })?;
                files += 1;
            } // symlinks, sockets and devices are deliberately not archived
        }
    }
    let enc = tar.into_inner()?;
    let mut hw = enc.finish()?;
    hw.flush()?;
    Ok((hex::encode(hw.h.finalize()), hw.n, files))
}

pub fn safe_entry_path(p: &FsPath) -> bool { p.components().all(|c| matches!(c, std::path::Component::Normal(_))) && p.components().count() > 0 }

/// Top-level directories (not excluded ones) that contain a `level.dat`: the worlds of a server directory.
pub fn worlds_of(root: &FsPath) -> Vec<String> {
    let mut w: Vec<String> = std::fs::read_dir(root).into_iter().flatten().flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.path().join("level.dat").is_file())
        .map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| !EXCLUDE_TOP.contains(&n.as_str())).collect();
    w.sort();
    w
}

/// First path component of an archive entry.
pub fn top_name(p: &FsPath) -> Option<String> { p.components().next().map(|c| c.as_os_str().to_string_lossy().to_string()) }

pub fn verify_archive(path: &FsPath, want: &str) -> Result<u64> { verify_archive_scan(path, want).map(|(n, _)| n) }

/// Verify checksum and entry safety; also returns the top-level names present in the archive.
pub fn verify_archive_scan(path: &FsPath, want: &str) -> Result<(u64, std::collections::BTreeSet<String>)> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop { let n = f.read(&mut buf)?; if n == 0 { break; } h.update(&buf[..n]); }
    let got = hex::encode(h.finalize());
    if !got.eq_ignore_ascii_case(want) { bail!("checksum mismatch: archive is corrupt (expected {}…, got {}…)", &want[..12.min(want.len())], &got[..12]); }
    let mut ar = tar::Archive::new(zstd::Decoder::new(std::fs::File::open(path)?)?);
    let (mut entries, mut names) = (0, std::collections::BTreeSet::new());
    for e in ar.entries()? {
        let e = e?;
        let p = e.path()?.into_owned();
        if !safe_entry_path(&p) { bail!("archive contains an unsafe path: {}", p.display()); }
        if !matches!(e.header().entry_type(), tar::EntryType::Regular | tar::EntryType::Directory) { bail!("archive contains a non-regular entry: {}", p.display()); }
        if p != FsPath::new(MANIFEST) && let Some(t) = top_name(&p) { names.insert(t); }
        entries += 1;
    }
    Ok((entries, names))
}

/// Extract into `dest`; with `only`, just those top-level entries.
pub fn extract_archive(path: &FsPath, dest: &FsPath, token: &tokio_util::sync::CancellationToken, only: Option<&[String]>) -> Result<()> { extract_archive_limited(path, dest, token, only, 2u64 << 40) }

/// Like `extract_archive` with an explicit cap on the bytes the archive may expand to (uploads use a much lower one).
pub fn extract_archive_limited(path: &FsPath, dest: &FsPath, token: &tokio_util::sync::CancellationToken, only: Option<&[String]>, max_bytes: u64) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut ar = tar::Archive::new(zstd::Decoder::new(std::fs::File::open(path)?)?);
    ar.set_preserve_permissions(false);
    ar.set_unpack_xattrs(false);
    let mut total = 0u64;
    for e in ar.entries()? {
        if token.is_cancelled() { bail!("cancelled"); }
        let mut e = e?;
        let p = e.path()?.into_owned();
        if p == FsPath::new(MANIFEST) { continue; }
        // Moss never archives its own metadata, so an archive that carries `.moss` was not made by Moss: never restore it
        if top_name(&p).is_some_and(|t| t.eq_ignore_ascii_case(".moss")) { continue; }
        if let Some(only) = only && !top_name(&p).is_some_and(|t| only.contains(&t)) { continue; }
        if !safe_entry_path(&p) || !matches!(e.header().entry_type(), tar::EntryType::Regular | tar::EntryType::Directory) { bail!("refusing unsafe archive entry {}", p.display()); }
        total += e.size();
        if total > max_bytes { bail!("archive expands beyond the safety limit"); }
        if !e.unpack_in(dest)? { bail!("entry {} would escape the destination", p.display()); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_paths_must_be_plain_relative() {
        assert!(safe_entry_path(FsPath::new("world/level.dat")));
        for bad in ["../x", "/etc/passwd", "a/../b", ""] { assert!(!safe_entry_path(FsPath::new(bad)), "{bad}"); }
    }

    #[test]
    fn archive_round_trip_verifies_and_extracts() {
        let base = std::env::temp_dir().join(format!("moss-bk-{}", std::process::id()));
        let (src, out, dst) = (base.join("src"), base.join("a.tar.zst"), base.join("dst"));
        std::fs::create_dir_all(src.join("world")).unwrap();
        std::fs::create_dir_all(src.join(".moss")).unwrap();
        std::fs::write(src.join("world/level.dat"), b"data").unwrap();
        std::fs::write(src.join(".moss/secret"), b"x").unwrap();
        let tok = tokio_util::sync::CancellationToken::new();
        let (sha, _size, files) = build_archive(src.clone(), out.clone(), "{}".into(), tok.clone()).unwrap();
        assert_eq!(files, 1);
        assert!(verify_archive(&out, &sha).is_ok());
        assert!(verify_archive(&out, &"0".repeat(64)).is_err());
        extract_archive(&out, &dst, &tok, None).unwrap();
        assert_eq!(std::fs::read(dst.join("world/level.dat")).unwrap(), b"data");
        assert!(!dst.join(".moss").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn extraction_never_writes_controller_metadata() {
        let base = std::env::temp_dir().join(format!("moss-bk-meta-{}", std::process::id()));
        let (out, dst) = (base.join("evil.tar.zst"), base.join("dst"));
        std::fs::create_dir_all(&base).unwrap();
        let mut tar = tar::Builder::new(zstd::Encoder::new(std::fs::File::create(&out).unwrap(), 3).unwrap());
        for (name, body) in [(".moss/console.fifo", "x"), (".MOSS/undo", "x"), ("server.properties", "a=b")] {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64); h.set_mode(0o644); h.set_entry_type(tar::EntryType::Regular);
            tar.append_data(&mut h, name, body.as_bytes()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        extract_archive(&out, &dst, &tokio_util::sync::CancellationToken::new(), None).unwrap();
        assert!(dst.join("server.properties").is_file() && !dst.join(".moss").exists() && !dst.join(".MOSS").exists());
        let e = extract_archive_limited(&out, &base.join("small"), &tokio_util::sync::CancellationToken::new(), None, 2).unwrap_err().to_string();
        assert!(e.contains("safety limit"), "{e}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn worlds_and_selective_extraction() {
        let base = std::env::temp_dir().join(format!("moss-bk-sel-{}", std::process::id()));
        let (src, out, dst) = (base.join("src"), base.join("a.tar.zst"), base.join("dst"));
        for d in ["world", "world_nether", "plugins", "logs"] { std::fs::create_dir_all(src.join(d)).unwrap(); }
        for f in ["world/level.dat", "world_nether/level.dat", "plugins/a.jar", "server.properties", "logs/level.dat"] { std::fs::write(src.join(f), b"x").unwrap(); }
        assert_eq!(worlds_of(&src), vec!["world", "world_nether"]);
        let tok = tokio_util::sync::CancellationToken::new();
        let (sha, ..) = build_archive(src.clone(), out.clone(), "{}".into(), tok.clone()).unwrap();
        let (_, names) = verify_archive_scan(&out, &sha).unwrap();
        assert_eq!(names.into_iter().collect::<Vec<_>>(), vec!["plugins", "server.properties", "world", "world_nether"]);
        extract_archive(&out, &dst, &tok, Some(&["world_nether".to_string(), "server.properties".to_string()])).unwrap();
        assert!(dst.join("world_nether/level.dat").is_file() && dst.join("server.properties").is_file());
        assert!(!dst.join("world").exists() && !dst.join("plugins").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
