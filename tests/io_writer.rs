//! The compact I/O writer preserves bytes and reports the first sink/formatting refusal.

use std::{cell::RefCell, collections::BTreeMap, fmt, io, rc::Rc};

use serde::{ser::SerializeMap, ser::SerializeStruct, Serialize, Serializer};
use sonic_rs::{format::Formatter, writer::BufferedWriter};

// The regression commit uses the old public entrypoint. The fix changes only this routing.
fn write_json<W: io::Write, T: Serialize + ?Sized>(writer: W, value: &T) -> sonic_rs::Result<()> {
    sonic_rs::to_writer(BufferedWriter::new(writer), value)
}

fn bytes<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    write_json(&mut out, value).unwrap();
    out
}

#[test]
fn compact_strings_characters_and_character_keys_keep_frozen_bytes() {
    assert_eq!(bytes(""), b"\"\"");
    assert_eq!(
        bytes("\0\u{8}\t\n\u{c}\r\"\\/\u{7f}é雪🦀"),
        "\"\\u0000\\b\\t\\n\\f\\r\\\"\\\\/\u{7f}é雪🦀\"".as_bytes()
    );
    for value in ['a', '\0', '"', '\\', 'é', '雪', '🦀'] {
        assert_eq!(bytes(&value), sonic_rs::to_vec(&value).unwrap());
    }
    let map = BTreeMap::from([('\0', 1), ('"', 2), ('\\', 3), ('雪', 4), ('🦀', 5)]);
    assert_eq!(
        bytes(&map),
        r#"{"\u0000":1,"\"":2,"\\":3,"雪":4,"🦀":5}"#.as_bytes()
    );
    let controls: String = (0..=31).map(char::from).collect();
    assert_eq!(bytes(&controls), sonic_rs::to_vec(&controls).unwrap());
    for text in ["plain-é雪🦀".repeat(8192), "\0\n\"\\é雪🦀".repeat(8192)] {
        assert_eq!(bytes(&text), sonic_rs::to_vec(&text).unwrap());
    }
}

#[derive(Serialize)]
enum Shape<'a> {
    Unit,
    Tuple(u64, &'a str),
    Record { label: &'a str, values: &'a [u8] },
}

#[test]
fn compact_shapes_and_numeric_lexemes_keep_existing_bytes() {
    for value in [
        Shape::Unit,
        Shape::Tuple(u64::MAX, "雪"),
        Shape::Record {
            label: "\0",
            values: &[0, 255],
        },
    ] {
        assert_eq!(bytes(&value), sonic_rs::to_vec(&value).unwrap());
    }
    assert_eq!(
        bytes(&i128::MIN),
        b"-170141183460469231731687303715884105728"
    );
    assert_eq!(
        bytes(&u128::MAX),
        b"340282366920938463463374607431768211455"
    );
    assert_eq!(
        bytes(&[f64::NAN, f64::INFINITY, f64::NEG_INFINITY]),
        b"[null,null,null]"
    );
    for number in [0.0, -0.0, 0.1, f64::MIN_POSITIVE, f64::MAX] {
        assert_eq!(bytes(&number), sonic_rs::to_vec(&number).unwrap());
    }
    for number in [0.1_f32, -0.0, f32::MIN_POSITIVE, f32::MAX] {
        assert_eq!(bytes(&number), sonic_rs::to_vec(&number).unwrap());
    }
    for text in ["-0", "1.0", "1e400", "18446744073709551617"] {
        let number: sonic_rs::RawNumber = sonic_rs::from_str(text).unwrap();
        assert_eq!(bytes(&number), text.as_bytes());
    }
}

struct Refusing {
    bytes: Vec<u8>,
    limit: usize,
    failed: bool,
    writes_after_error: usize,
}

impl Refusing {
    fn at(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            failed: false,
            writes_after_error: 0,
        }
    }
}

impl io::Write for Refusing {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.failed {
            self.writes_after_error += 1;
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let amount = bytes.len().min(self.limit - self.bytes.len());
        if amount == 0 && !bytes.is_empty() {
            self.failed = true;
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.bytes.extend_from_slice(&bytes[..amount]);
        Ok(amount)
    }

    fn flush(&mut self) -> io::Result<()> {
        panic!("serialization must not flush the caller's sink")
    }
}

#[test]
fn every_output_boundary_reports_the_original_io_error_and_exact_prefix() {
    #[derive(Serialize)]
    struct Input<'a> {
        a: &'a str,
        b: [u64; 2],
    }
    let value = Input {
        a: "x\n\"雪",
        b: [0, u64::MAX],
    };
    let expected = r#"{"a":"x\n\"雪","b":[0,18446744073709551615]}"#.as_bytes();
    for limit in 0..expected.len() {
        let mut sink = Refusing::at(limit);
        let error = write_json(&mut sink, &value).unwrap_err();
        assert_eq!(error.classify(), sonic_rs::error::Category::Io);
        assert_eq!(error.io_error_kind(), Some(io::ErrorKind::BrokenPipe));
        assert_eq!(sink.bytes, expected[..limit]);
        assert_eq!(sink.writes_after_error, 0);
    }
    let mut sink = Refusing::at(expected.len());
    write_json(&mut sink, &value).unwrap();
    assert_eq!(sink.bytes, expected);
    assert_eq!(bytes("next request"), b"\"next request\"");
}

