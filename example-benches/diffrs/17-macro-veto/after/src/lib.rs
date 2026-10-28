fn render(path: &str) -> String {
    let a = format!(
        "{header}\nrest\n",
        header = super::git_header(path)
    );
    let b = format!(
        "{header}\nrest\n",
        header = super::git_header(path)
    );
    let c = format!(
        "{header}\nrest\n",
        header = super::git_header(path)
    );
    format!("{a}{b}{c}")
}
