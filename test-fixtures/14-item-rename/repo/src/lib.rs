struct Old {
    x: i32,
}

enum Shape {
    Circle,
    Square,
}

const LIMIT: usize = 10;

mod inner {
    pub struct Helper {
        v: u8,
    }
}

impl Old {
    fn get(&self) -> i32 {
        self.x
    }
}
