fn keep() {
    work()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first() {
        let x = compute();
        assert_eq!(x, 1);
    }

    #[test]
    fn second() {
        let y = compute();
        assert_eq!(y, 2);
    }

    #[test]
    fn stays() {
        let z = compute();
        assert_eq!(z, 3);
    }
}
