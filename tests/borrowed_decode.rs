use std::{collections::BTreeMap, fmt};

use serde::{de, Deserialize};
use sonic_rs::{from_slice_borrowed, BorrowedRaw};

#[derive(Debug, Deserialize)]
struct Tokens<'a> {
    #[serde(borrow)]
    first: &'a str,
    #[serde(borrow)]
    second: &'a str,
}

#[test]
fn decoded_borrows_stay_stable_and_plain_text_borrows_input() {
    let input = br#"{"first":"a\u0062\n\ud83d\ude04","second":"plain"}"#;
    let mut arena = vec![0; input.len()];
    let arena_start = arena.as_ptr() as usize;
    let result: Tokens<'_> = from_slice_borrowed(input, &mut arena).unwrap();
    assert_eq!(result.first, "ab\n😄");
    assert_eq!(result.second, "plain");
    assert!((arena_start..arena_start + input.len()).contains(&(result.first.as_ptr() as usize)));
    assert!(
        (input.as_ptr() as usize..input.as_ptr() as usize + input.len())
            .contains(&(result.second.as_ptr() as usize))
    );
}

#[test]
fn escaped_raw_spans_keep_exact_input_and_decode_only_on_request() {
    let input = br#""a\u0062""#;
    let mut arena = vec![0; input.len()];
    let raw: BorrowedRaw<'_> = from_slice_borrowed(input, &mut arena).unwrap();
    assert_eq!(raw.as_json().as_bytes(), input);
    assert_eq!(raw.as_json().as_ptr(), input.as_ptr());
    assert!(!format!("{raw:?}").contains("u0062"));
    let mut second = vec![0; input.len()];
    let decoded: &str = from_slice_borrowed(raw.as_json().as_bytes(), &mut second).unwrap();
    assert_eq!(decoded, "ab");
    let nested = br#"{"a\u0062":["x\ny",123456789012345678901234567890]}"#;
    let mut arena = vec![0; nested.len()];
    let raw: BorrowedRaw<'_> = from_slice_borrowed(nested, &mut arena).unwrap();
    assert_eq!(raw.as_json().as_bytes(), nested);
    assert_eq!(raw.as_json().as_ptr(), nested.as_ptr());
}

#[test]
fn numeric_map_keys_keep_direct_lexeme_refusals() {
    for input in [
        br#"{"\u0031":0}"#.as_slice(),
        br#"{" 1":0}"#,
        br#"{"1\u0032":0}"#,
    ] {
        let mut arena = vec![0; input.len()];
        assert!(from_slice_borrowed::<BTreeMap<u64, u8>>(input, &mut arena).is_err());
        assert!(from_slice_borrowed::<BTreeMap<u128, u8>>(input, &mut arena).is_err());
        assert!(sonic_rs::from_slice::<BTreeMap<u64, u8>>(input).is_err());
        assert!(sonic_rs::from_slice::<BTreeMap<u128, u8>>(input).is_err());
    }
    let input = br#"{"340282366920938463463374607431768211455":0}"#;
    let mut arena = vec![0; input.len()];
    let got: BTreeMap<u128, u8> = from_slice_borrowed(input, &mut arena).unwrap();
    assert_eq!(got.get(&u128::MAX), Some(&0));
}

#[test]
fn integer128_uses_the_full_range_and_refuses_overflow() {
    for input in [i128::MIN.to_string(), i128::MAX.to_string()] {
        let mut arena = vec![0; input.len()];
        assert_eq!(
            from_slice_borrowed::<i128>(input.as_bytes(), &mut arena).unwrap(),
            input.parse::<i128>().unwrap()
        );
    }
    for input in ["340282366920938463463374607431768211456", "-1", "01", "1.0"] {
        let mut arena = vec![0; input.len()];
        assert!(from_slice_borrowed::<u128>(input.as_bytes(), &mut arena).is_err());
    }
}

#[test]
fn wrong_types_malformed_tail_and_invalid_unicode_refuse() {
    for input in [
        br#""secret\u0061""#.as_slice(),
        br#""secret\q""#,
        br#""secret\ud800""#,
    ] {
        let mut arena = vec![0; input.len()];
        let error = from_slice_borrowed::<u32>(input, &mut arena).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("secret"));
    }
    for input in [
        br#"{"first":"ok","second":"x"} trailing"#.as_slice(),
        br#"{"first":"ok","first":null,"second":"x"}"#,
        br#"{"first":"ok","second":"x","unknown":"bad\q"}"#,
        b"{\"first\":\"ok\",\"second\":\"\xff\"}",
    ] {
        let mut arena = vec![0; input.len()];
        assert!(from_slice_borrowed::<Tokens<'_>>(input, &mut arena).is_err());
    }
}

#[test]
fn native_owned_and_lazy_protocols_are_refused_before_dispatch() {
    let input = br#"{"secret":"value\u0061"}"#;
    let mut arena = vec![0; input.len()];
    assert!(from_slice_borrowed::<sonic_rs::Value>(input, &mut arena).is_err());
    assert!(from_slice_borrowed::<sonic_rs::OwnedLazyValue>(input, &mut arena).is_err());
    assert!(from_slice_borrowed::<sonic_rs::LazyValue<'_>>(input, &mut arena).is_err());
    assert!(from_slice_borrowed::<sonic_rs::RawNumber>(b"123", &mut arena).is_err());
    assert!(sonic_rs::from_slice::<BorrowedRaw<'_>>(input).is_err());
}

struct NeverFormat;
impl fmt::Display for NeverFormat {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("borrowed errors must not format supplied text")
    }
}

impl<'de> Deserialize<'de> for NeverFormat {
    fn deserialize<D: de::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(de::Error::custom(NeverFormat))
    }
}

#[test]
fn custom_error_does_not_format_and_small_arena_is_refused_untouched() {
    let mut arena = [9; 8];
    assert!(from_slice_borrowed::<NeverFormat>(b"null", &mut arena).is_err());
    let mut small = [9; 2];
    assert!(from_slice_borrowed::<&str>(br#""abc""#, &mut small).is_err());
    assert_eq!(small, [9; 2]);
    let mut exact = [0; 2];
    let empty: &str = from_slice_borrowed(br#""""#, &mut exact).unwrap();
    assert!(empty.is_empty());
}
