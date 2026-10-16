use verifies::verifies;

#[verifies("REQ-5")]
#[tokio::test]
async fn tokio_below() {
    helper();
}

#[tokio::test]
#[verifies("REQ-6")]
async fn tokio_above() {}

#[verifies("REQ-7")]
#[rstest::rstest]
#[case(1)]
#[case(2)]
fn rstest_cases(#[case] n: i32) {
    assert!(n > 0);
}

#[verifies("REQ-8")]
#[test_case::test_case(1 ; "one")]
#[test_case::test_case(2 ; "two")]
fn test_cases(n: i32) {
    assert!(n > 0);
}

#[test_case::test_case(1 ; "one")]
#[verifies("REQ-12")]
fn test_cases_below(n: i32) {
    assert!(n > 0);
}

#[verifies("REQ-9")]
#[test]
fn filtered_out() {}

#[verifies("REQ-10")]
#[test]
fn fails() {
    panic!("this test fails on purpose");
}

/// Not a test: cargo-verifies warns about it and links nothing.
#[verifies("REQ-11")]
fn helper() {}
