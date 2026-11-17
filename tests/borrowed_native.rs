use std::{cell::Cell, fmt};

use serde::{de, Deserialize};
use sonic_rs::{
    from_slice_borrowed, from_slice_borrowed_native, BorrowedRaw, JsonContainerTrait,
    JsonValueTrait, Value,
};

fn parity(input: &[u8]) {
    let ordinary = sonic_rs::from_slice::<Value>(input);
    let mut arena = vec![0; input.len()];
    let borrowed = from_slice_borrowed_native::<BorrowedRaw<'_>>(input, &mut arena);
    assert_eq!(
        ordinary.is_ok(),
        borrowed.is_ok(),
        "native acceptance differs for {input:?}"
    );
}

fn nested(depth: usize, mode: usize) -> String {
    let mut input = String::new();
    for i in 0..depth {
        input.push_str(if mode == 0 || (mode == 2 && i % 2 == 0) {
            "["
        } else {
            "{\"unused\":"
        });
    }
    input.push_str("null");
    for i in (0..depth).rev() {
        input.push(if mode == 0 || (mode == 2 && i % 2 == 0) {
            ']'
        } else {
            '}'
        });
    }
    input
}

#[test]
fn native_gate_keeps_scalar_container_and_complete_document_acceptance() {
    for input in [
        "null",
        "true",
        "false",
        "0",
        "\"a\\u0062\"",
        " \n {}\t ",
        "[]",
        "[null,1,{\"a\":true}]",
        "{\"token\":\"first\",\"token\":\"second\"}",
        "{\"token\":null,\"to\\u006ben\":false}",
        "",
        " ",
        "null null",
        "{}x",
        "[1,]",
        "{\"a\":1,}",
        "{\"token\":\"ok\",\"late\":}",
        "\"unfinished",
        "{\"a\":null",
        "[",
    ] {
        parity(input.as_bytes());
    }
}

#[test]
fn native_gate_uses_native_depth_for_all_values_including_unselected_members() {
    // Run on the normal harness stack. The native stack fix is a CI prerequisite.
    for depth in [0, 1, 253, 254, 255, 256] {
        for mode in 0..3 {
            let input = nested(depth, mode);
            parity(input.as_bytes());
            assert_eq!(
                sonic_rs::from_slice::<Value>(input.as_bytes()).is_ok(),
                depth <= 254
            );
            let selected = format!("{{\"token\":\"ok\",\"unused\":{input}}}");
            parity(selected.as_bytes());
        }
    }
    // This separate API must not tighten the existing raw/ignored traversal.
    let input = nested(10_000, 0);
    let mut arena = vec![0; input.len()];
    assert!(from_slice_borrowed::<BorrowedRaw<'_>>(input.as_bytes(), &mut arena).is_ok());
    assert!(from_slice_borrowed_native::<BorrowedRaw<'_>>(input.as_bytes(), &mut arena).is_err());
}

#[test]
fn native_gate_keeps_feature_specific_numeric_acceptance_in_unknown_fields() {
    for number in [
        "-9223372036854775808",
        "9223372036854775807",
        "18446744073709551615",
        "18446744073709551616",
        "340282366920938463463374607431768211456",
        "-0",
        "-0.0",
        "1e400",
        "1e-999",
        "3.14159265358979323846264338327950288419716939937510",
        "01",
        "1.",
        "1e",
        "1e+",
        "+1",
        "NaN",
        "Infinity",
    ] {
        parity(number.as_bytes());
        let unknown = format!("{{\"token\":\"ok\",\"unknown\":{number}}}");
        parity(unknown.as_bytes());
    }
    let mut arena = [0; 5];
    assert_eq!(
        from_slice_borrowed_native::<BorrowedRaw<'_>>(b"1e400", &mut arena).is_ok(),
        cfg!(feature = "arbitrary_precision")
    );
    parity("9".repeat(512).as_bytes());
}

#[test]
fn native_gate_keeps_strict_borrowed_unicode_contract() {
    for input in [
        br#""a\u0062\n\ud83d\ude04""#.as_slice(),
        br#"{"\u006b":"\"\\\/\b\f\n\r\t"}"#,
        "\"é😄\"".as_bytes(),
    ] {
        parity(input);
    }
    for input in [
        b"\"\xff\"".as_slice(),
        br#""\ud800""#,
        br#""\udc00""#,
        br#""\uZZZZ""#,
        br#""\u123""#,
        br#"{"token":"ok","late":"bad\q"}"#,
    ] {
        let mut arena = vec![0; input.len()];
        assert!(from_slice_borrowed_native::<BorrowedRaw<'_>>(input, &mut arena).is_err());
        #[cfg(not(feature = "utf8_lossy"))]
        parity(input);
    }
}

#[derive(Debug)]
struct First<'a> {
    members: usize,
    token: Option<BorrowedRaw<'a>>,
}
impl<'de> Deserialize<'de> for First<'de> {
    fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct FirstVisitor;
        impl<'de> de::Visitor<'de> for FirstVisitor {
            type Value = First<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut result = First {
                    members: 0,
                    token: None,
                };
                while let Some(key) = map.next_key::<&str>()? {
                    let value: BorrowedRaw<'de> = map.next_value()?;
                    result.members += 1;
                    if key == "token" && result.token.is_none() {
                        result.token = Some(value);
                    }
                }
                Ok(result)
            }
        }
        decoder.deserialize_map(FirstVisitor)
    }
}

