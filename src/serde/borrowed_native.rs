//! Full-document native validation before a borrowed projection.

use super::{borrowed::BorrowedError, de::Deserializer, from_slice_borrowed};
use crate::{
    reader::{Read, Reader},
    value::visitor::JsonVisitor,
};
use serde::Deserialize;

/// Validate the complete document using native DOM depth and number policy, then borrow `T`.
///
/// This entrypoint is for projections replacing a native [`crate::Value`] parse. It validates
/// every member before invoking any user `Deserialize` callback, including unselected values.
/// The native parser traverses the document without constructing a DOM or copying input.
/// The `arbitrary_precision` feature controls numeric validation as in [`crate::from_slice`].
/// UTF-8 and UTF-16 are always strict, even when the global `utf8_lossy` feature is enabled.
/// Parser error offsets and EOF/syntax subcategories need not match the padded native parser.
///
/// `decoded` must be at least as long as `input`. The validation pass decodes strings into
/// that arena and retains no references; the projection then reuses it from its start.
/// Returned borrows have the same lifetime/storage contract as [`from_slice_borrowed`].
/// Credential callers must keep both buffers under wiping ownership, including on error
/// and unwind. User `Deserialize` implementations may allocate.
///
/// This operation does not deduplicate members or choose field semantics. A projection
/// replacing native object lookup must retain the first decoded-key match, including null
/// or wrong-shaped first values, and count actual members when its shape contract requires it.
/// Typed DTOs and raw/ignored traversals that do not need native validation should continue
/// to use [`from_slice_borrowed`], whose existing acceptance is unchanged.
///
/// ```compile_fail
/// let value: &str;
/// {
///     let input = br#""a\u0062""#.to_vec();
///     let mut decoded = vec![0; input.len()];
///     value = sonic_rs::from_slice_borrowed_native(&input, &mut decoded).unwrap();
/// }
/// assert_eq!(value, "ab");
/// ```
pub fn from_slice_borrowed_native<'de, T: Deserialize<'de>>(
    input: &'de [u8],
    decoded: &'de mut [u8],
) -> Result<T, BorrowedError> {
    if decoded.len() < input.len() {
        return Err(BorrowedError::data("insufficient decode scratch"));
    }
    if input.len() > u32::MAX as usize {
        return Err(BorrowedError::data("JSON input too large"));
    }
    // This call's shorter reborrow ends before the projection can publish any reference.
    validate_native(input, decoded)?;
    from_slice_borrowed(input, decoded)
}

fn validate_native(input: &[u8], decoded: &mut [u8]) -> Result<(), BorrowedError> {
    let mut decoder = Deserializer::new(Read::borrowed(input, decoded));
    #[cfg(feature = "arbitrary_precision")]
    {
        decoder = decoder.use_rawnumber();
    }
    // Some selects checked numbers/keys throughout native traversal. The borrowed-reader
    // branch in parse_string_visit bypasses this Vec before any payload can be written.
    let mut checked_selector = Vec::new();
    let result = decoder
        .parser
        .parse_dom(&mut Validate, Some(&mut checked_selector))
        .and_then(|()| decoder.end())
        .and_then(|()| decoder.parser.read.check_utf8_final());
    debug_assert_eq!(checked_selector.capacity(), 0);
    result.map_err(|error| {
        decoder
            .parser
            .read
            .borrowed_error()
            .unwrap_or_else(|| BorrowedError::parser(&error))
    })
}

// Every callback accepts its scalar/structural fact and retains nothing. In particular,
// no first-pass arena reference can survive validate_native and alias the projection.
struct Validate;
impl<'de> JsonVisitor<'de> for Validate {
    fn visit_dom_start(&mut self) -> bool {
        true
    }
    fn visit_dom_end(&mut self) -> bool {
        true
    }
    fn visit_null(&mut self) -> bool {
        true
    }
    fn visit_bool(&mut self, _: bool) -> bool {
        true
    }
    fn visit_u64(&mut self, _: u64) -> bool {
        true
    }
    fn visit_i64(&mut self, _: i64) -> bool {
        true
    }
    fn visit_f64(&mut self, _: f64) -> bool {
        true
    }
    fn visit_raw_number(&mut self, _: &str) -> bool {
        true
    }
    fn visit_borrowed_raw_number(&mut self, _: &str) -> bool {
        true
    }
    fn visit_str(&mut self, _: &str) -> bool {
        true
    }
    fn visit_borrowed_str(&mut self, _: &'de str) -> bool {
        true
    }
    fn visit_key(&mut self, _: &str) -> bool {
        true
    }
    fn visit_borrowed_key(&mut self, _: &'de str) -> bool {
        true
    }
    fn visit_object_start(&mut self, _: usize) -> bool {
        true
    }
    fn visit_object_end(&mut self, _: usize) -> bool {
        true
    }
    fn visit_array_start(&mut self, _: usize) -> bool {
        true
    }
    fn visit_array_end(&mut self, _: usize) -> bool {
        true
    }
}
