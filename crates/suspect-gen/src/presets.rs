//! Built-in generation presets.
//!
//! A [`Preset`] bundles starter templates, a manifest, and an IR-to-context
//! builder for a known framework. This module is intentionally empty of
//! actual presets for now — real ones land in a later change.

use suspect_ir::IrSpec;

/// A bundled generation preset.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    /// `(template_name, template_source)` pairs installed into the engine.
    pub templates: &'static [(&'static str, &'static str)],
    /// Starter manifest text referencing the bundled templates.
    pub manifest_toml: &'static str,
    /// Builds the render context from a normalized spec.
    pub ctx_builder: fn(&IrSpec) -> serde_json::Value,
}

/// Looks up a preset by name.
///
/// Returns `None` for every name until real presets are introduced.
#[must_use]
pub fn get(_name: &str) -> Option<Preset> {
    None
}
