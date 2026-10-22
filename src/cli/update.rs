//! `moss update`: replace this binary with a newer release, the same way `scripts/install.sh` installs one. It finds
//! the release (latest published, or `--version`), downloads the platform tarball and `SHA256SUMS.txt`, checks the
//! cosign signature when cosign is installed, verifies the checksum, makes sure the new binary runs, and only then
//! renames it over the current executable. Game servers keep running; the controller picks up the new binary when it
//! is restarted (the command says how).
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{io::Read, path::{Path, PathBuf}};

#[derive(clap::Args)]
pub struct Update {
    /// Install this release instead of the latest one (for example v0.1.1); also allows going back to an older one
    #[arg(long)]
    version: Option<String>,
    /// Only report the installed and the latest version
    #[arg(long)]
    check: bool,
    /// Reinstall even when already up to date, and replace a binary that looks managed by a package manager
    #[arg(long)]
    force: bool,
    /// GitHub repository to update from
    #[arg(long, env = "MOSS_REPO", default_value = "T1nkiePlayz/Moss")]
    repo: String,
    /// Alternative download root laid out as <base>/<tag>/<asset> plus <base>/LATEST (mirrors and tests; https:// or file://)
    #[arg(long, env = "MOSS_RELEASE_BASE", hide = true)]
    release_base: Option<String>,
    /// Skip the optional cosign signature check
    #[arg(long, env = "MOSS_SKIP_COSIGN", value_parser = clap::builder::BoolishValueParser::new(), default_value_t = false)]
    skip_cosign: bool,
}

pub async fn run(a: Update) -> Result<()> {
    let exe = std::env::current_exe().context("cannot find this executable")?;
    let exe = exe.canonicalize().unwrap_or(exe); // a symlink (e.g. /usr/local/bin/moss -> …) is replaced at its target
    update(&a, &exe, env!("CARGO_PKG_VERSION")).await.map(|_| ())
}

#[derive(Debug, PartialEq)]
enum Outcome { UpToDate, Available(String), Installed(String) }

/// `vX.Y.Z[-pre]` → comparable numbers; a pre-release sorts before its release.
fn parse_version(v: &str) -> Option<(u64, u64, u64, bool)> {
    let v = v.trim().trim_start_matches('v');
    let (core, pre) = match v.split_once(['-', '+']) { Some((c, _)) => (c, v[c.len()..].starts_with('-')), None => (v, false) };
    let mut p = core.split('.').map(|n| n.parse::<u64>().ok());
    Some((p.next()??, p.next().flatten().unwrap_or(0), p.next().flatten().unwrap_or(0), !pre))
}

fn newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) { (Some(a), Some(b)) => a > b, _ => candidate.trim_start_matches('v') != current }
}

fn platform() -> Result<(&'static str, &'static str)> {
    let os = match std::env::consts::OS { "linux" => "linux", "macos" => "macos", o => bail!("there are no Moss releases for {o}") };
    let arch = match std::env::consts::ARCH { "x86_64" => "x86_64", "aarch64" => "aarch64", a => bail!("there are no Moss releases for {a}") };
    if os == "linux" && arch != "x86_64" { bail!("there is no prebuilt Linux {arch} release yet; build from source (cargo build --release)"); }
    Ok((os, arch))
}

/// Package managers own these copies; replacing them behind the manager's back breaks its next upgrade.
fn managed_by(exe: &Path) -> Option<&'static str> {
    let p = exe.to_string_lossy();
    if p.contains("/Cellar/") || p.contains("/homebrew/") { Some("Homebrew: run `brew upgrade moss`") }
    else if p.starts_with("/nix/store/") { Some("Nix: update it through your Nix configuration") }
    else if p.starts_with("/usr/bin/") || p.starts_with("/bin/") { Some("your system package manager (for example `yay -Syu moss` on Arch)") }
    else { None }
}

struct Source<'a> { base: Option<&'a str>, repo: &'a str, http: reqwest::Client, tmp: &'a Path }