#[derive(Clone, Copy)]
enum Ending {
    Propagate,
    RetryAndSucceed,
    SwallowAndSucceed,
    SwallowAndFail,
    BareFailure,
}

struct Text(Ending);
impl fmt::Display for Text {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if matches!(self.0, Ending::BareFailure) {
            return Err(fmt::Error);
        }
        let first = out.write_str("1");
        match self.0 {
            Ending::Propagate => {
                first?;
                out.write_str("2")
            }
            Ending::RetryAndSucceed => {
                let _ = out.write_str("2");
                Ok(())
            }
            Ending::SwallowAndSucceed => Ok(()),
            Ending::SwallowAndFail => Err(fmt::Error),
            Ending::BareFailure => unreachable!(),
        }
    }
}
impl Serialize for Text {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

struct Key(Text);
impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(&self.0, &0)?;
        map.end()
    }
}

struct Raw(Text);
impl Serialize for Raw {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut raw = serializer.serialize_struct("$sonic_rs::private::JsonNumber", 1)?;
        raw.serialize_field("$sonic_rs::private::JsonNumber", &self.0)?;
        raw.end()
    }
}

fn refusal<T: Serialize>(value: &T, prefix: &[u8]) {
    let mut sink = Refusing::at(prefix.len());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        write_json(&mut sink, value)
    }));
    let error = outcome
        .expect("a formatting refusal must not panic")
        .unwrap_err();
    assert_eq!(error.io_error_kind(), Some(io::ErrorKind::BrokenPipe));
    assert_eq!(sink.bytes, prefix);
    assert_eq!(sink.writes_after_error, 0);
}

#[test]
fn collect_str_preserves_the_first_error_even_when_display_ignores_it() {
    for ending in [
        Ending::Propagate,
        Ending::RetryAndSucceed,
        Ending::SwallowAndSucceed,
        Ending::SwallowAndFail,
    ] {
        refusal(&Text(ending), b"\"");
        refusal(&Key(Text(ending)), b"{\"");
        refusal(&Raw(Text(ending)), b"");
    }
}

fn format_refusal<T: Serialize>(value: &T) {
    let mut sink = Vec::new();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        write_json(&mut sink, value)
    }));
    let error = outcome
        .expect("bare fmt::Error is a serialization error, not a panic")
        .unwrap_err();
    assert_eq!(error.classify(), sonic_rs::error::Category::TypeUnmatched);
    assert_eq!(error.io_error_kind(), None);
    assert_eq!(error.to_string(), "error while formatting JSON value");
}

#[test]
fn bare_formatting_failure_is_a_non_io_error() {
    format_refusal(&Text(Ending::BareFailure));
    format_refusal(&Key(Text(Ending::BareFailure)));
    format_refusal(&Raw(Text(Ending::BareFailure)));
}

#[derive(Clone)]
struct WholeRaw(Rc<RefCell<Vec<String>>>);
impl Formatter for WholeRaw {
    fn write_raw_value<W: io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        raw: &str,
    ) -> io::Result<()> {
        self.0.borrow_mut().push(raw.to_owned());
        writer.write_all(b"<")?;
        writer.write_all(raw.as_bytes())?;
        writer.write_all(b">")
    }
}

#[test]
fn old_custom_formatter_receives_one_complete_raw_value() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut serializer = sonic_rs::Serializer::with_formatter(Vec::new(), WholeRaw(calls.clone()));
    Raw(Text(Ending::Propagate))
        .serialize(&mut serializer)
        .unwrap();
    assert_eq!(serializer.into_inner(), b"<12>");
    assert_eq!(*calls.borrow(), ["12"]);
    calls.borrow_mut().clear();
    let number: sonic_rs::RawNumber = sonic_rs::from_str("123").unwrap();
    let mut serializer = sonic_rs::Serializer::with_formatter(Vec::new(), WholeRaw(calls.clone()));
    number.serialize(&mut serializer).unwrap();
    assert_eq!(serializer.into_inner(), b"<123>");
    assert_eq!(*calls.borrow(), ["123"]);
}

struct ShortWrites {
    bytes: Vec<u8>,
    interrupt: bool,
    zero: bool,
}
impl io::Write for ShortWrites {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.zero {
            return Ok(0);
        }
        let amount = bytes.len().min(1);
        self.bytes.extend_from_slice(&bytes[..amount]);
        Ok(amount)
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("must not flush")
    }
}

#[test]
fn short_interrupted_and_zero_writes_follow_write_all_semantics() {
    let mut sink = ShortWrites {
        bytes: Vec::new(),
        interrupt: true,
        zero: false,
    };
    write_json(&mut sink, "é\n").unwrap();
    assert_eq!(sink.bytes, "\"é\\n\"".as_bytes());
    sink.zero = true;
    let error = write_json(&mut sink, "x").unwrap_err();
    assert_eq!(error.io_error_kind(), Some(io::ErrorKind::WriteZero));
}
