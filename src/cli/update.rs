//! `moss update`: replace this binary with a newer release, the same way `scripts/install.sh` installs one. It finds
//! the release (latest published, or `--version`), downloads the platform tarball and `SHA256SUMS.txt`, checks the
//! cosign signature when cosign is installed, verifies the checksum, makes sure the new binary runs, and only then
//! renames it over the current executable. Game servers keep running; the controller picks up the new binary when it
//! is restarted (the command says how).
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{io::Read, path::{Path, PathBuf}};
use std::ffi::OsStr;

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
    /// Reinstall the release that is already installed without asking
    #[arg(long)]
    reinstall: bool,
    /// How to get elevated rights for the final copy when the binary's folder is not writable (the CLI uses `sudo`,
    /// which asks for the password; the panel never elevates).
    #[arg(skip)]
    sudo: Option<PathBuf>,
    /// GitHub repository to update from
    #[arg(long, env = "MOSS_REPO", default_value = "T1nkiePlayz/Moss")]
    pub(crate) repo: String,
    /// Alternative download root laid out as <base>/<tag>/<asset> plus <base>/LATEST (mirrors and tests; https:// or file://)
    #[arg(long, env = "MOSS_RELEASE_BASE", hide = true)]
    release_base: Option<String>,
    /// Read the version to install from this file and delete it (used by the root `moss-update` systemd helper; the
    /// panel writes the file). The file must be a regular file, not a link.
    #[arg(long, value_name = "FILE")]
    request: Option<PathBuf>,
    /// After installing, restart this systemd unit (`systemctl try-restart`), e.g. moss.service
    #[arg(long, value_name = "UNIT")]
    restart_service: Option<String>,
    /// Skip the optional cosign signature check
    #[arg(long, env = "MOSS_SKIP_COSIGN", value_parser = clap::builder::BoolishValueParser::new(), default_value_t = false)]
    skip_cosign: bool,
}

pub async fn run(mut a: Update) -> Result<()> {
    let exe = std::env::current_exe().context("cannot find this executable")?;
    let exe = exe.canonicalize().unwrap_or(exe); // a symlink (e.g. /usr/local/bin/moss -> …) is replaced at its target
    if let Some(req) = a.request.take() {
        match read_request(&req)? {
            Some(v) if newer(&v, env!("CARGO_PKG_VERSION")) => a.version = Some(v),
            Some(v) => { println!("Ignoring the request for {v}: v{} is installed and requests only move forward", env!("CARGO_PKG_VERSION")); return Ok(()); }
            None => { println!("No update requested"); return Ok(()); }
        }
    }
    a.sudo = Some("sudo".into());
    let say = |l: String| println!("{l}");
    let mut out = update(&a, &exe, env!("CARGO_PKG_VERSION"), &say).await?;
    if matches!(out, Outcome::AlreadyInstalled) && !a.check && a.restart_service.is_none() {
        match ask("Do you really want to reinstall it? [y/N] ")? {
            Some(true) => { a.reinstall = true; out = update(&a, &exe, env!("CARGO_PKG_VERSION"), &say).await?; }
            Some(false) => println!("Nothing changed."),
            None => {}
        }
    }
    if !matches!(out, Outcome::Installed(_)) { return Ok(()); }
    if let Some(unit) = &a.restart_service {
        let ok = std::process::Command::new("systemctl").args(["try-restart", unit]).status().is_ok_and(|s| s.success());
        if !ok { bail!("installed, but restarting {unit} failed; restart it by hand"); }
        println!("Restarted {unit}");
        return Ok(());
    }
    println!();
    println!("Restart the controller to use it. Game servers keep running while it restarts.");
    if cfg!(target_os = "linux") { println!("  systemd:   sudo systemctl restart moss"); }
    else { println!("  launchd:   launchctl kickstart -k gui/$(id -u)/io.moss.controller"); }
    println!("  by hand:   stop `moss serve` and start it again");
    Ok(())
}

