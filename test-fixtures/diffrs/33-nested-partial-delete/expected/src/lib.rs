fn keep() {
    work()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stays() {
        let z = compute();
        assert_eq!(z, 3);
    }
}
