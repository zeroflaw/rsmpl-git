fn f() {
    let a = crate::new_api(1);
    let b = crate::new_api(x + 1);
    let c = crate::new_api(foo(y));
    let d = crate::new_api(4);
    let e = crate::new_api(z);
    let f = old_api(5);
    let g = other::old_api(6);
}
