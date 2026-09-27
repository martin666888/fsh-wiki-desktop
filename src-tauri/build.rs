fn main() {
    let commands = &[
        "create_tab",
        "close_tab",
        "activate_tab",
        "list_tabs",
        "list_fonts",
        "get_font_config",
        "set_font_config",
        "app_version",
        "open_settings",
        "take_settings_tab",
        "get_update_status",
        "get_update_prefs",
        "set_update_prefs",
        "check_update",
        "download_update",
        "install_update",
        "cancel_update",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("failed to build application permissions");
}
