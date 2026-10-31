struct A;
struct B<T>(T);
struct C;
struct D;

trait T {
    fn old_name(&self) -> i32;
}

impl A {
    fn new_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32 {
        1
    }
}

impl<T: Clone> B<T> {
    fn new_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> Result<i32, Error> {
        Ok(2)
    }
}

impl C {
    fn new_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument, third_argument: ThirdLongTypeArgument) -> Vec<i32> {
        vec![3]
    }
}

impl crate::T for D {
    fn new_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i64 {
        4
    }
}

fn old_name() -> i32 {
    0
}
