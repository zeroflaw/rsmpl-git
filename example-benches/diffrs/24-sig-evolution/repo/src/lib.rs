struct Alpha;
struct Beta;
struct Gamma;

impl Alpha {
    fn run(&self, first_argument: FirstLongTypeArgument) -> i32 {
        x + 1
    }
}

impl Beta {
    fn run(&self, second_argument: SecondLongTypeArgument, third_argument: ThirdLongTypeArgument) -> String {
        y
    }
}

impl Gamma {
    fn run(&self, fourth_argument: FourthLongTypeArgument, fifth_argument: FifthLongTypeArgument, sixth_argument: SixthLongTypeArgument) -> u8 {
        z
    }
}

fn untouched() {}
