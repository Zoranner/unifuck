use std::fs;

#[test]
fn application_icon_is_wired_to_window_and_executable() {
    let app_window = fs::read_to_string("ui/app-window.slint").unwrap();
    let status_bar = fs::read_to_string("ui/layouts/header-bar.slint").unwrap();
    let build_script = fs::read_to_string("build.rs").unwrap();
    let manifest = fs::read_to_string("Cargo.toml").unwrap();

    assert!(app_window.contains("icon: @image-url(\"assets/app-icon.png\");"));
    assert!(app_window.contains("border-color: Theme.magenta-50;"));
    assert!(app_window.contains("背景扫描线"));
    assert!(app_window.contains("dlg-glitch"));
    assert!(status_bar.contains("source: @image-url(\"../assets/app-icon.png\");"));
    assert!(status_bar.contains("y: (parent.height - self.height) / 2;"));
    assert!(status_bar.contains("width: 20px;"));
    assert!(!status_bar.contains("background: #f5f0ff;"));
    assert!(!status_bar.contains("border-color: Theme.magenta-50;"));
    assert!(!status_bar.contains("🔓 UNIFUCK"));
    assert!(build_script.contains("EmbedResourcesKind::EmbedFiles"));
    assert!(build_script.contains("ui/assets/app-icon.ico"));
    assert!(build_script.contains("winresource::WindowsResource::new()"));
    assert!(manifest.contains("winresource = \"0.1\""));
    assert!(fs::metadata("ui/assets/app-icon.png").is_ok());
    assert!(fs::metadata("ui/assets/app-icon.ico").is_ok());
    assert!(fs::metadata("ui/assets/app-icon.icns").is_ok());
}

#[test]
fn header_drag_area_is_limited_to_title_region() {
    let source = fs::read_to_string("ui/layouts/header-bar.slint").unwrap();
    let drag_area = source.split("drag-area := TouchArea").nth(1).unwrap();
    let drag_area = drag_area.split("// 标题").next().unwrap();

    assert!(source.contains("background: Theme.purple-bg-0;"));
    assert!(source.contains("color: #ffd400;"));
    assert!(source.contains("color: Theme.cyan-bright;"));
    assert!(source.contains("glitch-frame"));
    assert!(source.contains("color: Theme.magenta-50;"));
    assert!(source.contains("min-height: 30px;"));
    assert!(source.contains("max-height: 30px;"));
    assert!(source.contains("vertical-stretch: 0;"));
    assert!(drag_area.contains("width: parent.width - 72px;"));
    assert!(drag_area.contains("height: 30px;"));
    assert!(!drag_area.contains("width: 100%;"));
    assert!(!drag_area.contains("height: 100%;"));
}

#[test]
fn action_bar_does_not_expand_over_content() {
    let action_bar = fs::read_to_string("ui/layouts/action-bar.slint").unwrap();

    assert!(action_bar.contains("min-height: 44px;"));
    assert!(action_bar.contains("max-height: 44px;"));
    assert!(action_bar.contains("vertical-stretch: 0;"));
    assert!(action_bar.contains("working || !has-selection;"));
}