impl Source<'_> {
    /// Copy one asset of release `tag` into the temp directory.
    async fn get(&self, tag: &str, name: &str) -> Result<PathBuf> {
        let dest = self.tmp.join(name);
        match self.base {
            Some(b) => self.get_url(&format!("{}/{tag}/{name}", b.trim_end_matches('/')), &dest).await?,
            None => self.get_url(&format!("https://github.com/{}/releases/download/{tag}/{name}", self.repo), &dest).await?,
        }
        Ok(dest)
    }

    async fn get_url(&self, url: &str, dest: &Path) -> Result<()> {
        if let Some(p) = url.strip_prefix("file://") {
            tokio::fs::copy(p, dest).await.with_context(|| format!("cannot read {p}"))?;
            return Ok(());
        }
        // a mirror given with MOSS_RELEASE_BASE is the one extra host allowed besides GitHub's download hosts
        let extra = self.base.and_then(|b| reqwest::Url::parse(b).ok()).and_then(|u| u.host_str().map(String::from));
        crate::download::fetch_ext(&self.http, url, dest, None, &Default::default(), |_| {}, crate::download::Opts { extra_host: extra, ..Default::default() })
            .await.with_context(|| format!("download failed: {url}"))?;
        Ok(())
    }

    async fn latest(&self) -> Result<String> {
        let tag = match self.base {
            Some(b) => {
                let f = self.tmp.join("LATEST");
                self.get_url(&format!("{}/LATEST", b.trim_end_matches('/')), &f).await?;
                std::fs::read_to_string(&f)?.trim().to_string()
            }
            None => {
                let url = format!("https://api.github.com/repos/{}/releases/latest", self.repo);
                let mut req = self.http.get(&url).header("accept", "application/vnd.github+json");
                // like install.sh: a token (private repository, or the anonymous rate limit was hit) goes to api.github.com only
                if let Ok(t) = std::env::var("GITHUB_TOKEN") && !t.is_empty() { req = req.bearer_auth(t); }
                let r = req.send().await.context("could not reach GitHub")?;
                if !r.status().is_success() { bail!("could not look up the latest release ({}); pass --version vX.Y.Z", r.status()); }
                let v: serde_json::Value = r.json().await?;
                v["tag_name"].as_str().unwrap_or_default().to_string()
            }
        };
        if tag.is_empty() { bail!("could not work out the latest version; pass --version vX.Y.Z"); }
        Ok(tag)
    }
}

/// The hardened download client; with a mirror, redirects may also stay on the mirror's own host (HTTPS only).
fn client(base: Option<&str>) -> reqwest::Client {
    let Some(mirror) = base.and_then(|b| reqwest::Url::parse(b).ok()).and_then(|u| u.host_str().map(String::from)) else { return crate::download::client("") };
    reqwest::Client::builder()
        .user_agent(format!("moss/{} (update)", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(move |a| {
            let ok = a.url().scheme() == "https" && a.url().host_str().is_some_and(|h| h == mirror || crate::download::host_allowed(h));
            if a.previous().len() > 5 { a.error("too many redirects") } else if ok { a.follow() } else { a.error("redirect to a host that is not allowed") }
        }))
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(60))
        .build().expect("http client")
}

fn normalise_tag(v: &str) -> Result<String> {
    let t = if v.starts_with('v') { v.to_string() } else { format!("v{v}") };
    if t.len() < 2 || !t[1..].chars().all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c)) { bail!("invalid version: {v}"); }
    Ok(t)
}

fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let (mut h, mut buf) = (Sha256::new(), vec![0u8; 1 << 16]);
    loop { let n = f.read(&mut buf)?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(hex::encode(h.finalize()))
}

/// The entry for `file` in a `sha256sum`-style list (`<hex>  <name>` or `<hex> *<name>`).
fn checksum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| { let mut p = l.split_whitespace(); let (h, n) = (p.next()?, p.next()?); (n.trim_start_matches('*') == file).then(|| h.to_ascii_lowercase()) })
}

