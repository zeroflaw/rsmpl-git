fn render() {
    old_render_widget!(header_title_text);
    old_render_widget!(sidebar_panel_contents, expanded_view_flag);
    old_render_widget!(footer_status_line + pending_suffix);
    old_render_widget!(main_content_area);
    old_render_widget!(toolbar_button_row, compact_layout_mode);
}
