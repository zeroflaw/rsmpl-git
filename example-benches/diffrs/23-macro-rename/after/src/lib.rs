fn render() {
    new_render_widget!(header_title_text);
    new_render_widget!(sidebar_panel_contents, expanded_view_flag);
    new_render_widget!(footer_status_line + pending_suffix);
    new_render_widget!(main_content_area);
    new_render_widget!(toolbar_button_row, compact_layout_mode);
}
