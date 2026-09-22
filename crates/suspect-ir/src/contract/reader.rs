//! Reader selection for contract document value materialization.
//!
//! [`ContractReader::Lossless`] decodes document values from the full
//! lossless CST — the historical default. [`ContractReader::Fast`]
//! materializes document values *independently* through
//! `suspect_syntax::fast::try_parse_fast` and the same exact core-scalar
//! rules; any syntax outside the supported block-style subset declines with
//! an explicit error instead of silently substituting CST values.
//!
//! Both readers produce only *document values*, consumed by the same structural
//! graph traversal and static reference decoder. Document loading and source
//! spans still use the lossless workspace sidecar, so the fast reader makes no
//! parse-speed or span-advertising claim. Reader parity is checked through the
//! public Contract seam, including graph identity and diagnostics.

use serde_json::Value;
use suspect_low::LowDoc;
use suspect_source::Uri;
use suspect_syntax::{Format, try_parse_fast};

use super::ContractError;

/// Which reader materializes normalized document values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContractReader {
    /// Full lossless CST decoding. Historical default behavior.
    #[default]
    Lossless,
    /// Independent fast-path value materialization. Documents using YAML
    /// syntax outside the supported subset decline with an explicit error.
    Fast,
}

impl ContractReader {
    /// Normalized JSON document value for `doc`.
    pub(super) fn document_value(&self, uri: &Uri, doc: &LowDoc) -> Result<Value, ContractError> {
        match self {
            Self::Lossless => lossless_value(uri, doc),
            Self::Fast => {
                let value = fast_value(uri, doc)?;
                // Defensive parity check: report divergence instead of
                // silently accepting changed values.
                let lossless = lossless_value(uri, doc)?;
                if value != lossless {
                    return Err(ContractError(format!(
                        "fast/lossless reader divergence in `{uri}`: independently materialized values disagree"
                    )));
                }
                Ok(value)
            }
        }
    }
}

/// Lossless document value, identical to the historical `document()` output.
fn lossless_value(uri: &Uri, doc: &LowDoc) -> Result<Value, ContractError> {
    if uri
        .as_path()
        .is_some_and(|p| p.extension().is_some_and(|e| e == "json"))
    {
        serde_json::from_slice(doc.inner().bytes())
            .map_err(|e| ContractError(format!("invalid JSON in {uri}: {e}")))
    } else {
        let value = crate::fast::value_from_node(doc.root()).map_err(ContractError)?;
        Ok(crate::fast::json(Some(&value)))
    }
}

/// Fast document value. JSON uses the exact JSON parser; YAML must parse
/// with the block-style fast reader or the compilation declines.
fn fast_value(uri: &Uri, doc: &LowDoc) -> Result<Value, ContractError> {
    match doc.inner().format() {
        Format::Json => serde_json::from_slice(doc.inner().bytes())
            .map_err(|e| ContractError(format!("invalid JSON in {uri}: {e}"))),
        Format::Yaml => {
            let Some(fast) = try_parse_fast(doc.inner().bytes()) else {
                return Err(ContractError(format!(
                    "fast reader declined `{uri}`: the document uses YAML syntax outside the \
                     supported block-style subset (anchors, aliases, tags, directives, multi-line \
                     flow, tabs); select ContractReader::Lossless for this input"
                )));
            };
            // Same exact core-scalar conversion the lossless YAML path uses.
            Ok(crate::fast::json(Some(&fast)))
        }
    }
}
