//! With `arbitrary_precision`, `Value` numbers are equal when their text is.
#![cfg(feature = "arbitrary_precision")]

use sonic_rs::Value;

fn parse(json: &str) -> Value {
    sonic_rs::from_str(json).unwrap()
}

#[test]
fn numbers_beyond_binary64_compare_by_text() {
    let a = parse("18446744073709551617");
    assert_ne!(a, parse("18446744073709551618"));
    assert_eq!(a, a.clone());

    assert_ne!(
        parse("0.1000000000000000000001"),
        parse("0.1000000000000000000002")
    );

    // both overflow binary64: they used to compare equal
    let x = parse(r#"{"k":[1e400,2]}"#);
    assert_ne!(x, parse(r#"{"k":[1e401,2]}"#));
    assert_eq!(x, x.clone());
}

#[test]
fn the_same_number_written_differently_is_a_different_text() {
    assert_ne!(parse("1.0"), parse("1.00"));
    assert_eq!(parse("1.5"), sonic_rs::json!(1.5));
}
