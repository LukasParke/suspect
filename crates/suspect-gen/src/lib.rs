//! Template-driven code generation for the suspect platform.
//!
//! [`MinijinjaEngine`] evaluates [minijinja] templates against a JSON
//! context; [`FilterRegistry`] installs the built-in generation filters
//! (case conversion, type mapping, example synthesis, Mermaid graphs).
//! [`orchestrate::render_manifest`] walks a parsed [`Manifest`], renders
//! each output, splices preserved user-code regions back in, and only
//! rewrites files whose content hash actually changed.
//!
//! [minijinja]: https://docs.rs/minijinja

#![deny(missing_docs)]

use std::fmt;

pub mod filters;
mod json_context;
pub mod orchestrate;
pub mod presets;
mod rust_support;
#[cfg(test)]
mod tests;

/// Error raised by generation and orchestration failures.
///
/// Wraps a human-readable message; template errors, I/O errors, and
/// manifest parse errors all surface as this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenError(pub String);

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gen error: {}", self.0)
    }
}

impl std::error::Error for GenError {}

impl From<std::io::Error> for GenError {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<minijinja::Error> for GenError {
    fn from(e: minijinja::Error) -> Self {
        Self(e.to_string())
    }
}

/// An immutable input prepared for repeated template rendering.
///
/// Create one with [`TemplateEngine::prepare_context`] and reuse it with
/// [`TemplateEngine::render_prepared`]. Its borrow keeps the source JSON
/// unchanged for the duration of the prepared context; any converted data
/// belongs to this context rather than an engine-wide cache.
pub struct PreparedContext<'a> {
    source: &'a serde_json::Value,
    minijinja: Option<minijinja::Value>,
}

impl PreparedContext<'_> {
    fn minijinja_value(&self) -> minijinja::Value {
        self.minijinja
            .clone()
            .unwrap_or_else(|| json_context::to_template(self.source))
    }
}

/// Engine contract used by manifest rendering.
///
/// Implementations own their template store; templates are added by name
/// and evaluated against a JSON context.
pub trait TemplateEngine: Send + Sync {
    /// Renders the named template with `ctx` (any serializable JSON value).
    /// Each call observes the current value of `ctx`.
    ///
    /// # Errors
    /// When the template is unknown or evaluation fails.
    fn render(&self, template_name: &str, ctx: &serde_json::Value) -> Result<String, GenError>;

    /// Prepares immutable JSON for repeated rendering without reconversion.
    ///
    /// [`MinijinjaEngine`] converts the JSON once. The default keeps the
    /// JSON borrow, preserving compatibility with engines that implement
    /// only [`Self::render`] and [`Self::add_template`].
    #[must_use]
    fn prepare_context<'a>(&self, ctx: &'a serde_json::Value) -> PreparedContext<'a> {
        PreparedContext {
            source: ctx,
            minijinja: None,
        }
    }

    /// Renders with an immutable context prepared for reuse across files.
    ///
    /// The default delegates to [`Self::render`] with the source JSON.
    ///
    /// # Errors
    /// When the template is unknown or evaluation fails.
    fn render_prepared(
        &self,
        template_name: &str,
        ctx: &PreparedContext<'_>,
    ) -> Result<String, GenError> {
        self.render(template_name, ctx.source)
    }

    /// Adds (or replaces) a template under `name`.
    ///
    /// # Errors
    /// When the template source fails to compile.
    fn add_template(&mut self, name: &str, src: &str) -> Result<(), GenError>;
}

/// [`TemplateEngine`] over [minijinja].
///
/// minijinja 2.x evaluates every environment in a sandbox by design:
/// templates can only touch the values passed in as context, cannot access
/// host attributes outside the value tree, and have no I/O surface.
/// Templates are stored owned inside the environment.
///
/// [minijinja]: https://docs.rs/minijinja
pub struct MinijinjaEngine {
    env: minijinja::Environment<'static>,
}

impl MinijinjaEngine {
    /// Creates an empty engine.
    #[must_use]
    pub fn new() -> Self {
        let mut env = minijinja::Environment::new();
        env.add_filter("tojson", json_context::tojson);
        Self { env }
    }

    /// Renders a short-lived per-entity context without preparing it for reuse.
    pub(crate) fn render_once(
        &self,
        template_name: &str,
        ctx: &serde_json::Value,
    ) -> Result<String, GenError> {
        self.render(template_name, ctx)
    }
}

impl Default for MinijinjaEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateEngine for MinijinjaEngine {
    fn render(&self, template_name: &str, ctx: &serde_json::Value) -> Result<String, GenError> {
        let tmpl = self.env.get_template(template_name)?;
        Ok(tmpl.render(json_context::to_template(ctx))?)
    }

    fn prepare_context<'a>(&self, ctx: &'a serde_json::Value) -> PreparedContext<'a> {
        PreparedContext {
            source: ctx,
            minijinja: Some(json_context::to_template(ctx)),
        }
    }

    fn render_prepared(
        &self,
        template_name: &str,
        ctx: &PreparedContext<'_>,
    ) -> Result<String, GenError> {
        Ok(self
            .env
            .get_template(template_name)?
            .render(ctx.minijinja_value())?)
    }

    fn add_template(&mut self, name: &str, src: &str) -> Result<(), GenError> {
        self.env
            .add_template_owned(name.to_owned(), src.to_owned())?;
        Ok(())
    }
}

pub use filters::{
    FilterRegistry, example_of, examples_for_components, mermaid_refs, rust_type, scalar_example,
    split_words, ts_type,
};
pub use orchestrate::{
    BEGIN_MARK, END_MARK, Manifest, OutputRule, RenderOutcome, WriteReason, load_manifest,
    parse_manifest, parse_manifest_str, render_manifest, render_manifest_owned,
};
pub use suspect_artifact::Adoption;
