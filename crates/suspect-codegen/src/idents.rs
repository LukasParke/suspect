//! Identifier sanitization shared by emitters.

pub use crate::stg::Ident;

/// Builds a sanitized [`Ident`] from arbitrary source text.
#[must_use]
pub fn ident(original: &str) -> Ident {
    Ident::new(original)
}
