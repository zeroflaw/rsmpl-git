fn f() {
    let a = crate::old_api(1);
    let b = crate::old_api(x + 1);
    let c = crate::old_api(foo(y));
    let d = crate::old_api(4);
    let e = crate::old_api(z);
    let f = old_api(5);
    let g = other::old_api(6);
}
