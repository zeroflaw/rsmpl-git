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

trait Labeled {
    fn label(&self) -> String;
}

struct Foo;

impl Labeled for Foo {
    fn label(&self) -> String {
        "foo".to_string()
    }
}

impl Labeled for New {
    fn label(&self) -> String {
        "old".to_string()
    }
}
