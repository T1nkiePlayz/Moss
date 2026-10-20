//! Validation of destination settings. Pure checks plus the filesystem probes a folder needs; nothing here
//! talks to the network.
use super::{dest::DestConfig, s3::S3Cfg};
use crate::error::ApiError;
use std::path::{Path, PathBuf};

fn bad(field: &str, msg: &str) -> ApiError { ApiError::bad("validation", msg.to_string()).field(field, msg) }

pub fn name(s: &str) -> Result<String, ApiError> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > 60 || s.chars().any(|c| c.is_control()) { return Err(bad("name", "1 to 60 characters")); }
    Ok(s.to_string())
}

pub fn keep(field: &str, n: i64, min: i64) -> Result<i64, ApiError> {
    if !(min..=1000).contains(&n) { return Err(bad(field, &format!("between {min} and 1000"))); }
    Ok(n)
}

/// An absolute, existing, writable directory that is not inside (or equal to) the Moss data directory.
pub fn folder(path: &str, data_dir: &Path) -> Result<PathBuf, ApiError> {
    let p = Path::new(path.trim());
    if !p.is_absolute() { return Err(bad("path", "enter the full path, starting with /")); }
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) { return Err(bad("path", "the path may not contain ..")); }
    let real = std::fs::canonicalize(p).map_err(|_| bad("path", "this folder does not exist; create or mount it first"))?;
    if !real.is_dir() { return Err(bad("path", "this is not a folder")); }
    if real.parent().is_none() { return Err(bad("path", "choose a folder, not the root of the disk")); }
    let data = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    if real.starts_with(&data) { return Err(bad("path", "this folder is inside Moss's own data folder; backups there would not be separate copies")); }
    let probe = real.join(format!(".moss-write-test-{}", uuid::Uuid::new_v4()));
    match std::fs::write(&probe, b"x") { Ok(()) => { let _ = std::fs::remove_file(&probe); } Err(_) => return Err(bad("path", "Moss cannot write to this folder; check its permissions and that the disk is mounted read-write")) }
    Ok(real)
}

fn loopback_host(h: &str) -> bool { h == "localhost" || h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|i| i.is_loopback()) }

pub fn prefix(s: &str) -> Result<String, ApiError> {
    let t = s.trim().trim_matches('/');
    if t.chars().count() > 200 || t.contains("..") || t.contains("//") || t.contains('\\') || t.chars().any(|c| c.is_control()) { return Err(bad("prefix", "up to 200 characters, no .. or backslashes")); }
    Ok(t.to_string())
}

/// Normalises and checks S3 settings. `allow_private` is MOSS_ALLOW_PRIVATE_BACKUP_HOSTS (it also permits plain http to loopback).
pub fn s3(endpoint: &str, region: &str, bucket: &str, pre: &str, path_style: bool, allow_private: bool) -> Result<S3Cfg, ApiError> {
    let region = region.trim().to_string();
    if region.is_empty() || region.len() > 32 || !region.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') { return Err(bad("region", "for example us-east-1 or eu-central-003")); }
    let bucket = bucket.trim().to_string();
    let ok = (3..=63).contains(&bucket.len()) && bucket.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && !bucket.starts_with(['-', '.']) && !bucket.ends_with(['-', '.']) && !bucket.contains("..");
    if !ok { return Err(bad("bucket", "3 to 63 lowercase letters, digits, dots and dashes")); }
    let endpoint = match endpoint.trim() { "" => format!("https://s3.{region}.amazonaws.com"), e => e.trim_end_matches('/').to_string() };
    let u = reqwest::Url::parse(&endpoint).map_err(|_| bad("endpoint", "not a valid URL, for example https://s3.us-west-004.backblazeb2.com"))?;
    let host = u.host_str().unwrap_or("").to_string();
    if host.is_empty() || !u.username().is_empty() || u.password().is_some() || u.query().is_some() || u.fragment().is_some() || !(u.path().is_empty() || u.path() == "/") {
        return Err(bad("endpoint", "use only the address, like https://host or https://host:port (no login, path or query)"));
    }
    match u.scheme() {
        "https" => {}
        "http" if allow_private && loopback_host(&host) => {}
        "http" => return Err(bad("endpoint", "the endpoint must use https")),
        _ => return Err(bad("endpoint", "the endpoint must use https")),
    }
    if !path_style && (host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') || host == "localhost" || bucket.contains('.') && u.scheme() == "https") {
        return Err(bad("path_style", "turn on path-style addressing for this endpoint (virtual-hosted style needs a DNS name and a bucket name without dots)"));
    }
    let cfg = S3Cfg { endpoint: endpoint.clone(), region, bucket, prefix: prefix(pre)?, path_style };
    Ok(cfg)
}

