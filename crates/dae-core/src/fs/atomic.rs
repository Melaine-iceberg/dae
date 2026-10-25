//! Atomic writes for the small documents the profile owns.
//!
//! Every shell writes the same JSON documents — favorites, recents, spaces,
//! connections, cloud accounts, settings — so the temp-file-and-rename dance
//! belongs next to the paths, not in whichever module grew it first.

use crate::error::FileSystemError;
use std::fs;
use std::path::Path;

/// Writes via a temp file + rename so readers never observe a partial document.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), FileSystemError> {
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, contents)?;
    fs::rename(&temp_path, path)?;
    Ok(())
}
