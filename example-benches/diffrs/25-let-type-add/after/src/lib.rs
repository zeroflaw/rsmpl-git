struct Alpha;
struct Beta;
struct Gamma;

impl Alpha {
    fn run(&self, input: i32) -> u32 {
        let alpha_value: u32 = compute_first(input);
        alpha_value as u32
    }
}

impl Beta {
    fn run(&self, other: i32) -> u32 {
        let beta_value: u32 = compute_second(other);
        beta_value as u32
    }
}

impl Gamma {
    fn run(&self, extra: i32) -> u32 {
        let gamma_value: u32 = compute_third(extra);
        gamma_value as u32
    }
}

fn untouched() {
    let already: u32 = 1;
}