/// Ask a yes/no question on the terminal; None without asking when not run interactively.
fn ask(q: &str) -> Result<Option<bool>> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() { return Ok(None); }
    print!("{q}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(Some(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")))
}

/// The panel's update request: a tag in a small regular file in the data directory, which a less privileged user
/// (Moss) wrote and root reads. It is opened without following links, size-capped, validated and removed.
fn read_request(p: &Path) -> Result<Option<String>> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(p) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => { discard(p); bail!("cannot read the update request {}: {e}", p.display()); }
    };
    discard(p); // one request, one attempt; whatever it is, so the path unit does not fire again
    if !f.metadata()?.is_file() { bail!("the update request is not a regular file"); }
    let mut s = String::new();
    f.take(64).read_to_string(&mut s)?;
    let v = s.trim();
    if v.is_empty() { return Ok(None); }
    // never echo the contents: root reads this file and the error goes to the journal
    normalise_tag(v).map(Some).map_err(|_| anyhow::anyhow!("the update request does not name a release"))
}

fn discard(p: &Path) { if std::fs::remove_file(p).is_err() { let _ = std::fs::remove_dir(p); } }

/// How this binary can be updated from the panel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Method { SelfUpdate, Helper, Manual { reason: String, command: String } }

/// The path of the systemd helper that installs panel-requested updates as root.
/// Present once `systemctl enable moss-update.path` has run.
pub(crate) const HELPER_UNIT: &str = "/etc/systemd/system/paths.target.wants/moss-update.path";
pub(crate) const HELPER_DATA_DIR: &str = "/var/lib/moss";

pub(crate) fn method(exe: &Path, data_dir: &Path) -> Method {
    let dir = exe.parent().unwrap_or(Path::new("/"));
    if let Some(m) = managed_by(exe) { return Method::Manual { reason: format!("This copy of Moss is managed by {m}."), command: String::new() }; }
    if writable(dir) && writable(exe) { return Method::SelfUpdate; }
    if Path::new(HELPER_UNIT).exists() && data_dir.canonicalize().is_ok_and(|d| d == Path::new(HELPER_DATA_DIR)) { return Method::Helper; }
    let under_systemd = std::env::var_os("INVOCATION_ID").is_some();
    Method::Manual {
        reason: format!("Moss cannot replace {} itself (no write access). Run the update on the host, or install the moss-update helper (see the README, Updating).", exe.display()),
        command: if under_systemd { format!("sudo {} update && sudo systemctl restart moss", exe.display()) } else { format!("sudo {} update", exe.display()) },
    }
}

