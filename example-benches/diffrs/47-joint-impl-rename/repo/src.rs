trait PrintableWidget {
    fn render(&self);
}
struct OldWidget;
impl PrintableWidget for OldWidget {
    fn render(&self) {}
}
