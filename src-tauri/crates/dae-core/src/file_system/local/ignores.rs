//! The directory names every *search* in this backend skips.
//!
//! Name search and content search both fan out over a whole tree, and both
//! would otherwise descend into `.git`, `node_modules` and `target`: on a
//! conventional Node checkout that is 99 % of the entries visited and almost
//! none of the answers. Content search has always applied this list (ripgrep's
//! practical defaults, which its own tests pin); name search walked them,
//! which made a search over a 460-file project visit 72 000 dependency files
//! and let dependency hits fill the result cap. One definition, so the two
//! modes cannot disagree about what "the project" means.
//!
//! The patterns are `ignore`'s exclusion form — `!`-prefixed entries in an
//! `OverrideBuilder`, where a bare pattern would be a whitelist — and they are
//! relative to the walk root. That is deliberate: the rule excludes a
//! *descendant* named `node_modules`, never the root itself, so searching from
//! inside `node_modules/some-package` still finds that package's files.
//!
//! Deliberately not used by `directory_size.rs`: a size report is asked for a
//! number, and a number that silently omits dependency directories is wrong
//! rather than cheap.

use crate::file_system::error::FileSystemError;
use ignore::overrides::{Override, OverrideBuilder};
use std::path::Path;

/// Directories skipped wherever they appear below a search root.
const ALWAYS_IGNORED: [&str; 4] = ["!.git", "!**/.git", "!**/node_modules", "!**/target"];

/// [`ALWAYS_IGNORED`] as a matcher for a walk rooted at `root`.
pub(super) fn default_overrides(root: &Path) -> Result<Override, FileSystemError> {
    let mut overrides = OverrideBuilder::new(root);
    for pattern in ALWAYS_IGNORED {
        overrides.add(pattern).map_err(|error| {
            FileSystemError::Internal(format!("fs.ignore_rule_register_failed: {error}"))
        })?;
    }

    overrides
        .build()
        .map_err(|error| FileSystemError::Internal(format!("fs.ignore_rule_build_failed: {error}")))
}
