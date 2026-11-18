//! Borrowed decoding over the shared Serde state machine. The adapter changes the visitor
//! error family, not the JSON grammar. Originating parser errors are recorded as fixed
//! metadata by the reader before they cross the generic Serde error adapters.

use super::de::Deserializer;
use crate::{
    error::Category,
    reader::{Read, Reader},
    Error,
};
use serde::de::{self, DeserializeSeed, Visitor};
use std::fmt;

/// A position and fixed classification; never owns an input excerpt or visitor message.
#[derive(Clone, Copy, Debug)]
pub struct BorrowedError {
    category: Category,
    reason: &'static str,
    offset: usize,
    line: usize,
    column: usize,
}

impl BorrowedError {
    pub(crate) fn data(reason: &'static str) -> Self {
        Self {
            category: Category::TypeUnmatched,
            reason,
            offset: 0,
            line: 1,
            column: 1,
        }
    }
    pub(crate) fn parser(error: &Error) -> Self {
        let category = error.classify();
        let reason = if error.borrowed_native_refusal() {
            "unsupported native JSON type"
        } else {
            match category {
                Category::Syntax => "invalid JSON syntax",
                Category::Eof => "unexpected end of JSON",
                Category::Io => "JSON input failure",
                Category::NotFound => "JSON member not found",
                Category::TypeUnmatched => "unsupported or mismatched JSON type",
            }
        };
        Self {
            category,
            reason,
            offset: error.offset(),
            line: error.line(),
            column: error.column(),
        }
    }
    /// Error classification without input text.
    pub fn classify(&self) -> Category {
        self.category
    }
    /// Fixed reason text, without input or visitor-supplied text.
    pub fn reason(&self) -> &'static str {
        self.reason
    }
    /// Byte offset in the original input.
    pub fn offset(&self) -> usize {
        self.offset
    }
    /// One-based line in the original input.
    pub fn line(&self) -> usize {
        self.line
    }
    /// One-based column in the original input.
    pub fn column(&self) -> usize {
        self.column
    }
}

impl fmt::Display for BorrowedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {} column {}",
            self.reason, self.line, self.column
        )
    }
}
impl std::error::Error for BorrowedError {}
impl de::Error for BorrowedError {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Self::data("invalid JSON value")
    }
    fn invalid_type(_: de::Unexpected<'_>, _: &dyn de::Expected) -> Self {
        Self::data("JSON type mismatch")
    }
    fn invalid_value(_: de::Unexpected<'_>, _: &dyn de::Expected) -> Self {
        Self::data("invalid JSON value")
    }
    fn invalid_length(_: usize, _: &dyn de::Expected) -> Self {
        Self::data("invalid JSON length")
    }
    fn unknown_variant(_: &str, _: &'static [&'static str]) -> Self {
        Self::data("unknown JSON variant")
    }
    fn unknown_field(_: &str, _: &'static [&'static str]) -> Self {
        Self::data("unknown JSON field")
    }
    fn missing_field(_: &'static str) -> Self {
        Self::data("missing JSON field")
    }
    fn duplicate_field(_: &'static str) -> Self {
        Self::data("duplicate JSON field")
    }
}

/// Deserialize with stable borrowed strings, including escaped strings in `decoded`.
///
/// `decoded` must be at least as long as input. It must remain alive with input for the
/// returned result's lifetime. Callers handling credentials should own both in wiping
/// buffers and discard those buffers on error/unwind. No owned native Value/LazyValue
/// protocol or lossy UTF mode is supported. User Deserialize implementations may allocate.
///
/// ```compile_fail
/// let value: &str;
/// {
///     let input = br#""a\u0062""#.to_vec();
///     let mut scratch = vec![0; input.len()];
///     value = sonic_rs::from_slice_borrowed(&input, &mut scratch).unwrap();
/// }
/// assert_eq!(value, "ab");
/// ```
pub fn from_slice_borrowed<'de, T: de::Deserialize<'de>>(
    input: &'de [u8],
    decoded: &'de mut [u8],
) -> Result<T, BorrowedError> {
    if decoded.len() < input.len() {
        return Err(BorrowedError::data("insufficient decode scratch"));
    }
    if input.len() > u32::MAX as usize {
        return Err(BorrowedError::data("JSON input too large"));
    }
    let mut decoder = Deserializer::new(Read::borrowed(input, decoded));
    #[cfg(feature = "arbitrary_precision")]
    {
        decoder = decoder.use_rawnumber();
    }
    // Deliberately do not inherit the global utf8_lossy feature.
    let result = T::deserialize(Guarded(&mut decoder));
    let value = match result {
        Ok(value) => value,
        Err(error) => return Err(decoder.parser.read.borrowed_error().unwrap_or(error)),
    };
    if let Err(error) = decoder
        .end()
        .and_then(|()| decoder.parser.read.check_utf8_final())
    {
        return Err(decoder
            .parser
            .read
            .borrowed_error()
            .unwrap_or_else(|| BorrowedError::parser(&error)));
    }
    Ok(value)
}

// Generic Serde adapters must not inspect or format an arbitrary deserializer error.
// The shared reader records the original static category/position at its factory.
fn visitor_error<E>(_: E) -> BorrowedError {
    BorrowedError::data("invalid JSON value")
}
fn bridge_error<E: de::Error>(_: BorrowedError) -> E {
    E::custom("invalid borrowed JSON value")
}
struct Guarded<D>(D);
struct Bridge<V>(V);
struct Seed<S>(S);

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Seed<S> {
    type Value = S::Value;
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        self.0.deserialize(Guarded(decoder)).map_err(bridge_error)
    }
}

