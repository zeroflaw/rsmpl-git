struct Alpha;
struct Beta;
struct Gamma;

impl Alpha {
    fn execute(&self, first_argument: FirstLongTypeArgument) -> u64 {
        x + 1
    }
}

impl Beta {
    fn execute(&self, second_argument: SecondLongTypeArgument, third_argument: ThirdLongTypeArgument) -> u64 {
        y
    }
}

impl Gamma {
    fn execute(&self, fourth_argument: FourthLongTypeArgument, fifth_argument: FifthLongTypeArgument, sixth_argument: SixthLongTypeArgument) -> u64 {
        z
    }
}

fn untouched() {}
