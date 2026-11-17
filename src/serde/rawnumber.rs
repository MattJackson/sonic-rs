use ::serde::{
    de, de::Visitor, ser::SerializeStruct, Deserialize, Deserializer, Serialize, Serializer,
};
use ::std::fmt;
use faststr::FastStr;

use super::number::Number;
use crate::{util::private::Sealed, Error, JsonNumberTrait};

/// Represents a JSON number with arbitrary precision, the underlying representation of a string,
/// like as Golang `json.Number`.
///
/// Example1:
///
/// ```
/// use sonic_rs::RawNumber;
///
/// use crate::sonic_rs::JsonNumberTrait;
///
/// // RawNumber can be parsed from a JSON number text.
/// let num: RawNumber = sonic_rs::from_str("123").unwrap();
/// assert_eq!(num.as_i64(), Some(123));
/// assert_eq!(num.as_str(), "123");
///
/// // RawNumber can be parsed from a JSON string text that contains a number.
/// let num: RawNumber =
///     sonic_rs::from_str("\"1.2333333333333333333333333333333333333333\"").unwrap();
/// assert_eq!(num.as_f64(), Some(1.2333333333333334));
/// assert_eq!(num.as_str(), "1.2333333333333333333333333333333333333333");
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RawNumber {
    n: FastStr,
}

impl RawNumber {
    pub(crate) fn new(s: &str) -> Self {
        Self { n: FastStr::new(s) }
    }

    pub(crate) fn from_faststr(n: FastStr) -> Self {
        Self { n }
    }

    /// as_str returns the underlying string representation of the number.
    pub fn as_str(&self) -> &str {
        self.n.as_str()
    }
}

pub(crate) const TOKEN: &str = "$sonic_rs::private::JsonNumber";

/// The key of the one-entry map that `deserialize_any` hands a number as, with
/// `Deserializer::use_rawnumber_in_any`.
///
/// It is a static byte array, not a string literal: a static has an address of its own, which no
/// other string (an equal literal, a key read from a document) can share. [`is_number_token`]
/// recognises it by that address, so a key merely spelled the same stays an ordinary key.
#[cfg(feature = "arbitrary_precision")]
static NUMBER_TOKEN: [u8; 26] = *b"$sonic_rs::private::Number";

/// [`NUMBER_TOKEN`] as a string.
#[cfg(feature = "arbitrary_precision")]
pub(crate) fn number_token() -> &'static str {
    match std::str::from_utf8(&NUMBER_TOKEN) {
        Ok(token) => token,
        Err(_) => unreachable!("the number token is ASCII"),
    }
}

/// Whether `s` is [`NUMBER_TOKEN`] itself (same address and length), not merely equal text.
#[cfg(feature = "arbitrary_precision")]
pub(crate) fn is_number_token(s: &str) -> bool {
    std::ptr::eq(s.as_ptr(), NUMBER_TOKEN.as_ptr()) && s.len() == NUMBER_TOKEN.len()
}

/// Whether `s` is a whole JSON number: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
#[cfg(feature = "arbitrary_precision")]
pub(crate) fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |mut i: usize| {
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        (i, i > start)
    };
    let mut i = usize::from(b.first() == Some(&b'-'));
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => i = digits(i).0,
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        let (end, any) = digits(i + 1);
        if !any {
            return false;
        }
        i = end;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let (end, any) = digits(i);
        if !any {
            return false;
        }
        i = end;
    }
    i == b.len()
}

impl<'de> Deserialize<'de> for RawNumber {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<RawNumber, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct JsonNumberVisitor;

        impl<'de> Visitor<'de> for JsonNumberVisitor {
            type Value = RawNumber;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON number")
            }

            fn visit_borrowed_str<E>(self, raw: &'de str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawNumber::new(raw))
            }
        }

        deserializer.deserialize_newtype_struct(TOKEN, JsonNumberVisitor)
    }
}

impl Serialize for RawNumber {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct(TOKEN, 1)?;
        s.serialize_field(TOKEN, &self.n)?;
        s.end()
    }
}

impl Sealed for RawNumber {}

impl JsonNumberTrait for RawNumber {
    /// Returns true if the `Number` is an integer between `i64::MIN` and
    /// `i64::MAX`.
    ///
    /// For any Number on which `is_i64` returns true, `as_i64` is guaranteed to
    /// return the integer value.
    #[inline]
    fn is_i64(&self) -> bool {
        self.as_i64().is_some()
    }

    /// Returns true if the `Number` is an integer between zero and `u64::MAX`.
    ///
    /// For any Number on which `is_u64` returns true, `as_u64` is guaranteed to
    /// return the integer value.
    #[inline]
    fn is_u64(&self) -> bool {
        self.as_u64().is_some()
    }

    /// Returns true if the `Number` can be represented by f64.
    ///
    /// For any Number on which `is_f64` returns true, `as_f64` is guaranteed to
    /// return the floating point value.
    ///
    /// Currently this function returns true if and only if both `is_i64` and
    /// `is_u64` return false but this is not a guarantee in the future.
    #[inline]
    fn is_f64(&self) -> bool {
        self.as_f64().is_some()
    }

    /// If the `Number` is an integer, represent it as i64 if possible. Returns
    /// None otherwise.
    #[inline]
    fn as_i64(&self) -> Option<i64> {
        self.n.parse().ok()
    }

    /// If the `Number` is an integer, represent it as u64 if possible. Returns
    /// None otherwise.
    #[inline]
    fn as_u64(&self) -> Option<u64> {
        self.n.parse().ok()
    }

    /// Represents the number as finite f64 if possible. Returns None otherwise.
    #[inline]
    fn as_f64(&self) -> Option<f64> {
        self.n.parse::<f64>().ok().filter(|float| float.is_finite())
    }
}

impl TryFrom<RawNumber> for Number {
    type Error = Error;

    fn try_from(value: RawNumber) -> Result<Self, Self::Error> {
        let num: Number = crate::from_str(value.n.as_str())?;
        Ok(num)
    }
}