fn writable(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Settings for an update started by the controller (repository and mirror from the same environment as the CLI).
pub(crate) fn server_args(version: Option<String>) -> Update {
    Update {
        version, check: false, force: false, reinstall: false, sudo: None, request: None, restart_service: None,
        repo: std::env::var("MOSS_REPO").ok().filter(|r| !r.is_empty()).unwrap_or_else(|| "T1nkiePlayz/Moss".into()),
        release_base: std::env::var("MOSS_RELEASE_BASE").ok().filter(|b| !b.is_empty()),
        skip_cosign: std::env::var("MOSS_SKIP_COSIGN").is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes")),
    }
}

/// The newest published release tag.
pub(crate) async fn latest(a: &Update) -> Result<String> {
    let tmp = tempdir()?;
    let src = Source { base: a.release_base.as_deref(), repo: &a.repo, http: client(a.release_base.as_deref()), tmp: &tmp.0 };
    normalise_tag(&src.latest().await?)
}

/// Install release `a.version` (or the latest) over `exe`. Ok(Some(tag)) when installed, Ok(None) when already current.
pub(crate) async fn install(a: &Update, exe: &Path, say: Say<'_>) -> Result<Option<String>> {
    Ok(match update(a, exe, env!("CARGO_PKG_VERSION"), say).await? { Outcome::Installed(t) => Some(t), _ => None })
}

pub(crate) fn is_newer(candidate: &str, current: &str) -> bool { newer(candidate, current) }
pub(crate) fn valid_tag(v: &str) -> Result<String> { normalise_tag(v) }

#[derive(Debug, PartialEq)]
enum Outcome { UpToDate, AlreadyInstalled, Available(String), Installed(String) }

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

/// Progress lines: printed by the CLI, written to the operation log by the panel.
pub(crate) type Say<'a> = &'a (dyn Fn(String) + Send + Sync);

async fn cosign_check(src: &Source<'_>, tag: &str, repo: &str, sums: &Path, skip: bool, say: Say<'_>) -> Result<()> {
    if skip { say("Skipping the signature check (MOSS_SKIP_COSIGN)".into()); return Ok(()); }
    if std::process::Command::new("cosign").arg("version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_err() {
        say("cosign is not installed; skipping the optional signature check (the checksum is still verified)".into());
        return Ok(());
    }
    let Ok(bundle) = src.get(tag, "SHA256SUMS.txt.sigstore.json").await else { say("No signature bundle in this release; relying on the checksum only".into()); return Ok(()) };
    let repo_re = regex::escape(repo);
    let ok = tokio::process::Command::new("cosign").arg("verify-blob").arg("--bundle").arg(&bundle)
        .arg("--certificate-identity-regexp").arg(format!(r"^https://github\.com/{repo_re}/\.github/workflows/release\.yml@refs/(tags/v.+|heads/.+)$"))
        .arg("--certificate-oidc-issuer").arg("https://token.actions.githubusercontent.com").arg(sums)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await?.success();
    if !ok { bail!("the signature on SHA256SUMS.txt did not verify; nothing was changed (run cosign verify-blob by hand to see why)"); }
    say(format!("Signature OK (cosign, signed by the {repo} release workflow)"));
    Ok(())
}

async fn update(a: &Update, exe: &Path, current: &str, say: Say<'_>) -> Result<Outcome> {
    let (os, arch) = platform()?;
    let tmp = tempdir()?;
    let src = Source { base: a.release_base.as_deref(), repo: &a.repo, http: client(a.release_base.as_deref()), tmp: &tmp.0 };
    let tag = match &a.version { Some(v) => normalise_tag(v)?, None => normalise_tag(&src.latest().await?)? };
    let pinned = a.version.is_some();
    say(format!("Installed: v{current}   {}: {tag}", if pinned { "Requested" } else { "Latest" }));
    let same = !newer(&tag, current) && !newer(current, &tag);
    let wanted = if same { a.reinstall } else { pinned || newer(&tag, current) };
    if a.check { return Ok(if newer(&tag, current) { say("An update is available: run `moss update`".into()); Outcome::Available(tag) } else { say("Moss is up to date".into()); Outcome::UpToDate }); }
    if !wanted && !a.force {
        if same { say(format!("Moss {tag} is already installed")); return Ok(Outcome::AlreadyInstalled); }
        say("Moss is up to date".into());
        return Ok(Outcome::UpToDate);
    }
    if pinned && newer(current, &tag) { say(format!("Note: {tag} is older than this Moss. An older Moss refuses a database that a newer one has already upgraded; keep a copy of the data directory.")); }
    if let Some(m) = managed_by(exe) && !a.force { bail!("{} looks managed by {m}, or pass --force to replace it anyway", exe.display()); }

    let name = format!("moss-{tag}-{os}-{arch}");
    let tarball = format!("{name}.tar.gz");
    say(format!("Downloading Moss {tag} for {os}-{arch}"));
    let tb = src.get(&tag, &tarball).await?;
    let sums = src.get(&tag, "SHA256SUMS.txt").await?;
    cosign_check(&src, &tag, &a.repo, &sums, a.skip_cosign, say).await?;
    let want = checksum_for(&std::fs::read_to_string(&sums)?, &tarball).with_context(|| format!("SHA256SUMS.txt has no entry for {tarball}"))?;
    let have = sha256_file(&tb)?;
    if want != have { bail!("checksum mismatch for {tarball} (expected {want}, got {have}); nothing was changed"); }
    say(format!("Checksum OK ({have})"));

    // Stage next to the executable so the final rename stays on one filesystem (atomic, and safe while this copy runs).
    let dir = exe.parent().context("the executable has no parent directory")?;
    let staged = dir.join(format!(".moss-update-{}", std::process::id()));
    let extracted = tmp.0.join("moss");
    extract_binary(&tb, &name, &extracted)?;
    // Like the install script: everything runs as this user, and only the copy into a folder they cannot write uses
    // sudo (which asks for the password). The copy is owned by root, so this user cannot change it afterwards.
    let sudo = a.sudo.as_deref().filter(|_| !writable(dir) && unsafe { libc::geteuid() } != 0);
    let manual = || format!("no permission to write to {}; rerun with sudo (sudo {} update)", dir.display(), exe.display());
    let staged_guard = Cleanup(staged.clone(), sudo.map(Path::to_path_buf));
    // leftovers of an update that was interrupted (Ctrl-C skips the cleanup below)
    let stale: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".moss-update-") && e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path()).collect();
    if let Some(sudo) = sudo {
        say(format!("{} needs elevated rights; using sudo for the copy only", dir.display()));
        if !stale.is_empty() {
            let mut args = vec![OsStr::new("rm"), OsStr::new("-f")];
            args.extend(stale.iter().map(|p| p.as_os_str()));
            let _ = elevated(sudo, &args);
        }
        let ok = elevated(sudo, &[OsStr::new("install"), OsStr::new("-m"), OsStr::new("0755"), OsStr::new("-o"), OsStr::new("0"), OsStr::new("-g"), OsStr::new("0"), extracted.as_os_str(), staged.as_os_str()]);
        if !ok { bail!("{}", manual()); }
    } else {
        for p in &stale { let _ = std::fs::remove_file(p); }
        if let Err(e) = std::fs::copy(&extracted, &staged) {
            if e.kind() == std::io::ErrorKind::PermissionDenied { bail!("{}", manual()); }
            return Err(e).with_context(|| format!("cannot write to {}", dir.display()));
        }
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?; }
        #[cfg(target_os = "macos")]
        { let _ = std::process::Command::new("xattr").args(["-d", "com.apple.quarantine"]).arg(&staged).stderr(std::process::Stdio::null()).status(); }
    }
    std::fs::File::open(&staged)?.sync_all()?; // on disk before the rename, so a power cut cannot leave an empty `moss`
    // the new binary must at least start before it replaces a working one
    let out = tokio::process::Command::new(&staged).arg("--version").output().await.context("the downloaded binary does not run")?;
    let reported = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !reported.starts_with("moss ") { bail!("the downloaded binary does not run on this machine; nothing was changed"); }
    match sudo {
        Some(sudo) => if !elevated(sudo, &[OsStr::new("mv"), OsStr::new("-f"), staged.as_os_str(), exe.as_os_str()]) { bail!("cannot replace {}", exe.display()); },
        None => std::fs::rename(&staged, exe).with_context(|| format!("cannot replace {}", exe.display()))?,
    }
    std::mem::forget(staged_guard);
    if let Ok(d) = std::fs::File::open(dir) { let _ = d.sync_all(); } // make the rename itself durable
    say(format!("Updated {} to {tag} ({reported})", exe.display()));
    Ok(Outcome::Installed(tag))
}

