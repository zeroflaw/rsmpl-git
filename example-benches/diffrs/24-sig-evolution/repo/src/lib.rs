struct Alpha;
struct Beta;
struct Gamma;

impl Alpha {
    fn run(&self, x: i32) -> i32 {
        x + 1
    }
}

impl Beta {
    fn run(&self, y: String) -> String {
        y
    }
}

impl Gamma {
    fn run(&self, z: u8) -> u8 {
        z
    }
}

fn untouched() {}
