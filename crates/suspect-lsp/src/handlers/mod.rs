//! LSP request and notification handlers, one module per feature.
//!
//! `impl LanguageServer for Backend` in the crate root only delegates here;
//! each module holds the inherent `Backend` methods that serve one feature,
//! wiring the protocol to the pure logic in the crate's feature modules.

mod capabilities;
mod commands;
mod completion;
mod custom;
mod diagnostics;
mod editing;
mod files;
mod hover;
mod lifecycle;
mod links;
mod navigation;
mod semantic;
mod symbols;
mod sync;
mod workspace;

#[cfg(test)]
pub(crate) use hover::hover_meaning;

use suspect_source::Uri;
use tower_lsp::lsp_types::Url;

/// Parses a [`Uri`] into an LSP [`Url`], or `None` when it does not parse.
pub(crate) fn to_url(uri: &Uri) -> Option<Url> {
    Url::parse(uri.as_str()).ok()
}

/// The configuration flavour of a document, when it is one of suspect's own
/// configuration files.
pub(crate) fn config_kind(uri: &Uri) -> Option<crate::config_schema::FileKind> {
    let path = uri.as_path()?;
    crate::config_schema::kind_of(path.file_name()?.to_str()?)
}
