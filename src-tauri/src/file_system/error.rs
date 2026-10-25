//! Temporary shim.
//!
//! See `types.rs` next to this file: the error enum moved to `dae-core`, the
//! path stayed so existing call sites keep compiling.
pub use dae_core::error::*;
