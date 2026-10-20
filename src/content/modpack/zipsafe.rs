//! Safe handling of modpack archives: path cleaning, entry validation (no traversal, no symlinks, size caps) and
//! extraction of `overrides/` + `server-overrides/` into a server directory through `files::safe_join`.
use crate::files::safe_join;
use anyhow::{Result, anyhow, bail};
use std::{io::{Read, Seek, Write}, path::Path};
use zip::ZipArchive;

pub const MAX_ENTRIES: usize = 50_000;
pub const MAX_ENTRY: u64 = 1 << 30;
pub const MAX_TOTAL: u64 = 4 << 30;

/// Top-level names that belong to the installed server software; packs never overwrite them.
const PROTECTED: &[&str] = &["server.jar", "libraries", "run.sh", "run.bat", "user_jvm_args.txt", "quilt-server-launch.jar", "eula.txt", "installer.log"];

/// Normalise a pack-relative path: no empty, absolute, drive-letter, backslash, NUL or `..` components, and never
/// `.moss`. Returns the cleaned `a/b/c` form.
pub fn clean_rel(p: &str) -> Result<String> {
    if p.is_empty() || p.contains(['\0', '\\']) || p.starts_with('/') || p.chars().nth(1) == Some(':') { bail!("unsafe path {p:?} in the pack"); }
    let mut parts = vec![];
    for c in p.split('/') {
        match c { "" | "." => {} ".." => bail!("unsafe path {p:?} in the pack"), c => parts.push(c) }
    }
    if parts.is_empty() || parts[0].eq_ignore_ascii_case(".moss") { bail!("path {p:?} in the pack is not allowed"); }
    Ok(parts.join("/"))
}

pub fn is_protected(rel: &str) -> bool { PROTECTED.contains(&rel.split('/').next().unwrap_or_default()) }

/// One overrides file to extract.
#[derive(Clone, Debug, PartialEq)]
pub struct OverrideEntry { pub index: usize, pub rel: String, pub size: u64 }

/// Validate every entry and list the overrides in extraction order (`common_dir/` first, `server-overrides/` after,
/// so the server-specific files win). Unsafe entries reject the whole pack. Returns (entries, skipped protected paths).
pub fn scan_overrides<R: Read + Seek>(z: &mut ZipArchive<R>, common_dir: &str) -> Result<(Vec<OverrideEntry>, Vec<String>)> {
    if z.len() > MAX_ENTRIES { bail!("the pack has more than {MAX_ENTRIES} entries"); }
    let (mut common, mut server, mut skipped, mut total) = (vec![], vec![], vec![], 0u64);
    for i in 0..z.len() {
        let f = z.by_index(i)?;
        let name = f.name()?.to_string();
        let common_prefix = format!("{common_dir}/");
        let (rest, is_server) = match (name.strip_prefix(common_prefix.as_str()), name.strip_prefix("server-overrides/")) { (Some(r), _) => (r, false), (_, Some(r)) => (r, true), _ => continue };
        if f.is_symlink() || f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) { bail!("the pack contains a symbolic link ({name:?}); refusing to import it"); }
        if f.is_dir() || rest.is_empty() { continue; }
        let rel = clean_rel(rest)?;
        if f.size() > MAX_ENTRY { bail!("{name:?} is larger than the per-file limit"); }
        total += f.size();
        if total > MAX_TOTAL { bail!("the pack's files exceed the total size limit"); }
        if is_protected(&rel) { skipped.push(rel); continue; }
        (if is_server { &mut server } else { &mut common }).push(OverrideEntry { index: i, rel, size: f.size() });
    }
    common.extend(server);
    Ok((common, skipped))
}

/// Write the entries under `root`. Every target goes through `safe_join` (no symlink components), is written to a
/// temp file and renamed, and the bytes actually read must not exceed what the header declared.
/// Returns the relative paths written, for rollback.
pub fn extract<R: Read + Seek>(z: &mut ZipArchive<R>, entries: &[OverrideEntry], root: &Path) -> Result<Vec<String>> {
    let mut written = vec![];
    for e in entries {
        let target = safe_join(root, &e.rel).map_err(|x| anyhow!(x.message))?;
        if target.is_dir() { bail!("{} is a directory on the server", e.rel); }
        if let Some(p) = target.parent() { std::fs::create_dir_all(p)?; }
        // re-check after creating parents: nothing along the way may have become a link
        let target = safe_join(root, &e.rel).map_err(|x| anyhow!(x.message))?;
        let tmp = target.with_file_name(format!(".{}.moss-tmp", target.file_name().and_then(|n| n.to_str()).unwrap_or("f")));
        let mut f = z.by_index(e.index)?;
        let mut out = std::fs::File::create(&tmp)?;
        let n = std::io::copy(&mut (&mut f).take(e.size + 1), &mut out);
        let n = match n { Ok(n) => n, Err(x) => { let _ = std::fs::remove_file(&tmp); return Err(x.into()); } };
        if n > e.size { let _ = std::fs::remove_file(&tmp); bail!("{} is larger than its header claims", e.rel); }
        out.flush()?;
        drop(out);
        std::fs::rename(&tmp, &target)?;
        written.push(e.rel.clone());
    }
    Ok(written)
}

#[cfg(test)]
pub(crate) mod tests;
