struct Alpha;
struct Beta;
struct Gamma;

impl Alpha {
    fn run(&self, input: i32) -> usize {
        let alpha_value: Box<[u32]> = compute_first(input);
        alpha_value.len()
    }
}

impl Beta {
    fn run(&self, other: i32) -> usize {
        let beta_value: Box<[String]> = compute_second(other);
        beta_value.len()
    }
}

impl Gamma {
    fn run(&self, extra: i32) -> usize {
        let gamma_value: Box<[u8]> = compute_third(extra);
        gamma_value.len()
    }
}

fn untouched() {
    let already: u32 = 1;
}
