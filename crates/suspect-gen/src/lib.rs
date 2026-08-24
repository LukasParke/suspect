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
pub mod orchestrate;
pub mod presets;
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

/// Engine contract used by manifest rendering.
///
/// Implementations own their template store; templates are added by name
/// and evaluated against a JSON context.
pub trait TemplateEngine: Send + Sync {
    /// Renders the named template with `ctx` (any serializable JSON value).
    ///
    /// # Errors
    /// When the template is unknown or evaluation fails.
    fn render(&self, template_name: &str, ctx: &serde_json::Value) -> Result<String, GenError>;

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
    /// Memoized context conversion: `render` deep-converts the incoming
    /// [`serde_json::Value`] into a minijinja value on every call, which
    /// dominates render time for large specs (megabytes of JSON). The most
    /// recently converted context is kept, keyed by its address; callers
    /// must not mutate a context in place between renders.
    ctx_cache: std::sync::Mutex<CtxCache>,
}

#[derive(Default)]
struct CtxCache {
    addr: usize,
    value: Option<minijinja::Value>,
}

impl MinijinjaEngine {
    /// Creates an empty engine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            env: minijinja::Environment::new(),
            ctx_cache: std::sync::Mutex::new(CtxCache::default()),
        }
    }

    /// Renders without touching the context memoization cache.
    ///
    /// Used by context builders that render many short-lived per-entity
    /// contexts (whose addresses would thrash or falsely hit the cache).
    pub(crate) fn render_once(
        &self,
        template_name: &str,
        ctx: &serde_json::Value,
    ) -> Result<String, GenError> {
        let tmpl = self.env.get_template(template_name)?;
        Ok(tmpl.render(ctx)?)
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
        let mut cache = self.ctx_cache.lock().expect("ctx cache lock");
        let addr = std::ptr::from_ref(ctx).addr();
        if cache.addr != addr || cache.value.is_none() {
            *cache = CtxCache {
                addr,
                value: Some(minijinja::Value::from_serialize(ctx)),
            };
        }
        Ok(tmpl.render(cache.value.clone().expect("cache populated"))?)
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
    parse_manifest, parse_manifest_str, render_manifest,
};