#[test]
fn gated_projection_keeps_member_count_and_first_decoded_key_even_when_null_or_wrong_type() {
    for input in [
        br#"{"token":"first\u0061","token":"second"}"#.as_slice(),
        br#"{"token":null,"to\u006ben":"second"}"#,
        br#"{"to\u006ben":false,"token":"second"}"#,
        br#"{"token":{"ref":"a"},"token":null}"#,
        br#"{"token":"only"}"#,
        br#"{}"#,
    ] {
        let native: Value = sonic_rs::from_slice(input).unwrap();
        let mut arena = vec![0; input.len()];
        let got: First<'_> = from_slice_borrowed_native(input, &mut arena).unwrap();
        assert_eq!(got.members, native.as_object().unwrap().len());
        if let Some(raw) = got.token {
            assert_eq!(raw.kind(), native["token"].get_type());
            let original = raw.as_json().as_ptr() as usize;
            assert!(
                (input.as_ptr() as usize..input.as_ptr() as usize + input.len())
                    .contains(&original)
            );
            if let Some(expected) = native["token"].as_str() {
                let mut second = vec![0; raw.as_json().len()];
                let actual: &str =
                    from_slice_borrowed(raw.as_json().as_bytes(), &mut second).unwrap();
                assert_eq!(actual, expected);
            }
        } else {
            assert!(native["token"].is_null());
        }
    }
}

thread_local! { static PROJECTED: Cell<bool> = const { Cell::new(false) }; }
#[derive(Debug)]
struct RefuseProjection;
impl<'de> Deserialize<'de> for RefuseProjection {
    fn deserialize<D: de::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        PROJECTED.with(|flag| flag.set(true));
        Err(de::Error::custom("projection refused"))
    }
}

#[test]
fn complete_native_validation_precedes_any_projection_callback() {
    for input in [
        b"{\"token\":null,\"later\":}".as_slice(),
        b"{\"token\":null,\"later\":\"\xff\"}",
        b"{\"token\":null} trailing",
    ] {
        PROJECTED.with(|flag| flag.set(false));
        let mut arena = vec![0; input.len()];
        assert!(from_slice_borrowed_native::<RefuseProjection>(input, &mut arena).is_err());
        assert!(!PROJECTED.with(Cell::get));
    }
    let deep = format!("{{\"token\":null,\"later\":{}}}", nested(255, 0));
    PROJECTED.with(|flag| flag.set(false));
    let mut arena = vec![0; deep.len()];
    assert!(from_slice_borrowed_native::<RefuseProjection>(deep.as_bytes(), &mut arena).is_err());
    assert!(!PROJECTED.with(Cell::get));
    let mut arena = [0; 4];
    assert!(from_slice_borrowed_native::<RefuseProjection>(b"null", &mut arena).is_err());
    assert!(PROJECTED.with(Cell::get));
}

#[test]
fn native_pass_reuses_exact_guarded_arena_without_invalidating_returned_borrows() {
    let input = br#"["a\u0062","c\u0064"]"#;
    let mut arena = vec![0xa5; input.len() + 2];
    let base = arena.as_ptr() as usize + 1;
    let got: [&str; 2] = from_slice_borrowed_native(input, &mut arena[1..input.len() + 1]).unwrap();
    assert_eq!(got, ["ab", "cd"]);
    for value in got {
        assert!((base..base + input.len()).contains(&(value.as_ptr() as usize)));
    }
    assert_eq!(arena[0], 0xa5);
    assert_eq!(arena[input.len() + 1], 0xa5);
    let mut small = [0xa5; 1];
    assert!(from_slice_borrowed_native::<&str>(br#""a\u0062""#, &mut small).is_err());
    assert_eq!(small, [0xa5; 1]);
}
