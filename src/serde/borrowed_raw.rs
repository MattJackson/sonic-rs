//! Original JSON spans. This protocol deliberately has no LazyValue escape flags or cache.

use crate::JsonType;
use serde::de::{self, Visitor};
use std::fmt;

pub(crate) const TOKEN: &str = "$sonic_rs::BorrowedRaw";

/// A validated original token, including quotes and escapes. No decoded or owned accessor.
#[derive(Clone, Copy)]
pub struct BorrowedRaw<'de> {
    raw: &'de str,
}

impl<'de> BorrowedRaw<'de> {
    /// The exact original JSON token, not decoded string contents.
    pub fn as_json(&self) -> &'de str {
        self.raw
    }
    /// Token kind without decoding or materializing it.
    pub fn kind(&self) -> JsonType {
        match self.raw.as_bytes()[0] {
            b'"' => JsonType::String,
            b'{' => JsonType::Object,
            b'[' => JsonType::Array,
            b'n' => JsonType::Null,
            b't' | b'f' => JsonType::Boolean,
            _ => JsonType::Number,
        }
    }
}
impl fmt::Debug for BorrowedRaw<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BorrowedRaw")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}
struct RawVisitor;
impl<'de> Visitor<'de> for RawVisitor {
    type Value = BorrowedRaw<'de>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a borrowed raw JSON token")
    }
    fn visit_borrowed_str<E: de::Error>(self, raw: &'de str) -> Result<Self::Value, E> {
        Ok(BorrowedRaw { raw })
    }
}
impl<'de: 'a, 'a> de::Deserialize<'de> for BorrowedRaw<'a> {
    fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        decoder
            .deserialize_newtype_struct(TOKEN, RawVisitor)
            .map(|value| Self { raw: value.raw })
    }
}
