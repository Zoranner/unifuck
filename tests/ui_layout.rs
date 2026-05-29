use std::fs;

#[test]
fn header_drag_area_is_limited_to_title_region() {
    let source = fs::read_to_string("ui/layouts/status-bar.slint").unwrap();
    let drag_area = source.split("drag-area := TouchArea").nth(1).unwrap();
    let drag_area = drag_area.split("// 标题").next().unwrap();

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
    let source = fs::read_to_string("ui/layouts/action-bar.slint").unwrap();

    assert!(source.contains("min-height: 44px;"));
    assert!(source.contains("max-height: 44px;"));
    assert!(source.contains("vertical-stretch: 0;"));
}
