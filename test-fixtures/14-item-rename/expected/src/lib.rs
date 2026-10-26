struct New {
    x: i32,
}

enum Form {
    Circle,
    Square,
}

const MAX: usize = 10;

mod core {
    pub struct Aid {
        v: u8,
    }
}

impl New {
    fn get(&self) -> i32 {
        self.x
    }
}