macro_rules! decode {
    ($($method:ident),* $(,)?) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.0.$method(Bridge(visitor)).map_err(visitor_error)
        }
    )*};
}
impl<'de, D: de::Deserializer<'de>> de::Deserializer<'de> for Guarded<D> {
    type Error = BorrowedError;
    decode!(
        deserialize_any,
        deserialize_bool,
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64,
        deserialize_i128,
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64,
        deserialize_u128,
        deserialize_f32,
        deserialize_f64,
        deserialize_char,
        deserialize_str,
        deserialize_string,
        deserialize_bytes,
        deserialize_byte_buf,
        deserialize_option,
        deserialize_unit,
        deserialize_seq,
        deserialize_map,
        deserialize_identifier,
        deserialize_ignored_any
    );
    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_unit_struct(name, Bridge(visitor))
            .map_err(visitor_error)
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_newtype_struct(name, Bridge(visitor))
            .map_err(visitor_error)
    }
    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_tuple(len, Bridge(visitor))
            .map_err(visitor_error)
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_tuple_struct(name, len, Bridge(visitor))
            .map_err(visitor_error)
    }
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_struct(name, fields, Bridge(visitor))
            .map_err(visitor_error)
    }
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_enum(name, variants, Bridge(visitor))
            .map_err(visitor_error)
    }
    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
}

macro_rules! scalar {
    ($($method:ident($ty:ty)),* $(,)?) => {$(
        fn $method<E: de::Error>(self, value: $ty) -> Result<Self::Value, E> {
            self.0.$method::<BorrowedError>(value).map_err(bridge_error)
        }
    )*};
}
impl<'de, V: Visitor<'de>> Visitor<'de> for Bridge<V> {
    type Value = V::Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a compatible JSON value")
    }
    scalar!(
        visit_bool(bool),
        visit_i8(i8),
        visit_i16(i16),
        visit_i32(i32),
        visit_i64(i64),
        visit_i128(i128),
        visit_u8(u8),
        visit_u16(u16),
        visit_u32(u32),
        visit_u64(u64),
        visit_u128(u128),
        visit_f32(f32),
        visit_f64(f64),
        visit_char(char),
        visit_str(&str),
        visit_borrowed_str(&'de str),
        visit_string(String),
        visit_bytes(&[u8]),
        visit_borrowed_bytes(&'de [u8]),
        visit_byte_buf(Vec<u8>)
    );
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        self.0.visit_none::<BorrowedError>().map_err(bridge_error)
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        self.0.visit_unit::<BorrowedError>().map_err(bridge_error)
    }
    fn visit_some<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        self.0.visit_some(Guarded(decoder)).map_err(bridge_error)
    }
    fn visit_newtype_struct<D: de::Deserializer<'de>>(
        self,
        decoder: D,
    ) -> Result<Self::Value, D::Error> {
        self.0
            .visit_newtype_struct(Guarded(decoder))
            .map_err(bridge_error)
    }
    fn visit_seq<A: de::SeqAccess<'de>>(self, access: A) -> Result<Self::Value, A::Error> {
        self.0.visit_seq(Guarded(access)).map_err(bridge_error)
    }
    fn visit_map<A: de::MapAccess<'de>>(self, access: A) -> Result<Self::Value, A::Error> {
        self.0.visit_map(Guarded(access)).map_err(bridge_error)
    }
    fn visit_enum<A: de::EnumAccess<'de>>(self, access: A) -> Result<Self::Value, A::Error> {
        self.0.visit_enum(Guarded(access)).map_err(bridge_error)
    }
}

impl<'de, A: de::SeqAccess<'de>> de::SeqAccess<'de> for Guarded<A> {
    type Error = BorrowedError;
    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.0.next_element_seed(Seed(seed)).map_err(visitor_error)
    }
    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}
impl<'de, A: de::MapAccess<'de>> de::MapAccess<'de> for Guarded<A> {
    type Error = BorrowedError;
    fn next_key_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.0.next_key_seed(Seed(seed)).map_err(visitor_error)
    }
    fn next_value_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        self.0.next_value_seed(Seed(seed)).map_err(visitor_error)
    }
    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}
impl<'de, A: de::EnumAccess<'de>> de::EnumAccess<'de> for Guarded<A> {
    type Error = BorrowedError;
    type Variant = Guarded<A::Variant>;
    fn variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<(T::Value, Self::Variant), Self::Error> {
        self.0
            .variant_seed(Seed(seed))
            .map(|(value, variant)| (value, Guarded(variant)))
            .map_err(visitor_error)
    }
}
impl<'de, A: de::VariantAccess<'de>> de::VariantAccess<'de> for Guarded<A> {
    type Error = BorrowedError;
    fn unit_variant(self) -> Result<(), Self::Error> {
        self.0.unit_variant().map_err(visitor_error)
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        self.0
            .newtype_variant_seed(Seed(seed))
            .map_err(visitor_error)
    }
    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .tuple_variant(len, Bridge(visitor))
            .map_err(visitor_error)
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .struct_variant(fields, Bridge(visitor))
            .map_err(visitor_error)
    }
}
