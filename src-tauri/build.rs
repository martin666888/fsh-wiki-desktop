fn main() {
    let commands = &[
        "create_tab",
        "close_tab",
        "activate_tab",
        "list_tabs",
        "set_split",
        "choose_split_tab",
        "create_split_page",
        "focus_pane",
        "set_split_ratio",
        "reload_tab",
        "report_workspace_error",
        "capture_tab_preview",
        "begin_split_resize",
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
