fn render(path: &str) -> String {
    let a = format!(
        "{header}\nrest\n",
        header = rsmpl_patch::collapse_git_header(&format!("diff --git a/{path} b/{path}"))
    );
    let b = format!(
        "{header}\nrest\n",
        header = rsmpl_patch::collapse_git_header(&format!("diff --git a/{path} b/{path}"))
    );
    let c = format!(
        "{header}\nrest\n",
        header = rsmpl_patch::collapse_git_header(&format!("diff --git a/{path} b/{path}"))
    );
    format!("{a}{b}{c}")
}