/// Pull `<dir>/moss` (a regular file, size-capped) out of the release tarball.
fn extract_binary(tarball: &Path, dir: &str, out: &Path) -> Result<()> {
    const MAX: u64 = 512 << 20;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(tarball)?));
    let want = Path::new(dir).join("moss");
    for e in ar.entries()? {
        let mut e = e?;
        if e.path()? != want { continue; }
        if !e.header().entry_type().is_file() || e.size() > MAX { bail!("{} in the tarball is not a plain file", want.display()); }
        let mut f = std::fs::File::create(out)?;
        std::io::copy(&mut (&mut e).take(MAX), &mut f)?;
        return Ok(());
    }
    bail!("the tarball does not contain {}", want.display())
}

async fn cosign_check(src: &Source<'_>, tag: &str, repo: &str, sums: &Path, skip: bool) -> Result<()> {
    if skip { println!("Skipping the signature check (MOSS_SKIP_COSIGN)"); return Ok(()); }
    if std::process::Command::new("cosign").arg("version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_err() {
        println!("cosign is not installed; skipping the optional signature check (the checksum is still verified)");
        return Ok(());
    }
    let Ok(bundle) = src.get(tag, "SHA256SUMS.txt.sigstore.json").await else { println!("No signature bundle in this release; relying on the checksum only"); return Ok(()) };
    let repo_re = regex::escape(repo);
    let ok = tokio::process::Command::new("cosign").arg("verify-blob").arg("--bundle").arg(&bundle)
        .arg("--certificate-identity-regexp").arg(format!(r"^https://github\.com/{repo_re}/\.github/workflows/release\.yml@refs/(tags/v.+|heads/.+)$"))
        .arg("--certificate-oidc-issuer").arg("https://token.actions.githubusercontent.com").arg(sums)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await?.success();
    if !ok { bail!("the signature on SHA256SUMS.txt did not verify; nothing was changed (run cosign verify-blob by hand to see why)"); }
    println!("Signature OK (cosign, signed by the {repo} release workflow)");
    Ok(())
}

