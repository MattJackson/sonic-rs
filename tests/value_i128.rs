//! 128-bit integers through `Value`, exactly (`arbitrary_precision`): `to_value` keeps one beyond
//! 64 bits as a raw number, as the `Value` builder keeps one it is handed, and `from_value` reads
//! it back from the lexeme. Nothing is rounded through binary64; what has no exact reading is
//! refused.
#![cfg(feature = "arbitrary_precision")]

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Wide {
    signed: i128,
    unsigned: u128,
}

/// Every 128-bit integer, inside and beyond the 64-bit ranges, survives `to_value` and
/// `from_value`, and the `Value` writes the integer's own digits.
#[test]
fn wide_integers_round_trip_through_value_exactly() {
    for (signed, unsigned) in [
        (i128::MIN, u128::MAX),
        (i128::MAX, u128::from(u64::MAX) + 1),
        (i128::from(i64::MIN) - 1, u128::MAX - 1),
        (i128::from(u64::MAX) + 1, 0),
        (-1, 1),
    ] {
        let wide = Wide { signed, unsigned };
        let value = sonic_rs::to_value(&wide).unwrap();
        assert_eq!(
            sonic_rs::to_string(&value).unwrap(),
            format!(r#"{{"signed":{signed},"unsigned":{unsigned}}}"#)
        );
        assert_eq!(sonic_rs::from_value::<Wide>(&value).unwrap(), wide);
    }
}

/// A parsed document's wide integer reads into its 128-bit field exactly.
#[test]
fn a_parsed_wide_integer_reads_exactly() {
    let text = format!(r#"{{"signed":{},"unsigned":{}}}"#, i128::MIN, u128::MAX);
    let value: sonic_rs::Value = sonic_rs::from_str(&text).unwrap();
    assert_eq!(
        sonic_rs::from_value::<Wide>(&value).unwrap(),
        Wide {
            signed: i128::MIN,
            unsigned: u128::MAX
        }
    );
}

/// A number that is not exactly a 128-bit integer of the field's type (beyond its range, a
/// fraction, an exponent, a sign `u128` cannot hold) is refused, never rounded.
#[test]
fn a_number_without_an_exact_128_bit_reading_is_refused() {
    let beyond = format!("{}", u128::MAX);
    for text in [
        beyond.as_str(),
        "170141183460469231731687303715884105728",
        "1.5",
        "1e3",
    ] {
        let value: sonic_rs::Value = sonic_rs::from_str(text).unwrap();
        assert!(
            sonic_rs::from_value::<i128>(&value).is_err(),
            "i128 from {text}"
        );
    }
    let beyond = format!("{}0", u128::MAX);
    for text in [beyond.as_str(), "-1", "1.5", "1e3"] {
        let value: sonic_rs::Value = sonic_rs::from_str(text).unwrap();
        assert!(
            sonic_rs::from_value::<u128>(&value).is_err(),
            "u128 from {text}"
        );
    }
}
