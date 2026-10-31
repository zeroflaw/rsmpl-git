trait Greeter {
    fn greet(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> String;
}

struct A;
impl Greeter for A {
    fn greet(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> String {
        "a".to_string()
    }
}

struct B;
impl Greeter for B {
    fn greet(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> String {
        "b".to_string()
    }
}

fn greet() -> i32 {
    0
}