async fn update(a: &Update, exe: &Path, current: &str) -> Result<Outcome> {
    let (os, arch) = platform()?;
    let tmp = tempdir()?;
    let src = Source { base: a.release_base.as_deref(), repo: &a.repo, http: client(a.release_base.as_deref()), tmp: &tmp.0 };
    let tag = match &a.version { Some(v) => normalise_tag(v)?, None => normalise_tag(&src.latest().await?)? };
    let pinned = a.version.is_some();
    println!("Installed: v{current}   {}: {tag}", if pinned { "Requested" } else { "Latest" });
    let wanted = pinned || newer(&tag, current);
    if a.check { return Ok(if newer(&tag, current) { println!("An update is available: run `moss update`"); Outcome::Available(tag) } else { println!("Moss is up to date"); Outcome::UpToDate }); }
    if !wanted && !a.force { println!("Moss is up to date"); return Ok(Outcome::UpToDate); }
    if pinned && newer(current, &tag) { println!("Note: {tag} is older than this Moss. An older Moss refuses a database that a newer one has already upgraded; keep a copy of the data directory."); }
    if let Some(m) = managed_by(exe) && !a.force { bail!("{} looks managed by {m}, or pass --force to replace it anyway", exe.display()); }

    let name = format!("moss-{tag}-{os}-{arch}");
    let tarball = format!("{name}.tar.gz");
    println!("Downloading Moss {tag} for {os}-{arch}");
    let tb = src.get(&tag, &tarball).await?;
    let sums = src.get(&tag, "SHA256SUMS.txt").await?;
    cosign_check(&src, &tag, &a.repo, &sums, a.skip_cosign).await?;
    let want = checksum_for(&std::fs::read_to_string(&sums)?, &tarball).with_context(|| format!("SHA256SUMS.txt has no entry for {tarball}"))?;
    let have = sha256_file(&tb)?;
    if want != have { bail!("checksum mismatch for {tarball} (expected {want}, got {have}); nothing was changed"); }
    println!("Checksum OK ({have})");

    // Stage next to the executable so the final rename stays on one filesystem (atomic, and safe while this copy runs).
    let dir = exe.parent().context("the executable has no parent directory")?;
    // leftovers of an update that was interrupted (Ctrl-C skips the cleanup below)
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with(".moss-update-") && e.file_type().is_ok_and(|t| t.is_file()) { let _ = std::fs::remove_file(e.path()); }
    }
    let staged = dir.join(format!(".moss-update-{}", std::process::id()));
    let staged_guard = Cleanup(staged.clone());
    let extracted = tmp.0.join("moss");
    extract_binary(&tb, &name, &extracted)?;
    if let Err(e) = std::fs::copy(&extracted, &staged) {
        if e.kind() == std::io::ErrorKind::PermissionDenied { bail!("no permission to write to {}; rerun with sudo (sudo {} update)", dir.display(), exe.display()); }
        return Err(e).with_context(|| format!("cannot write to {}", dir.display()));
    }
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?; }
    std::fs::File::open(&staged)?.sync_all()?; // on disk before the rename, so a power cut cannot leave an empty `moss`
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("xattr").args(["-d", "com.apple.quarantine"]).arg(&staged).stderr(std::process::Stdio::null()).status(); }
    // the new binary must at least start before it replaces a working one
    let out = tokio::process::Command::new(&staged).arg("--version").output().await.context("the downloaded binary does not run")?;
    let reported = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !reported.starts_with("moss ") { bail!("the downloaded binary does not run on this machine; nothing was changed"); }
    std::fs::rename(&staged, exe).with_context(|| format!("cannot replace {}", exe.display()))?;
    std::mem::forget(staged_guard);
    if let Ok(d) = std::fs::File::open(dir) { let _ = d.sync_all(); } // make the rename itself durable
    println!("Updated {} to {tag} ({reported})", exe.display());
    println!();
    println!("Restart the controller to use it. Game servers keep running while it restarts.");
    if cfg!(target_os = "linux") { println!("  systemd:   sudo systemctl restart moss"); }
    else { println!("  launchd:   launchctl kickstart -k gui/$(id -u)/io.moss.controller"); }
    println!("  by hand:   stop `moss serve` and start it again");
    Ok(Outcome::Installed(tag))
}

/// Removes a path when dropped (the staged binary after a failed update).
struct Cleanup(PathBuf);
impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }

