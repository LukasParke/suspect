#![deny(missing_docs)]
//! suspect-lint: Spectral-compatible linting engine and builtin rule packs.
//!
//! Rules are compiled once into a [`Linter`] and run against any
//! [`suspect_low::LowDoc`]. Rule definitions come either from the builtin
//! packs ([`Linter::spectral_default`]) or from a Spectral-style ruleset
//! document parsed through suspect-low ([`Linter::from_ruleset`]).
//!
//! ```
//! use suspect_source::{Source, Uri};
//! let doc = suspect_low::LowDoc::parse(
//!     Uri::from("mem://api.yaml"),
//!     Source::from_vec(b"openapi: 3.1.0\ninfo: {}\npaths: {}\n".to_vec()),
//! );
//! let linter = suspect_lint::Linter::spectral_default();
//! for finding in linter.run(&doc) {
//!     println!("{}: {}", finding.severity, finding.message);
//! }
//! ```

mod engine;
mod fast;
mod functions;
mod packs;
mod rule;
mod ruleset;

pub use engine::{Finding, Linter, RulesetError};
pub use rule::Severity;
