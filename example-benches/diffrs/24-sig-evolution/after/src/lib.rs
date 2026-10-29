struct Alpha;
struct Beta;
struct Gamma;

impl Alpha {
    fn execute(&self, x: i32) -> u64 {
        x + 1
    }
}

impl Beta {
    fn execute(&self, y: String) -> u64 {
        y
    }
}

impl Gamma {
    fn execute(&self, z: u8) -> u64 {
        z
    }
}

fn untouched() {}