/// A private temp directory removed when dropped.
struct TmpDir(PathBuf);
impl Drop for TmpDir { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn tempdir() -> Result<TmpDir> {
    let p = std::env::temp_dir().join(format!("moss-update-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&p)?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700))?; }
    Ok(TmpDir(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("v0.1.10", "0.1.9"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("v0.1.1", "0.1.1"));
        assert!(!newer("v0.1.0", "0.2.0"));
        assert!(newer("v0.2.0", "0.2.0-rc.1"), "a release is newer than its pre-release");
        assert!(!newer("v0.2.0-rc.1", "0.2.0"));
    }

    #[test]
    fn tags_and_checksums_are_parsed_strictly() {
        assert_eq!(normalise_tag("0.1.1").unwrap(), "v0.1.1");
        for bad in ["v", "v1;rm -rf", "v1/../x", "v1 2"] { assert!(normalise_tag(bad).is_err(), "{bad}"); }
        let sums = "aa  other.tar.gz\nBB *moss-v1-linux-x86_64.tar.gz\n";
        assert_eq!(checksum_for(sums, "moss-v1-linux-x86_64.tar.gz").as_deref(), Some("bb"));
        assert_eq!(checksum_for(sums, "missing"), None);
    }

    #[test]
    fn package_managed_copies_are_left_alone() {
        assert!(managed_by(Path::new("/opt/homebrew/Cellar/moss/0.1.0/bin/moss")).is_some());
        assert!(managed_by(Path::new("/usr/bin/moss")).is_some());
        assert!(managed_by(Path::new("/usr/local/bin/moss")).is_none());
        assert!(managed_by(Path::new("/home/me/.local/bin/moss")).is_none());
    }

    /// A release directory laid out like MOSS_RELEASE_BASE: <base>/LATEST and <base>/<tag>/<assets>. The "binary" is a
    /// script that answers `--version` like Moss does.
    fn release(root: &Path, tag: &str, script: &str, corrupt_sum: bool) -> String {
        let (os, arch) = platform().unwrap();
        let name = format!("moss-{tag}-{os}-{arch}");
        let d = root.join(tag);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(root.join("LATEST"), format!("{tag}\n")).unwrap();
        let tb = d.join(format!("{name}.tar.gz"));
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(std::fs::File::create(&tb).unwrap(), flate2::Compression::fast()));
        let mut h = tar::Header::new_gnu();
        h.set_size(script.len() as u64); h.set_mode(0o755); h.set_entry_type(tar::EntryType::Regular); h.set_cksum();
        b.append_data(&mut h, format!("{name}/moss"), script.as_bytes()).unwrap();
        b.into_inner().unwrap().finish().unwrap();
        let sum = if corrupt_sum { "0".repeat(64) } else { sha256_file(&tb).unwrap() };
        std::fs::write(d.join("SHA256SUMS.txt"), format!("{sum}  {name}.tar.gz\n")).unwrap();
        format!("file://{}", root.display())
    }

    fn args(base: &str) -> Update { Update { version: None, check: false, force: false, repo: "T1nkiePlayz/Moss".into(), release_base: Some(base.into()), skip_cosign: true } }

    fn scratch(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("moss-update-test-{tag}-{}", uuid::Uuid::new_v4()));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("moss");
        std::fs::write(&exe, "old").unwrap();
        (root, exe)
    }

    #[tokio::test]
    async fn updates_to_the_latest_release_after_verifying_it() {
        let (root, exe) = scratch("ok");
        std::fs::write(exe.with_file_name(".moss-update-1"), "stale").unwrap();
        let base = release(&root.join("rel"), "v9.9.9", "#!/bin/sh\necho 'moss 9.9.9'\n", false);
        assert_eq!(update(&Update { check: true, ..args(&base) }, &exe, "0.1.0").await.unwrap(), Outcome::Available("v9.9.9".into()));
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old", "--check changes nothing");
        assert_eq!(update(&args(&base), &exe, "0.1.0").await.unwrap(), Outcome::Installed("v9.9.9".into()));
        assert!(std::fs::read_to_string(&exe).unwrap().contains("moss 9.9.9"));
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755); }
        assert_eq!(update(&args(&base), &exe, "9.9.9").await.unwrap(), Outcome::UpToDate);
        assert_eq!(std::fs::read_dir(exe.parent().unwrap()).unwrap().count(), 1, "no staged file left behind");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_bad_checksum_or_a_binary_that_does_not_run_changes_nothing() {
        let (root, exe) = scratch("bad");
        let base = release(&root.join("rel"), "v9.9.9", "#!/bin/sh\necho 'moss 9.9.9'\n", true);
        let e = update(&args(&base), &exe, "0.1.0").await.unwrap_err();
        assert!(format!("{e:#}").contains("checksum mismatch"), "{e:#}");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        let base = release(&root.join("rel2"), "v9.9.9", "#!/bin/sh\nexit 1\n", false);
        assert!(update(&args(&base), &exe, "0.1.0").await.is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        assert_eq!(std::fs::read_dir(exe.parent().unwrap()).unwrap().count(), 1, "the staged binary is removed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_pinned_version_installs_even_when_older() {
        let (root, exe) = scratch("pin");
        let base = release(&root.join("rel"), "v0.0.1", "#!/bin/sh\necho 'moss 0.0.1'\n", false);
        let a = Update { version: Some("0.0.1".into()), ..args(&base) };
        assert_eq!(update(&a, &exe, "0.2.0").await.unwrap(), Outcome::Installed("v0.0.1".into()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
