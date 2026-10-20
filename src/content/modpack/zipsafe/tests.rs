//! Override path safety tests, using zips built in memory.
use super::*;
use std::io::{Cursor, Write};
use zip::{ZipWriter, write::SimpleFileOptions};

pub fn build(entries: &[(&str, &[u8])]) -> ZipArchive<Cursor<Vec<u8>>> {
    let mut w = ZipWriter::new(Cursor::new(vec![]));
    for (n, d) in entries { w.start_file(*n, SimpleFileOptions::default()).unwrap(); w.write_all(d).unwrap(); }
    ZipArchive::new(w.finish().unwrap()).unwrap()
}

fn tmpdir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("moss-zip-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn clean_rel_accepts_normal_paths_and_rejects_escapes() {
    assert_eq!(clean_rel("config/a/./b.toml").unwrap(), "config/a/b.toml");
    assert_eq!(clean_rel("mods//x.jar").unwrap(), "mods/x.jar");
    for bad in ["", "../x", "a/../../x", "/etc/passwd", "a\\b", "C:/x", "a\0b", ".moss/token", ".MOSS/x", "./.moss", "a/.."] { assert!(clean_rel(bad).is_err(), "{bad:?}"); }
    assert!(is_protected("server.jar") && is_protected("libraries/x/y.jar") && !is_protected("config/server.jar"));
}

#[test]
fn scan_orders_server_overrides_last_and_skips_protected_files() {
    let mut z = build(&[("overrides/config/a.toml", b"1"), ("server-overrides/config/a.toml", b"2"), ("overrides/server.jar", b"x"), ("overrides/readme.txt", b"r"), ("client-overrides/o.txt", b"c"), ("modrinth.index.json", b"{}")]);
    let (e, skipped) = scan_overrides(&mut z, "overrides").unwrap();
    assert_eq!(e.iter().map(|x| x.rel.as_str()).collect::<Vec<_>>(), ["config/a.toml", "readme.txt", "config/a.toml"]);
    assert_eq!(skipped, ["server.jar"]);
    let root = tmpdir();
    extract(&mut z, &e, &root).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("config/a.toml")).unwrap(), "2");
    assert!(!root.join("server.jar").exists() && !root.join("o.txt").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn traversal_absolute_and_moss_entries_reject_the_pack() {
    for name in ["overrides/../evil.txt", "overrides/a/../../evil.txt", "overrides//etc/passwd.txt", "overrides/.moss/x", "server-overrides/.moss/secret"] {
        let mut z = build(&[(name, b"x")]);
        assert!(scan_overrides(&mut z, "overrides").is_err(), "{name}");
    }
}

#[test]
fn symlink_entries_are_refused() {
    let mut w = ZipWriter::new(Cursor::new(vec![]));
    w.add_symlink("overrides/config/link", "/etc", SimpleFileOptions::default()).unwrap();
    let mut z = ZipArchive::new(w.finish().unwrap()).unwrap();
    assert!(scan_overrides(&mut z, "overrides").unwrap_err().to_string().contains("symbolic link"));
}

#[test]
fn extraction_refuses_to_write_through_existing_symlinks() {
    let root = tmpdir();
    let outside = tmpdir();
    std::os::unix::fs::symlink(&outside, root.join("config")).unwrap();
    let mut z = build(&[("overrides/config/pwn.txt", b"x")]);
    let (e, _) = scan_overrides(&mut z, "overrides").unwrap();
    assert!(extract(&mut z, &e, &root).is_err());
    assert!(!outside.join("pwn.txt").exists());
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn entry_count_and_size_caps_apply() {
    let mut z = build(&[("overrides/a.txt", b"hello")]);
    let (mut e, _) = scan_overrides(&mut z, "overrides").unwrap();
    assert_eq!(e[0].size, 5);
    // a header that under-reports the size must not be trusted during extraction
    e[0].size = 2;
    let root = tmpdir();
    assert!(extract(&mut z, &e, &root).unwrap_err().to_string().contains("larger than its header"));
    assert!(!root.join("a.txt").exists());
    let _ = std::fs::remove_dir_all(&root);
}
