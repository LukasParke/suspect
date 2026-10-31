//! Ruleset loading for `suspect lint`, which is `check` with
//! [`super::check::LINT`]: findings, policy and output are the check
//! pass's.

use std::path::Path;

use suspect_lint::Linter;
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

/// Builds a linter from an optional ruleset file.
///
/// # Errors
/// IO on the ruleset path or a malformed ruleset document.
pub fn build_linter(ruleset: Option<&Path>) -> anyhow::Result<Linter> {
    match ruleset {
        None => Ok(Linter::spectral_default()),
        Some(path) => {
            let source = Source::from_path(path)
                .map_err(|e| anyhow::anyhow!("ruleset {}: {e}", path.display()))?;
            let uri = Uri::from_path(path)?;
            let doc = LowDoc::parse(uri, source);
            Linter::from_ruleset(&doc).map_err(|e| anyhow::anyhow!("ruleset error: {e}"))
        }
    }
}
