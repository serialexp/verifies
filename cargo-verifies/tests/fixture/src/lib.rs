pub fn double(x: i32) -> i32 {
    x * 2
}

#[cfg(test)]
mod tests {
    use verifies::verifies;

    #[verifies("REQ-1.1")]
    #[test]
    fn doubles() {
        assert_eq!(super::double(2), 4);
    }

    #[verifies("REQ-1.1")]
    #[verifies("REQ-1.2", "REQ-2")]
    #[test]
    fn stacked() {}

    #[test]
    #[verifies("REQ-3")]
    #[ignore = "not run by default"]
    fn ignored() {}

    #[test]
    fn untraced() {}

    mod nested {
        #[verifies::verifies("REQ-4")]
        #[test]
        fn deep() {}
    }
}
