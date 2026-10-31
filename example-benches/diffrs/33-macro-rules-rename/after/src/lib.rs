macro_rules! new_widget {
    ($x:expr) => {
        $x + 1
    };
}

fn render() {
    let _a = new_widget!(header_title_text);
    let _b = new_widget!(sidebar_panel_contents, expanded_view_flag);
    let _c = new_widget! { footer_status_line };
    let _d = new_widget!(toolbar_button_row, compact_layout_mode);
    let _e = other_widget!(untouched_distractor);
}
