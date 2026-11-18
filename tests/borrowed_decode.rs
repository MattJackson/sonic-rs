use std::{collections::BTreeMap, fmt};

use serde::{de, Deserialize};
use sonic_rs::{from_slice_borrowed, BorrowedError, BorrowedRaw};

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

#[derive(Debug)]
struct BorrowedBytes<'a>(&'a [u8]);
struct BytesVisitor;
impl<'de> de::Visitor<'de> for BytesVisitor {
    type Value = BorrowedBytes<'de>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("borrowed bytes")
    }
    fn visit_borrowed_bytes<E: de::Error>(self, value: &'de [u8]) -> Result<Self::Value, E> {
        Ok(BorrowedBytes(value))
    }
}
impl<'de> Deserialize<'de> for BorrowedBytes<'de> {
    fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        decoder.deserialize_bytes(BytesVisitor)
    }
}

#[test]
fn byte_visitors_receive_stable_strict_utf8_borrows() {
    let input = br#""a\u0062""#;
    let mut arena = vec![0; input.len()];
    let base = arena.as_ptr() as usize;
    let bytes: BorrowedBytes<'_> = from_slice_borrowed(input, &mut arena).unwrap();
    assert_eq!(bytes.0, b"ab");
    assert!((base..base + input.len()).contains(&(bytes.0.as_ptr() as usize)));
    for input in [b"\"\xff\"".as_slice(), br#""\ud800""#, br#""\udc00""#] {
        let mut arena = vec![0; input.len()];
        assert!(from_slice_borrowed::<BorrowedBytes<'_>>(input, &mut arena).is_err());
    }
}

struct NeverExpected;
impl de::Expected for NeverExpected {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("expected text must not be formatted")
    }
}

#[test]
fn every_serde_error_constructor_is_static() {
    let errors = [
        <BorrowedError as de::Error>::custom(NeverFormat),
        de::Error::invalid_type(de::Unexpected::Str("secret-payload"), &NeverExpected),
        de::Error::invalid_value(de::Unexpected::Bytes(b"secret-payload"), &NeverExpected),
        de::Error::invalid_length(7, &NeverExpected),
        de::Error::unknown_variant("secret-payload", &[]),
        de::Error::unknown_field("secret-payload", &[]),
        de::Error::missing_field("secret-payload"),
        de::Error::duplicate_field("secret-payload"),
    ];
    for error in errors {
        assert!(!format!("{error:?} {error}").contains("secret-payload"));
    }
}

#[test]
fn raw_fields_support_shorter_borrowed_lifetimes_and_checked_deep_values() {
    #[derive(Deserialize)]
    struct Envelope<'a> {
        #[serde(borrow)]
        raw: BorrowedRaw<'a>,
    }
    let nested = format!(
        "{{\"raw\":{}\"a\\u0062\"{}}}",
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    let mut arena = vec![0; nested.len()];
    let envelope: Envelope<'_> = from_slice_borrowed(nested.as_bytes(), &mut arena).unwrap();
    assert_eq!(envelope.raw.as_json().len(), 20_009);
    assert!(envelope.raw.as_json().contains(r#""a\u0062""#));
    // A malformed ignored/raw value must not succeed just because no string was requested.
    let invalid = br#"{"raw":{"unused":"bad\q"}}"#;
    let mut arena = vec![0; invalid.len()];
    assert!(from_slice_borrowed::<Envelope<'_>>(invalid, &mut arena).is_err());
}

#[test]
fn originating_parser_errors_keep_category_and_position_through_nested_visitors() {
    let invalid = b"{\n\"first\":\"ok\",\"second\":\"bad\\q\"}";
    let mut arena = vec![0; invalid.len()];
    let error = from_slice_borrowed::<Tokens<'_>>(invalid, &mut arena).unwrap_err();
    assert!(matches!(
        error.classify(),
        sonic_rs::error::Category::Syntax
    ));
    assert_eq!(error.line(), 2);
    assert!(error.offset() > 20);
    let incomplete = br#"{"first":"ok","second":"unfinished"#;
    let mut arena = vec![0; incomplete.len()];
    let error = from_slice_borrowed::<Tokens<'_>>(incomplete, &mut arena).unwrap_err();
    assert!(matches!(error.classify(), sonic_rs::error::Category::Eof));
}

#[test]
fn every_escape_and_multiple_arena_regions_keep_guards_and_earlier_borrows() {
    let input = br#"["\"\\\/\b\f\n\r\t\u0000","01234567890123456789012345678901\u0061","\u00e9"]"#;
    let mut guarded = vec![0xa5; input.len() + 2];
    let base = guarded.as_ptr() as usize + 1;
    let values: [&str; 3] = from_slice_borrowed(input, &mut guarded[1..input.len() + 1]).unwrap();
    assert_eq!(
        values,
        [
            "\"\\/\u{8}\u{c}\n\r\t\0",
            "01234567890123456789012345678901a",
            "é"
        ]
    );
    for value in values {
        let start = value.as_ptr() as usize;
        assert!(start >= base && start + value.len() <= base + input.len());
    }
    assert_eq!(guarded[0], 0xa5);
    assert_eq!(guarded[input.len() + 1], 0xa5);
}

#[test]
fn ignored_fields_keep_checked_skip_acceptance_and_strict_escape_validation() {
    let nested = format!(
        "{{\"first\":\"ab\",\"second\":\"ok\",\"unused\":{}null{}}}",
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    assert!(sonic_rs::from_slice::<Tokens<'_>>(nested.as_bytes()).is_ok());
    let mut arena = vec![0; nested.len()];
    let got: Tokens<'_> = from_slice_borrowed(nested.as_bytes(), &mut arena).unwrap();
    assert_eq!(got.first, "ab");
    for tail in [r#""\uZZZZ""#, r#""\ud800""#, r#""\udc00""#, r#""\u123""#] {
        let input = format!("{{\"first\":\"ok\",\"second\":\"ok\",\"unused\":{tail}}}");
        let mut arena = vec![0; input.len()];
        assert!(from_slice_borrowed::<Tokens<'_>>(input.as_bytes(), &mut arena).is_err());
    }
}

#[cfg(feature = "arbitrary_precision")]
#[test]
fn arbitrary_precision_wrong_type_errors_do_not_copy_the_numeric_lexeme() {
    #[derive(Debug)]
    struct NumberMap;
    impl<'de> Deserialize<'de> for NumberMap {
        fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
            struct NumberMapVisitor;
            impl<'de> de::Visitor<'de> for NumberMapVisitor {
                type Value = NumberMap;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a map with boolean values")
                }
                fn visit_map<A: de::MapAccess<'de>>(
                    self,
                    mut access: A,
                ) -> Result<NumberMap, A::Error> {
                    while access.next_entry::<&str, bool>()?.is_some() {}
                    Ok(NumberMap)
                }
            }
            // deserialize_any selects the private number map; deserialize_map would
            // reject the numeric token before reaching its borrowed lexeme adapter.
            decoder.deserialize_any(NumberMapVisitor)
        }
    }
    let input = b"123456789012345678901234567890123456789012345678901234567890";
    let mut arena = vec![0; input.len()];
    let error = from_slice_borrowed::<NumberMap>(input, &mut arena).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("12345678901234567890"));
}
