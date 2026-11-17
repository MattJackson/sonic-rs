//! `Value`, `Object` and `Array` read through deserializers that forward
//! `deserialize_newtype_struct` to `deserialize_any`, such as serde's own `de::value` ones.

use std::collections::BTreeMap;

use serde::{
    de::value::{
        BoolDeserializer, Error, F64Deserializer, I64Deserializer, MapDeserializer,
        SeqDeserializer, StrDeserializer, U64Deserializer, UnitDeserializer,
    },
    Deserialize,
};
use sonic_rs::{json, Array, JsonValueTrait, Object, Value};

#[test]
fn value_from_scalars() {
    let v = Value::deserialize(BoolDeserializer::<Error>::new(true)).unwrap();
    assert_eq!(v.as_bool(), Some(true));
    let v = Value::deserialize(I64Deserializer::<Error>::new(-7)).unwrap();
    assert_eq!(v.as_i64(), Some(-7));
    let v = Value::deserialize(U64Deserializer::<Error>::new(u64::MAX)).unwrap();
    assert_eq!(v.as_u64(), Some(u64::MAX));
    let v = Value::deserialize(F64Deserializer::<Error>::new(1.5)).unwrap();
    assert_eq!(v.as_f64(), Some(1.5));
    let v = Value::deserialize(StrDeserializer::<Error>::new("hi")).unwrap();
    assert_eq!(v.as_str(), Some("hi"));
    let v = Value::deserialize(UnitDeserializer::<Error>::new()).unwrap();
    assert!(v.is_null());
}

#[test]
fn value_from_containers() {
    let seq = SeqDeserializer::<_, Error>::new(vec![1u64, 2, 3].into_iter());
    assert_eq!(Value::deserialize(seq).unwrap(), json!([1, 2, 3]));

    let entries = BTreeMap::from([("a".to_string(), 1u64), ("b".to_string(), 2)]);
    let map = MapDeserializer::<_, Error>::new(entries.clone().into_iter());
    assert_eq!(Value::deserialize(map).unwrap(), json!({"a": 1, "b": 2}));

    let map = MapDeserializer::<_, Error>::new(entries.into_iter());
    let object = Object::deserialize(map).unwrap();
    assert_eq!(object.get(&"b").and_then(|v| v.as_u64()), Some(2));

    let seq = SeqDeserializer::<_, Error>::new(vec!["x", "y"].into_iter());
    let array = Array::deserialize(seq).unwrap();
    assert_eq!(array.len(), 2);
    assert_eq!(array[1].as_str(), Some("y"));
}

/// With `arbitrary_precision` and `Deserializer::use_rawnumber_in_any`, numbers keep their text
/// through serde's buffering (`untagged`, `flatten`) and through foreign deserializers.
#[cfg(feature = "arbitrary_precision")]
mod exact_numbers {
    use serde::{de::value::U128Deserializer, Serialize};
    use sonic_rs::JsonNumberTrait;

    use super::*;

    /// Deserializes `json` with numbers kept as text through `deserialize_any`.
    fn exact<'a, T: Deserialize<'a>>(json: &'a str) -> sonic_rs::Result<T> {
        let mut de = sonic_rs::Deserializer::from_str(json).use_rawnumber_in_any();
        let value = T::deserialize(&mut de)?;
        de.end()?;
        Ok(value)
    }

    #[derive(Deserialize, Serialize)]
    #[serde(untagged)]
    enum Untagged {
        Object { n: Value, c: Value, d: Value },
    }

    #[derive(Deserialize)]
    struct Flattened {
        id: u32,
        #[serde(flatten)]
        rest: Value,
    }

    /// Numbers beyond binary64 survive buffering, byte for byte.
    #[test]
    fn untagged_and_flatten_keep_the_text() {
        let numbers =
            r#""n":18446744073709551617,"c":12345678901.1234567,"d":0.1000000000000000000001"#;

        let input = format!("{{{numbers}}}");
        let parsed: Untagged = exact(&input).unwrap();
        assert_eq!(sonic_rs::to_string(&parsed).unwrap(), input);

        let input = format!(r#"{{"id":7,{numbers}}}"#);
        let parsed: Flattened = exact(&input).unwrap();
        assert_eq!(parsed.id, 7);
        // the order of the members is the input's, or sorted with `sort_keys`: check each
        let out = sonic_rs::to_string(&parsed.rest).unwrap();
        for member in numbers.split(',') {
            assert!(out.contains(member), "{member} in {out}");
        }
        assert_eq!(out.len(), numbers.len() + 2);
    }

    #[derive(Debug, PartialEq, Deserialize)]
    #[serde(untagged)]
    enum Scalar {
        Int(u64),
        Float(f64),
    }

    #[derive(Debug, PartialEq, Deserialize)]
    #[serde(tag = "type")]
    enum Tagged {
        A { id: u64 },
    }

    /// Without the opt-in, the feature changes nothing for types that read numbers through
    /// `deserialize_any`.
    #[test]
    fn default_deserializer_is_unchanged() {
        let scalars: Vec<Scalar> = sonic_rs::from_str("[1,2.5]").unwrap();
        assert_eq!(scalars, [Scalar::Int(1), Scalar::Float(2.5)]);
        let tagged: Tagged = sonic_rs::from_str(r#"{"type":"A","id":3}"#).unwrap();
        assert_eq!(tagged, Tagged::A { id: 3 });
        let foreign: serde_json::Value = sonic_rs::from_str(r#"{"a":[1,2.5]}"#).unwrap();
        assert_eq!(foreign, serde_json::json!({"a": [1, 2.5]}));
    }

    /// A key merely spelled like the number token is an ordinary key, from a foreign deserializer
    /// or from the document itself.
    #[test]
    fn token_spelled_key_stays_a_key() {
        let token = "$sonic_rs::private::Number";
        let entries = vec![(token.to_string(), "abc".to_string())];
        let value =
            Value::deserialize(MapDeserializer::<_, Error>::new(entries.into_iter())).unwrap();
        assert!(value.is_object());
        assert_eq!(value.get(token).and_then(|v| v.as_str()), Some("abc"));

        let parsed: Flattened = exact(r#"{"id":1,"$sonic_rs::private::Number":"5"}"#).unwrap();
        assert_eq!(parsed.rest.get(token).and_then(|v| v.as_str()), Some("5"));
    }

    #[test]
    fn u128_is_exact() {
        let value = Value::deserialize(U128Deserializer::<Error>::new(u128::MAX)).unwrap();
        assert_eq!(
            value.as_raw_number().unwrap().as_str(),
            "340282366920938463463374607431768211455"
        );
    }

    /// `Number` reads the token map, in binary64.
    #[test]
    fn number_reads_the_token_map() {
        let n: sonic_rs::Number = exact("12").unwrap();
        assert_eq!(n.as_u64(), Some(12));
        let n: sonic_rs::Number = exact("1.5").unwrap();
        assert_eq!(n.as_f64(), Some(1.5));
    }
}
