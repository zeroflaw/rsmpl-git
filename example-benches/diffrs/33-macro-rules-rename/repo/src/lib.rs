macro_rules! old_widget {
    ($x:expr) => {
        $x + 1
    };
}

fn render() {
    let _a = old_widget!(header_title_text);
    let _b = old_widget!(sidebar_panel_contents, expanded_view_flag);
    let _c = old_widget! { footer_status_line };
    let _d = old_widget!(toolbar_button_row, compact_layout_mode);
    let _e = other_widget!(untouched_distractor);
}
