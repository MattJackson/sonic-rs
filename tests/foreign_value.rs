//! `Value` read through foreign deserializers, with exact numbers (`arbitrary_precision`).
#![cfg(feature = "arbitrary_precision")]

use serde::{
    de::value::{BytesDeserializer, Error, F64Deserializer, MapDeserializer, U128Deserializer},
    Deserialize, Serialize,
};
use sonic_rs::{JsonNumberTrait, JsonValueTrait, Value};

const TOKEN_SPELLING: &str = "$sonic_rs::private::JsonNumber";

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Tagged {
    Object { n: Value, c: Value, d: Value },
}

#[derive(Deserialize)]
struct Flattened {
    id: u32,
    #[serde(flatten)]
    rest: Value,
}

/// Numbers beyond binary64 survive buffering (`untagged`, `flatten`), byte for byte.
#[test]
fn tagged_untagged_flatten_value_keeps_lexeme() {
    let numbers =
        r#""n":18446744073709551617,"c":12345678901.1234567,"d":0.1000000000000000000001"#;

    let input = format!("{{{numbers}}}");
    let parsed: Tagged = sonic_rs::from_str(&input).unwrap();
    assert_eq!(sonic_rs::to_string(&parsed).unwrap(), input);

    let input = format!(r#"{{"id":7,{numbers}}}"#);
    let parsed: Flattened = sonic_rs::from_str(&input).unwrap();
    assert_eq!(parsed.id, 7);
    assert_eq!(
        sonic_rs::to_string(&parsed.rest).unwrap(),
        format!("{{{numbers}}}")
    );
}

/// A key merely spelled like the number token is an ordinary key.
#[test]
fn foreign_token_spelled_key_stays_object() {
    let entries = vec![(TOKEN_SPELLING.to_string(), "abc".to_string())];
    let value = Value::deserialize(MapDeserializer::<_, Error>::new(entries.into_iter())).unwrap();
    assert!(value.is_object());
    assert_eq!(
        value.get(TOKEN_SPELLING).and_then(|v| v.as_str()),
        Some("abc")
    );
}

/// A foreign byte string the size of a `Value` is refused, never taken as a `Value`.
#[test]
fn foreign_bytes_never_transmuted() {
    let bytes = vec![0u8; std::mem::size_of::<Value>()];
    assert!(Value::deserialize(BytesDeserializer::<Error>::new(&bytes)).is_err());
    let short = [1u8, 2, 3];
    assert!(Value::deserialize(BytesDeserializer::<Error>::new(&short)).is_err());
}

#[test]
fn visit_f64_inf_refused() {
    for bad in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        assert!(Value::deserialize(F64Deserializer::<Error>::new(bad)).is_err());
    }
    let ok = Value::deserialize(F64Deserializer::<Error>::new(1.5)).unwrap();
    assert_eq!(ok.as_f64(), Some(1.5));
}

#[test]
fn foreign_u128_is_an_exact_number() {
    let value = Value::deserialize(U128Deserializer::<Error>::new(u128::MAX)).unwrap();
    assert_eq!(
        value.as_raw_number().unwrap().as_str(),
        "340282366920938463463374607431768211455"
    );
}

/// Numbers are equal when their text is.
#[test]
fn eq_by_lexeme() {
    let a: Value = sonic_rs::from_str("18446744073709551617").unwrap();
    let b: Value = sonic_rs::from_str("18446744073709551618").unwrap();
    assert_ne!(a, b);
    assert_eq!(a, a.clone());

    let c: Value = sonic_rs::from_str("0.1000000000000000000001").unwrap();
    let d: Value = sonic_rs::from_str("0.1000000000000000000002").unwrap();
    assert_ne!(c, d);

    let x: Value = sonic_rs::from_str(r#"{"k":[1e400,2]}"#).unwrap();
    let y: Value = sonic_rs::from_str(r#"{"k":[1e401,2]}"#).unwrap();
    assert_ne!(x, y);
    assert_eq!(x, x.clone());
}

/// `Number` still reads from `deserialize_any` (binary64).
#[test]
fn number_reads_through_the_token_map() {
    let n: sonic_rs::Number = sonic_rs::from_str("12").unwrap();
    assert_eq!(n.as_u64(), Some(12));
}
