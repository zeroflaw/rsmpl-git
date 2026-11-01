trait LabeledWidget {
    fn render(&self);
}
struct NewWidget;
impl LabeledWidget for NewWidget {
    fn render(&self) {}
}
