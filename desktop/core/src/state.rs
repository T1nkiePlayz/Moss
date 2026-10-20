//! The record of a running controller (`controller.json` in the app data directory), so a second launch, or a
//! relaunch after the window was closed and the app quit, reuses the controller instead of starting another.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controller {
    pub pid: u32,
    pub port: u16,
}

const FILE: &str = "controller.json";

pub fn read(dir: &Path) -> Option<Controller> {
    serde_json::from_slice(&std::fs::read(dir.join(FILE)).ok()?).ok()
}

pub fn write(dir: &Path, c: Controller) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&c)?)?;
    std::fs::rename(tmp, dir.join(FILE))
}

pub fn clear(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(FILE));
}

/// Is a process with this id running? (Permission denied still means it exists.)
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 { return false; }
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid as i32, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips_and_clears() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(read(d.path()), None);
        write(d.path(), Controller { pid: 42, port: 5000 }).unwrap();
        assert_eq!(read(d.path()), Some(Controller { pid: 42, port: 5000 }));
        clear(d.path());
        assert_eq!(read(d.path()), None);
    }
    #[test]
    fn garbage_is_ignored() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(FILE), b"not json").unwrap();
        assert_eq!(read(d.path()), None);
    }
    #[test]
    fn own_pid_is_alive_and_zero_is_not() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
    }
}
