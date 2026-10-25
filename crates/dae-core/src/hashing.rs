//! The checksum report a finished hash run hands to the properties dialog.
//!
//! All three digests come from one read of the file (`sha2`, `md-5` and `sha1`
//! share the `digest` trait), so this is the shape of that single pass — not a
//! request to compute any one of them.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Lowercase hex digests of one file, computed in a single pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileHashDigests {
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}
