fn main() {
    // Only the commands listed here exist as permissions; the default capability grants just `allow-retry`.
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&["retry"])),
    )
    .expect("failed to run tauri-build");
}
