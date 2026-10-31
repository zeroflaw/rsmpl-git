trait T {
    fn old_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32;
}

trait U<T: Clone> {
    fn old_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> Result<i32, Error>;
}

trait V {
    fn old_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument, third_argument: ThirdLongTypeArgument) -> i32 {
        3
    }
}

impl Svc {
    fn old_name(&self) -> i32 {
        0
    }
}

fn old_name() -> i32 {
    1
}
