fn a(p: bool, v: i32) -> i32 {
    if p {
        log!("hit");
        v
    } else {
        0
    }
}

fn b(q: bool, w: i32) -> i32 {
    if q {
        log!("hit");
        w
    } else {
        0
    }
}

fn c(r: bool, x: i32) -> i32 {
    if r {
        log!("hit");
        x
    } else {
        0
    }
}

fn d(s: bool, y: i32) -> i32 {
    if s {
        log!("hit");
        y
    } else {
        0
    }
}
