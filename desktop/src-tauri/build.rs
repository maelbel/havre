// Tauri reads tauri.conf.json at build time. The list of our own commands
// also declares a permission for each ("allow-notify"…), which is what lets
// us give the server's page access to exactly those, and nothing else.
fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "connect",
            "desktop_info",
            "notify",
            "set_unread",
            "open_external",
        ]),
    ))
    .expect("failed to run the Tauri build script");
}
