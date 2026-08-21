//! suspect-arazzo: Arazzo 1.0 models, runtime expressions, validation.
//!
//! The workflow *execution* engine is out of scope by design; this crate
//! covers document models, the full runtime-expression grammar (parser +
//! evaluator over a caller-supplied [`RuntimeContext`]), and structural /
//! cross-reference validation.

mod expr;
mod model;
mod validate;

pub use expr::{parse, parse_embedded, ComponentKind, Expr, ExprError, ExprPart, HttpPart};
pub use model::{
    ActionView, ArazzoDoc, CriterionView, ParameterView, SourceDescriptionView, SourceType,
    StepView, WorkflowView,
};
pub use validate::{validate_arazzo, ArazzoDiagnostic};

use suspect_low::NodeRef;

pub enum Evaluated<'d> {
    /// A scalar rendered as text (headers, query params, statuses, outputs).
    Text(String),
    /// A resolved payload node (`$request.body#/pointer` et al).
    Body(NodeRef<'d>),
}

impl std::fmt::Debug for Evaluated<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(t) => f.debug_tuple("Text").field(t).finish(),
            Self::Body(n) => f.debug_tuple("Body").field(&n.byte_range()).finish(),
        }
    }
}

/// Execution-context abstraction: production engines implement it over HTTP
/// traffic; validation uses mocks.
pub trait RuntimeContext<'d> {
    fn method(&self) -> Option<&str> {
        None
    }
    fn url(&self) -> Option<&str> {
        None
    }
    fn status_code(&self) -> Option<i64> {
        None
    }
    fn header(&self, _response: bool, _name: &str) -> Option<&'d str> {
        None
    }
    fn query(&self, _response: bool, _name: &str) -> Option<&'d str> {
        None
    }
    fn path_param(&self, _response: bool, _name: &str) -> Option<&'d str> {
        None
    }
    fn body(&self, _response: bool) -> Option<NodeRef<'d>> {
        None
    }
    fn output(&self, _name: &str) -> Option<&'d str> {
        None
    }
    fn input(&self, _name: &str) -> Option<&'d str> {
        None
    }
    fn component(&self, _kind: ComponentKind, _name: &str) -> Option<&'d str> {
        None
    }
    fn workflow_output(&self, _workflow: &str, _step: &str, _name: &str) -> Option<&'d str> {
        None
    }
}

/// Evaluates a parsed expression against a context.
pub fn evaluate<'d>(expr: &Expr, ctx: &dyn RuntimeContext<'d>) -> Option<Evaluated<'d>> {
    match expr {
        Expr::Method => ctx.method().map(|m| Evaluated::Text(m.to_owned())),
        Expr::Url => ctx.url().map(|u| Evaluated::Text(u.to_owned())),
        Expr::StatusCode => ctx.status_code().map(|c| Evaluated::Text(c.to_string())),
        Expr::Request { part } | Expr::Response { part } => {
            let response = matches!(expr, Expr::Response { .. });
            Some(match part {
                HttpPart::Header(name) => Evaluated::Text(ctx.header(response, name)?.to_owned()),
                HttpPart::Query(name) => Evaluated::Text(ctx.query(response, name)?.to_owned()),
                HttpPart::Path(name) => Evaluated::Text(ctx.path_param(response, name)?.to_owned()),
                HttpPart::Body(pointer) => {
                    let body = ctx.body(response)?;
                    match pointer {
                        Some(p) => Evaluated::Body(body.pointer(p)?),
                        None => Evaluated::Body(body),
                    }
                }
            })
        }
        Expr::Outputs { name } => ctx.output(name).map(|o| Evaluated::Text(o.to_owned())),
        Expr::Inputs { name } => ctx.input(name).map(|i| Evaluated::Text(i.to_owned())),
        Expr::Component { kind, name } => {
            ctx.component(*kind, name).map(|o| Evaluated::Text(o.to_owned()))
        }
        Expr::WorkflowOutput { workflow, step, name } => ctx
            .workflow_output(workflow, step, name)
            .map(|o| Evaluated::Text(o.to_owned())),
        Expr::SourceDescription { .. } => None,
        Expr::Text(_) => None, // literal text is not an evaluatable expression
    }
}

/// Renders an evaluated value as text; bodies render via their raw slice.
#[must_use]
pub fn render(value: &Evaluated<'_>) -> String {
    match value {
        Evaluated::Text(t) => t.clone(),
        Evaluated::Body(n) => match n.kind() {
            suspect_low::ValueKind::Str => {
                String::from_utf8_lossy(n.scalar_bytes()).to_string()
            }
            _ => String::from_utf8_lossy(n.raw_text()).to_string(),
        },
    }
}

/// Evaluates an embedded expression string (`"see {$response.body#/id}"`)
/// producing concatenated text; unmatched parts render as empty.
#[must_use]
pub fn render_embedded(input: &str, ctx: &dyn RuntimeContext<'_>) -> String {
    let mut out = String::new();
    for part in parse_embedded(input) {
        match part {
            ExprPart::Text(t) => out.push_str(&t),
            ExprPart::Expr(e) => {
                if let Some(v) = evaluate(&e, ctx) {
                    out.push_str(&render(&v));
                }
            }
        }
    }
    out
}