/// Removes a path when dropped (the staged binary after a failed update), through sudo when it was copied with sudo.
struct Cleanup(PathBuf, Option<PathBuf>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        match &self.1 {
            Some(sudo) => if self.0.exists() { let _ = elevated(sudo, &[OsStr::new("rm"), OsStr::new("-f"), self.0.as_os_str()]); },
            None => { let _ = std::fs::remove_file(&self.0); }
        }
    }
}

/// Run a command through sudo on this terminal (sudo asks for the password itself). True when it succeeded.
fn elevated(sudo: &Path, args: &[&OsStr]) -> bool {
    std::process::Command::new(sudo).arg("--").args(args).status().is_ok_and(|s| s.success())
}

/// A private temp directory removed when dropped.
struct TmpDir(PathBuf);
impl Drop for TmpDir { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn tempdir() -> Result<TmpDir> {
    let p = std::env::temp_dir().join(format!("moss-update-{}", uuid::Uuid::new_v4()));
    // private from the moment it exists: what is verified here may later be copied into place as root
    { use std::os::unix::fs::DirBuilderExt; std::fs::DirBuilder::new().mode(0o700).create(&p)?; }
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

    fn args(base: &str) -> Update { Update { version: None, check: false, force: false, reinstall: false, sudo: None, request: None, restart_service: None, repo: "T1nkiePlayz/Moss".into(), release_base: Some(base.into()), skip_cosign: true } }

