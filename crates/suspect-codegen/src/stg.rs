//! The Semantic Type Graph: OpenAPI schemas lifted into real type theory.
//!
//! The lift resolves everything templates cannot: `allOf` flattens into
//! composition (with conflict reporting), `oneOf` + discriminator becomes a
//! tagged sum, string `enum`s become enums, and constraint keywords
//! (`pattern`, `format`, `min`/`max`, lengths) fold into refinements each
//! target lowers to branded types / newtypes / validated constructors.
//! Components are emitted in topological order so generated modules never
//! reference types before they exist; cycles keep spec order within their
//! group.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use suspect_gen::split_words;

// ---------------------------------------------------------------- naming

/// A sanitized identifier for one target language.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ident {
    /// Original source text (`pet_id`, `/pets/{petId}`).
    pub original: String,
    /// PascalCase rendering (type/variant names).
    pub pascal: String,
    /// camelCase rendering (fields/functions).
    pub camel: String,
    /// snake_case rendering (Rust fields/functions).
    pub snake: String,
}

impl Ident {
    /// Builds an identifier from arbitrary source text.
    #[must_use]
    pub fn new(original: &str) -> Self {
        let words = split_words(original);
        let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        let pascal = lower
            .iter()
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join("");
        let camel = lower
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if i == 0 {
                    w.clone()
                } else {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("");
        // Rust reserves `r#`; plain snake avoids raw identifiers entirely by
        // suffixing known conflicts is overkill — prefix underscore instead.
        let mut snake = lower.join("_");
        if matches!(
            snake.as_str(),
            "type" | "impl" | "fn" | "let" | "match" | "ref" | "move" | "box" | "where" | "use"
        ) {
            snake.push('_');
        }
        Self {
            original: original.to_owned(),
            pascal,
            camel,
            snake,
        }
    }
}

// ---------------------------------------------------------------- graph

/// Primitive base kind after core-schema resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Base {
    /// Textual content.
    Str,
    /// Integral number.
    Int,
    /// Floating-point number.
    Float,
    /// True/false.
    Bool,
}

/// Constraint refinements folded from validation keywords.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Refinements {
    /// Inclusive/exclusive numeric bounds.
    pub min: Option<f64>,
    /// Inclusive/exclusive numeric bounds.
    pub max: Option<f64>,
    /// Exclusive lower bound.
    pub exclusive_min: Option<f64>,
    /// Exclusive upper bound.
    pub exclusive_max: Option<f64>,
    /// Minimum string length.
    pub min_length: Option<u64>,
    /// Maximum string length.
    pub max_length: Option<u64>,
    /// Anchored pattern constraint.
    pub pattern: Option<String>,
    /// Declared enum values (raw JSON text).
    pub enum_values: Vec<String>,
    /// Semantic format hints we recognize.
    pub format: Option<WellKnownFormat>,
}

/// Formats with cross-language semantic meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WellKnownFormat {
    /// Email address.
    Email,
    /// UUID.
    Uuid,
    /// RFC 3339 timestamp.
    DateTime,
    /// Calendar date.
    Date,
    /// Base64 content.
    Byte,
    /// Binary content.
    Binary,
}

/// A primitive leaf with its refinements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StgPrim {
    /// Core base type.
    pub base: Base,
    /// Folded constraints.
    pub refs: Refinements,
}

/// One object field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StgField {
    /// Sanitized identifier carrying the original name.
    pub ident: Ident,
    /// Field type.
    pub ty: StgType,
    /// Required in requests (present in all instances).
    pub required: bool,
    /// Description docs.
    pub docs: Option<String>,
    /// Deprecated flag.
    pub deprecated: bool,
}

/// A lifted component definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StgNode {
    /// Product type with named fields.
    Struct(StgStruct),
    /// Closed set of string literals.
    StringEnum(StgStringEnum),
    /// Tagged sum (`oneOf` + discriminator): variants carry their
    /// discriminant value and referenced payload.
    Sum(StgSum),
    /// Untagged union (`anyOf` without discriminator).
    Union(StgUnion),
    /// Transparent alias: resolved transparently on use.
    Alias(Box<StgType>),
}

/// Closed string enumeration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StgStringEnum {
    /// Enum name.
    pub name: Ident,
    /// Variants as `(original literal, sanitized ident)` pairs.
    pub variants: Vec<(String, Ident)>,
    /// Docs.
    pub docs: Option<String>,
    /// Deprecated flag.
    pub deprecated: bool,
}

/// Tagged sum type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StgSum {
    /// Sum name.
    pub name: Ident,
    /// Discriminator property name (the tag field).
    pub tag_field: String,
    /// Variants: `(discriminant value, payload component name)`.
    pub variants: Vec<(String, Ident)>,
    /// Docs.
    pub docs: Option<String>,
}

/// Untagged union.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StgUnion {
    /// Union name.
    pub name: Ident,
    /// Member types.
    pub members: Vec<StgType>,
    /// Docs.
    pub docs: Option<String>,
}

/// A product type body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StgStruct {
    /// Struct name.
    pub name: Ident,
    /// Fields in document order.
    pub fields: Vec<StgField>,
    /// Docs.
    pub docs: Option<String>,
    /// Deprecated flag.
    pub deprecated: bool,
}

/// A type reference or inline shape, as it appears in a field/parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StgType {
    /// Reference to a named component in [`Graph::components`].
    Named(String),
    /// Inline anonymous structure.
    InlineStruct(StgStruct),
    /// Inline anonymous string enum.
    InlineEnum(StgStringEnum),
    /// Inline tagged sum.
    InlineSum(StgSum),
    /// Inline untagged union.
    InlineUnion(StgUnion),
    /// List of elements.
    List(Box<StgType>),
    /// Object with additionalProperties of this element type.
    Dict(Box<StgType>),
    /// Primitive leaf.
    Prim(StgPrim),
    /// Optional wrapper (schema not marked required).
    Optional(Box<StgType>),
}

/// An operation parameter grouped by location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpParam {
    /// Parameter name.
    pub name: Ident,
    /// Location (`path`/`query`/`header`).
    pub location: String,
    /// Required flag.
    pub required: bool,
    /// Parameter schema.
    pub ty: StgType,
}

/// One synthesized API operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpModel {
    /// Operation id (sanitized identifier).
    pub op_id: Ident,
    /// HTTP method (uppercase).
    pub method: String,
    /// Path template including `{param}` placeholders.
    pub path_template: String,
    /// Parameters grouped by location, path first then query.
    pub params_path: Vec<OpParam>,
    pub params_query: Vec<OpParam>,
    pub params_header: Vec<OpParam>,
    /// Request body component name, when present.
    pub request_body: Option<String>,
    /// Declared responses: `(status or "default", component name)`.
    pub responses: Vec<(String, String)>,
    /// Summary docs.
    pub summary: Option<String>,
    /// Deprecated flag.
    pub deprecated: bool,
    /// Tags.
    pub tags: Vec<String>,
}

/// The compiled semantic graph for one specification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Graph {
    /// Component nodes keyed by component name, in topological order
    /// (dependencies before dependents; cycle groups keep spec order).
    pub components: BTreeMap<String, StgNode>,
    /// Topological component-name order.
    pub topo_order: Vec<String>,
    /// Synthesized operations.
    pub operations: Vec<OpModel>,
    /// Non-fatal lifting notes (allOf field conflicts, skipped shapes).
    pub warnings: Vec<String>,
}
