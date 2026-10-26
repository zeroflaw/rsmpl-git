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

trait Printable {
    fn label(&self) -> String;
}

struct Foo;

impl Printable for Foo {
    fn label(&self) -> String {
        "foo".to_string()
    }
}

impl Printable for Old {
    fn label(&self) -> String {
        "old".to_string()
    }
}