pub fn credential(field: &str, s: &str) -> Result<String, ApiError> {
    let s = s.trim();
    if s.is_empty() || s.len() > 256 || s.chars().any(|c| c.is_control() || c.is_whitespace()) { return Err(bad(field, "required; no spaces")); }
    Ok(s.to_string())
}

pub fn config_of(c: &DestConfig, data_dir: &Path, allow_private: bool) -> Result<DestConfig, ApiError> {
    Ok(match c {
        DestConfig::Folder { path } => DestConfig::Folder { path: folder(path, data_dir)?.to_string_lossy().to_string() },
        DestConfig::S3 { endpoint, region, bucket, prefix, path_style } => { let c = s3(endpoint, region, bucket, prefix, *path_style, allow_private)?; DestConfig::S3 { endpoint: c.endpoint, region: c.region, bucket: c.bucket, prefix: c.prefix, path_style: c.path_style } }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf { let d = std::env::temp_dir().join(format!("moss-val-{}", uuid::Uuid::new_v4())); std::fs::create_dir_all(&d).unwrap(); d }

    #[test]
    fn folders_must_be_absolute_existing_writable_and_outside_the_data_dir() {
        let (data, out) = (tmp(), tmp());
        std::fs::create_dir_all(data.join("servers/abc")).unwrap();
        assert!(folder(out.to_str().unwrap(), &data).is_ok());
        assert!(folder("relative/dir", &data).is_err());
        assert!(folder(&format!("{}/../x", out.display()), &data).is_err());
        assert!(folder(out.join("missing").to_str().unwrap(), &data).is_err());
        assert!(folder("/", &data).is_err());
        assert!(folder(data.to_str().unwrap(), &data).is_err());
        assert!(folder(data.join("servers/abc").to_str().unwrap(), &data).is_err());
        let file = out.join("f"); std::fs::write(&file, b"x").unwrap();
        assert!(folder(file.to_str().unwrap(), &data).is_err());
        // a symlink into the data dir is resolved and refused
        #[cfg(unix)] { let l = out.join("link"); std::os::unix::fs::symlink(data.join("servers"), &l).unwrap(); assert!(folder(l.to_str().unwrap(), &data).is_err()); }
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let ro = tmp(); std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();
            if !nix_root() { assert!(folder(ro.to_str().unwrap(), &data).is_err()); } std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap(); }
    }
    #[cfg(unix)] fn nix_root() -> bool { unsafe { libc::geteuid() == 0 } }

    #[test]
    fn s3_settings_are_normalised_and_checked() {
        let c = s3("", "us-east-1", "my-bucket", "/moss/backups/", false, false).unwrap();
        assert_eq!((c.endpoint.as_str(), c.prefix.as_str()), ("https://s3.us-east-1.amazonaws.com", "moss/backups"));
        assert!(s3("https://s3.us-west-004.backblazeb2.com/", "us-west-004", "bkt", "", true, false).is_ok());
        for (e, r, b, p, ps) in [("http://example.com", "us-east-1", "bkt", "", true), ("ftp://example.com", "us-east-1", "bkt", "", true), ("https://u:p@example.com", "us-east-1", "bkt", "", true),
            ("https://example.com/path", "us-east-1", "bkt", "", true), ("https://example.com?x=1", "us-east-1", "bkt", "", true), ("not a url", "us-east-1", "bkt", "", true),
            ("", "US East", "bkt", "", true), ("", "", "bkt", "", true), ("", "us-east-1", "Bad_Bucket", "", true), ("", "us-east-1", "ab", "", true), ("", "us-east-1", "a..b", "", true),
            ("", "us-east-1", "bkt", "../x", true), ("", "us-east-1", "bkt", "a\\b", true), ("https://10.0.0.5:9000", "us-east-1", "bkt", "", false), ("", "us-east-1", "my.dotted.bucket", "", false)] {
            assert!(s3(e, r, b, p, ps, false).is_err(), "{e} {r} {b} {p}");
        }
        // plain http is only for loopback and only with the private-hosts opt-in
        assert!(s3("http://127.0.0.1:9000", "us-east-1", "bkt", "", true, false).is_err());
        assert!(s3("http://127.0.0.1:9000", "us-east-1", "bkt", "", true, true).is_ok());
        assert!(s3("http://192.168.1.5:9000", "us-east-1", "bkt", "", true, true).is_err());
        assert!(s3("http://example.com", "us-east-1", "bkt", "", true, true).is_err());
    }

    #[test]
    fn names_keeps_and_credentials() {
        assert!(name("  NAS ").is_ok() && name("").is_err() && name(&"x".repeat(61)).is_err());
        assert!(keep("keep_local", 0, 0).is_ok() && keep("keep_remote", 0, 1).is_err() && keep("keep_remote", 1001, 1).is_err());
        assert!(credential("access_key", "AKIA123").is_ok() && credential("access_key", "a b").is_err() && credential("access_key", "").is_err());
    }
}