    fn quiet(_: String) {}

    #[test]
    fn requests_are_read_once_and_never_through_a_link() {
        let d = std::env::temp_dir().join(format!("moss-update-req-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let req = d.join("update-request");
        std::fs::write(&req, "0.1.2\n").unwrap();
        assert_eq!(read_request(&req).unwrap().as_deref(), Some("v0.1.2"));
        assert!(!req.exists(), "consumed");
        assert_eq!(read_request(&req).unwrap(), None);
        std::fs::write(&req, "v1;reboot").unwrap();
        assert!(read_request(&req).is_err());
        let secret = d.join("secret");
        std::fs::write(&secret, "v9.9.9").unwrap();
        std::os::unix::fs::symlink(&secret, &req).unwrap();
        assert!(read_request(&req).is_err(), "a link is refused");
        assert!(secret.exists() && !req.exists());
        std::fs::create_dir(&req).unwrap();
        assert!(read_request(&req).is_err());
        assert!(!req.exists(), "a directory is removed too, so the path unit does not loop");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_update_method_follows_write_access() {
        let d = std::env::temp_dir().join(format!("moss-update-m-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let exe = d.join("moss");
        std::fs::write(&exe, "x").unwrap();
        assert_eq!(method(&exe, &d), Method::SelfUpdate);
        assert!(matches!(method(Path::new("/opt/homebrew/Cellar/moss/1/bin/moss"), &d), Method::Manual { .. }));
        if unsafe { libc::geteuid() } != 0 {
            assert!(matches!(method(Path::new("/proc/version"), &d), Method::Manual { .. } | Method::Helper));
        }
        let _ = std::fs::remove_dir_all(&d);
    }

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
        assert_eq!(update(&Update { check: true, ..args(&base) }, &exe, "0.1.0", &quiet).await.unwrap(), Outcome::Available("v9.9.9".into()));
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old", "--check changes nothing");
        assert_eq!(update(&args(&base), &exe, "0.1.0", &quiet).await.unwrap(), Outcome::Installed("v9.9.9".into()));
        assert!(std::fs::read_to_string(&exe).unwrap().contains("moss 9.9.9"));
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755); }
        assert_eq!(update(&args(&base), &exe, "9.9.9", &quiet).await.unwrap(), Outcome::AlreadyInstalled);
        assert_eq!(update(&args(&base), &exe, "10.0.0", &quiet).await.unwrap(), Outcome::UpToDate, "a build newer than the latest release is not offered a reinstall");
        assert_eq!(update(&Update { version: Some("9.9.9".into()), ..args(&base) }, &exe, "9.9.9", &quiet).await.unwrap(), Outcome::AlreadyInstalled, "the installed release is not reinstalled unasked");
        assert_eq!(update(&Update { reinstall: true, ..args(&base) }, &exe, "9.9.9", &quiet).await.unwrap(), Outcome::Installed("v9.9.9".into()));
        assert_eq!(std::fs::read_dir(exe.parent().unwrap()).unwrap().count(), 1, "no staged file left behind");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A stand-in for sudo: logs the command, opens the folder for it and runs it (without the root ownership).
    fn fake_sudo(root: &Path, bin: &Path, fail: bool) -> PathBuf {
        let p = root.join(if fail { "sudo-no" } else { "sudo" });
        let body = if fail { "exit 1".to_string() } else { format!(r#"[ "$1" = -- ] && shift
chmod u+w '{b}'
if [ "$1" = install ]; then install -m 0755 "$8" "$9"; else "$@"; fi
s=$?
chmod u-w '{b}'
exit $s"#, b = bin.display()) };
        std::fs::write(&p, format!("#!/bin/sh\necho \"$*\" >> '{}'\n{body}\n", root.join("sudo.log").display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[tokio::test]
    async fn a_folder_this_user_cannot_write_is_updated_through_sudo_for_the_copy_only() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 { return; } // root can write anywhere
        let (root, exe) = scratch("sudo");
        let bin = exe.parent().unwrap().to_path_buf();
        let base = release(&root.join("rel"), "v9.9.9", "#!/bin/sh\necho 'moss 9.9.9'\n", false);
        std::fs::write(bin.join(".moss-update-1"), "stale").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o555)).unwrap();

        let err = update(&args(&base), &exe, "0.1.0", &quiet).await.unwrap_err();
        assert!(format!("{err:#}").contains("rerun with sudo"), "without sudo it says what to run: {err:#}");
        let refused = Update { sudo: Some(fake_sudo(&root, &bin, true)), ..args(&base) };
        assert!(format!("{:#}", update(&refused, &exe, "0.1.0", &quiet).await.unwrap_err()).contains("rerun with sudo"));
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old", "a refused password changes nothing");

        let a = Update { sudo: Some(fake_sudo(&root, &bin, false)), ..args(&base) };
        assert_eq!(update(&a, &exe, "0.1.0", &quiet).await.unwrap(), Outcome::Installed("v9.9.9".into()));
        assert!(std::fs::read_to_string(&exe).unwrap().contains("moss 9.9.9"));
        let log = std::fs::read_to_string(root.join("sudo.log")).unwrap();
        let cmds: Vec<&str> = log.lines().filter_map(|l| l.strip_prefix("-- ")).map(|l| l.split(' ').next().unwrap()).collect();
        assert_eq!(cmds[2..], ["rm", "install", "mv"], "only the cleanup, the copy and the swap are elevated: {log}");
        assert!(log.contains("install -m 0755 -o 0 -g 0 "), "the copy is owned by root");
        assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 1, "no staged file left behind");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_bad_checksum_or_a_binary_that_does_not_run_changes_nothing() {
        let (root, exe) = scratch("bad");
        let base = release(&root.join("rel"), "v9.9.9", "#!/bin/sh\necho 'moss 9.9.9'\n", true);
        let e = update(&args(&base), &exe, "0.1.0", &quiet).await.unwrap_err();
        assert!(format!("{e:#}").contains("checksum mismatch"), "{e:#}");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        let base = release(&root.join("rel2"), "v9.9.9", "#!/bin/sh\nexit 1\n", false);
        assert!(update(&args(&base), &exe, "0.1.0", &quiet).await.is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        assert_eq!(std::fs::read_dir(exe.parent().unwrap()).unwrap().count(), 1, "the staged binary is removed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_pinned_version_installs_even_when_older() {
        let (root, exe) = scratch("pin");
        let base = release(&root.join("rel"), "v0.0.1", "#!/bin/sh\necho 'moss 0.0.1'\n", false);
        let a = Update { version: Some("0.0.1".into()), ..args(&base) };
        assert_eq!(update(&a, &exe, "0.2.0", &quiet).await.unwrap(), Outcome::Installed("v0.0.1".into()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
