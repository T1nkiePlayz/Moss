//! The data-directory lock: the controller holds an exclusive `flock` on `<data>/moss.lock` for as long as it runs.
//! `moss recover` tries the same lock to learn whether a controller is alive. The kernel drops the lock when the
//! process dies, even on SIGKILL, so a stale lock file never blocks anything. The descriptor is close-on-exec, so
//! game servers started by the controller do not inherit it.
use std::{fs::File, path::Path};

/// Create the data directory if it is missing, private (0700) because it holds the database, backups and secrets. An
/// existing directory keeps whatever mode its owner gave it; missing parents get the default mode.
pub fn create_data_dir(data_dir: &Path) -> std::io::Result<()> {
    if data_dir.is_dir() { return Ok(()); }
    if let Some(parent) = data_dir.parent().filter(|p| !p.as_os_str().is_empty()) { std::fs::create_dir_all(parent)?; }
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(data_dir) { Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => Err(e), _ => Ok(()) }
}

/// Try to take the lock. `Ok(None)` means another process holds it.
pub fn try_acquire(data_dir: &Path) -> std::io::Result<Option<File>> {
    create_data_dir(data_dir)?;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(data_dir.join("moss.lock"))?;
    use std::os::fd::AsRawFd;
    // SAFETY: plain flock(2) on a descriptor we own.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 { return Ok(Some(f)); }
    let e = std::io::Error::last_os_error();
    if e.kind() == std::io::ErrorKind::WouldBlock { Ok(None) } else { Err(e) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn second_holder_is_refused_until_the_first_drops() {
        let d = std::env::temp_dir().join(format!("moss-lock-{}", uuid::Uuid::new_v4()));
        let a = super::try_acquire(&d).unwrap().expect("first");
        assert!(super::try_acquire(&d).unwrap().is_none());
        drop(a);
        assert!(super::try_acquire(&d).unwrap().is_some());
        let _ = std::fs::remove_dir_all(d);
    }
}
