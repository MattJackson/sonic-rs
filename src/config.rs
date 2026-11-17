#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DeserializeCfg {
    pub use_rawnumber: bool,
    #[cfg(feature = "arbitrary_precision")]
    pub use_rawnumber_in_any: bool,
    pub utf8_lossy: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SerializeCfg {
    pub sort_map_keys: bool,
}
