//! suspect-oas: typed OpenAPI 3.0/3.1/3.2 models as lazy views.
//!
//! Views borrow a [`Session`] which pins every loaded document in an
//! append-only arena, so all [`NodeRef`]s derived from it stay valid for the
//! session borrow — sound, allocation-light, no `Rc`/`RefCell` in the model.
//! `$ref`s resolve transparently through the [`Workspace`](suspect_ref::Workspace);
//! cycles surface as [`CycleGuard`] markers rather than errors or loops.

mod components;
mod model;
mod paths;
mod schema;
mod session;

pub use components::Components;
pub use model::{
    Callback, Contact, Discriminator, Encoding, Example, ExternalDocumentation, Header, Info,
    License, Link, MediaType, OauthFlow, OauthFlows, Parameter, ParameterIn, ParameterStyle,
    RequestBody, Response, SecurityRequirement, SecurityScheme, SecuritySchemeType, Server,
    ServerVariable, Tag, Xml,
};
mod openapi;

pub use schema::{SchemaView, TypeSet};
pub use session::{CycleGuard, ModelError, OasVersion, OpenApi, Session};

use suspect_low::ValueKind;

/// Deep structural equality between two nodes (objects order-insensitive,
/// arrays order-sensitive, scalars by semantic kind).
#[must_use]
pub fn node_eq(a: suspect_low::NodeRef<'_>, b: suspect_low::NodeRef<'_>) -> bool {
    match (a.kind(), b.kind()) {
        (ValueKind::Object, ValueKind::Object) => {
            let ae = a.entries();
            if ae.len() != b.entries().len() {
                return false;
            }
            ae.iter().all(|e| b.get(e.key).is_some_and(|v| node_eq(e.value.unwrap(), v)))
        }
        (ValueKind::Array, ValueKind::Array) => {
            let ai = a.items();
            let bi = b.items();
            ai.len() == bi.len() && ai.iter().zip(bi.iter()).all(|(x, y)| node_eq(*x, *y))
        }
        (ValueKind::Null, ValueKind::Null) => true,
        (ValueKind::Bool, ValueKind::Bool) => a.as_bool() == b.as_bool(),
        (ValueKind::Int, ValueKind::Int) => a.as_i64() == b.as_i64(),
        (ValueKind::Float, ValueKind::Float) | (ValueKind::Int, ValueKind::Float)
        | (ValueKind::Float, ValueKind::Int) => {
            a.as_f64().zip(b.as_f64()).is_some_and(|(x, y)| x == y)
        }
        (ValueKind::Str, ValueKind::Str) => a.scalar_bytes() == b.scalar_bytes(),
        _ => false,
    }
}
