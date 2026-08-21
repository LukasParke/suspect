#![deny(missing_docs)]
//! suspect-overlay: Overlay 1.0 models and apply engine.
//!
//! Actions apply sequentially to an owned value tree materialized from the
//! target document; output re-emits canonically as JSON or YAML. Targets are
//! RFC 9535 JSONPath queries evaluated through `suspect-jsonpath`.

mod apply;
mod error;
mod model;
mod value;

pub use apply::{apply, Applied};
pub use error::OverlayError;
pub use model::{validate_overlay, ActionView, OverlayDiagnostic, OverlayDoc};
pub use value::Value;
