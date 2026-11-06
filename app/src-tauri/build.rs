fn main() {
    // Tauri's default manifest, plus Windows 10 support so the title bar can layer a child
    // window over its maximize button.
    let windows = tauri_build::WindowsAttributes::new()
        .app_manifest(include_str!("windows-app-manifest.xml"));
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run the Tauri build script");
}
