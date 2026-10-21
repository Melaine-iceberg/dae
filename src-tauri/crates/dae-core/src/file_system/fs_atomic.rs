//! Atomic document writes shared by the JSON stores in this crate.

use super::error::FileSystemError;

/// Writes via a temp file + rename so readers never observe a partial document.
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> Result<(), FileSystemError> {
    let temp_path = path.with_extension("tmp");
    std::fs::write(&temp_path, contents)?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}
