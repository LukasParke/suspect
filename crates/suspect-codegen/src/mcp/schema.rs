//! Bounded projection of one admitted native model expression into an MCP
//! JSON Schema (2020-12) plus the matching value adapter.
//!
//! The projection walks exactly the native representation the generated
//! TypeScript SDK already admitted - never a re-read source schema - so a tool
//! surface can never claim something the codecs do not implement.
//!
//! Two rules govern the whole projection:
//!
//! 1. The projected schema never admits a JSON number anywhere. A tool's
//!    arguments reach the server through the MCP client's own JSON parser, so
//!    a real JSON number would already have been rounded before the server saw
//!    it. Every schema-declared numeric position is therefore projected as a
//!    string holding one exact JSON number token, and the generated codec - not
//!    this projection - converts it to its native representation. Numeric
//!    source assertions are never copied onto that string.
//! 2. A projected position never admits one JSON value with two possible
//!    meanings. String-or-number unions, arbitrary JSON, intersections and
//!    recursion are refused during generation instead of being coerced.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use suspect_ir::contract::SchemaId;

use crate::typescript::{Expr, Literal, ModelSymbol, Primitive};

use super::Representation;

/// JSON number grammar, as the exact-token surface accepts it. This is the
/// representation grammar only: no declared numeric assertion is ever copied.
const NUMBER_PATTERN: &str = r"^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?$";

/// Ceiling on projected schema nodes, so a widely shared model closure cannot
/// expand into an unbounded inlined schema.
const MAX_NODES: usize = 4096;

/// One admitted projection: what a client sees, and how one argument becomes an
/// exact JSON value the generated codec can decode.
#[derive(Debug)]
pub(super) struct Projection {
    pub schema: Value,
    pub adapter: Adapter,
    pub representation: Representation,
}

/// A located projection refusal. `source` is the innermost model source the
/// projection had reached, so the diagnostic points at the actual schema.
#[derive(Debug)]
pub(super) struct Refusal {
    pub source: Option<SchemaId>,
    pub message: String,
}

/// How one projected JSON value becomes an exact JSON value for a codec.
/// Equality is structural: union alternatives that project to the same surface
/// kind are admitted only when their adapters are identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Adapter {
    /// A JSON string is itself.
    Text,
    Boolean,
    Null,
    /// A JSON string holds one exact JSON number token.
    Number,
    Array(Box<Adapter>),
    Object {
        fields: BTreeMap<String, Adapter>,
        extra: Option<Box<Adapter>>,
    },
    /// Dispatched by the argument's own JSON value kind.
    Union(Box<Branches>),
}

/// The admitted alternatives of one projected union. `text` and `number` are
/// mutually exclusive: both would accept a JSON string with two meanings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Branches {
    pub null: bool,
    pub boolean: bool,
    pub text: bool,
    pub number: bool,
    pub array: Option<Adapter>,
    pub object: Option<Adapter>,
}

impl Adapter {
    /// The adapter as the literal the generated server embeds.
    pub(super) fn literal(&self) -> Value {
        match self {
            Self::Text => json!({"kind": "text"}),
            Self::Boolean => json!({"kind": "boolean"}),
            Self::Null => json!({"kind": "null"}),
            Self::Number => json!({"kind": "number"}),
            Self::Array(item) => json!({"kind": "array", "item": item.literal()}),
            Self::Object { fields, extra } => json!({
                "kind": "object",
                "fields": fields
                    .iter()
                    .map(|(name, adapter)| (name.clone(), adapter.literal()))
                    .collect::<serde_json::Map<_, _>>(),
                "extra": extra.as_ref().map_or(Value::Null, |extra| extra.literal()),
            }),
            Self::Union(branches) => json!({
                "kind": "union",
                "null": branches.null,
                "boolean": branches.boolean,
                "text": branches.text,
                "number": branches.number,
                "array": branches.array.as_ref().map_or(Value::Null, Adapter::literal),
                "object": branches.object.as_ref().map_or(Value::Null, Adapter::literal),
            }),
        }
    }

    fn representation(&self) -> Representation {
        match self {
            Self::Text => Representation::String,
            Self::Boolean => Representation::Boolean,
            Self::Null => Representation::Null,
            Self::Number => Representation::NumberToken,
            Self::Array(_) => Representation::Array,
            Self::Object { .. } => Representation::Object,
            Self::Union(_) => Representation::Union,
        }
    }
}

/// Project `root` for one tool input position.
///
/// `symbols` resolves the native reference names the expression carries, keyed
/// by allocated model name.
pub(super) fn project(
    symbols: &BTreeMap<&str, &ModelSymbol>,
    root: &Expr,
) -> Result<Projection, Refusal> {
    let mut projector = Projector {
        symbols,
        stack: Vec::new(),
        nodes: 0,
    };
    let (schema, adapter) = projector.walk(root, None)?;
    Ok(Projection {
        representation: adapter.representation(),
        schema,
        adapter,
    })
}

struct Projector<'a> {
    symbols: &'a BTreeMap<&'a str, &'a ModelSymbol>,
    /// Reference names currently being expanded, so recursion is detected
    /// rather than expanded forever.
    stack: Vec<String>,
    nodes: usize,
}

fn refuse<T>(at: Option<&SchemaId>, message: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal {
        source: at.cloned(),
        message: message.into(),
    })
}

impl Projector<'_> {
    fn walk(&mut self, expr: &Expr, at: Option<&SchemaId>) -> Result<(Value, Adapter), Refusal> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return refuse(
                at,
                format!(
                    "the projected tool input schema exceeds {MAX_NODES} nodes; expose a narrower operation"
                ),
            );
        }
        match expr {
            Expr::At(source, inner) => self.walk(inner, Some(source)),
            Expr::Any => refuse(
                at,
                "arbitrary JSON has no tool input projection: its numbers would already be rounded by the client's own JSON parser before the server could preserve them",
            ),
            Expr::Never => refuse(at, "an uninhabited schema has no tool input representation"),
            Expr::Primitive(Primitive::Null) => Ok((json!({"type": "null"}), Adapter::Null)),
            Expr::Primitive(Primitive::Boolean) => {
                Ok((json!({"type": "boolean"}), Adapter::Boolean))
            }
            Expr::Primitive(Primitive::String) => Ok((json!({"type": "string"}), Adapter::Text)),
            Expr::Primitive(kind) => Ok((number(*kind, None), Adapter::Number)),
            Expr::Literal(Literal::Boolean(value)) => {
                Ok((json!({"type": "boolean", "const": value}), Adapter::Boolean))
            }
            Expr::Literal(Literal::String(value)) => {
                Ok((json!({"type": "string", "const": value}), Adapter::Text))
            }
            Expr::Literal(Literal::Integer { value, safe }) => Ok((
                number(
                    if *safe {
                        Primitive::SafeInteger
                    } else {
                        Primitive::Integer
                    },
                    Some(value),
                ),
                Adapter::Number,
            )),
            Expr::Array(item) => {
                let (schema, adapter) = self.walk(item, at)?;
                Ok((
                    json!({"type": "array", "items": schema}),
                    Adapter::Array(Box::new(adapter)),
                ))
            }
            Expr::Object(declared, extra) => {
                let mut properties = serde_json::Map::new();
                let mut required = Vec::new();
                let mut fields = BTreeMap::new();
                for field in declared {
                    let (schema, adapter) = self.walk(&field.expression, at)?;
                    properties.insert(field.name.clone(), schema);
                    if field.required {
                        required.push(Value::from(field.name.clone()));
                    }
                    fields.insert(field.name.clone(), adapter);
                }
                // An undeclared member carries no declared schema, so the
                // projection can only expose it when its native representation
                // is itself exactly projectable. Otherwise the tool surface is
                // explicitly closed - visibly, in the published schema - rather
                // than admitting a value it could not keep exact.
                let (additional, adapter, closed) = match extra {
                    None => (Value::from(false), None, None),
                    Some(extra) => match self.walk(extra, at) {
                        Ok((schema, adapter)) => (schema, Some(Box::new(adapter)), None),
                        Err(refusal) => (Value::from(false), None, Some(refusal.message)),
                    },
                };
                let mut schema = json!({
                    "type": "object",
                    "properties": properties,
                    "additionalProperties": additional,
                });
                if !required.is_empty() {
                    schema["required"] = Value::from(required);
                }
                if let Some(reason) = closed {
                    schema["description"] = Value::from(format!(
                        "Only the properties declared here are exposed. The source schema also admits undeclared properties, which this tool surface does not carry: {reason}"
                    ));
                }
                Ok((
                    schema,
                    Adapter::Object {
                        fields,
                        extra: adapter,
                    },
                ))
            }
            Expr::Union(alternatives) => {
                let mut schemas = Vec::new();
                let mut branches = Branches::default();
                for alternative in alternatives {
                    let (schema, adapter) = self.walk(alternative, at)?;
                    schemas.push(schema);
                    merge(&mut branches, adapter, at)?;
                }
                Ok((json!({"anyOf": schemas}), collapse(branches)))
            }
            Expr::Intersection(_) => refuse(
                at,
                "an intersection schema has no unambiguous tool input projection; expose a single concrete schema instead",
            ),
            Expr::Reference(name) => {
                if self.stack.iter().any(|entry| entry == name) {
                    return refuse(
                        at,
                        format!(
                            "model {name} is recursive, and a recursive schema has no finite tool input projection"
                        ),
                    );
                }
                let Some(symbol) = self.symbols.get(name.as_str()) else {
                    return refuse(
                        at,
                        format!("model {name} is not part of the generated SDK's model closure"),
                    );
                };
                self.stack.push(name.clone());
                let projected = self.walk(symbol.expression(), Some(symbol.source()));
                self.stack.pop();
                projected
            }
        }
    }
}

/// The exact-token string projection of one numeric position. The declared
/// value, when a literal declared one, is named in prose; no numeric assertion
/// is ever copied onto the string.
fn number(kind: Primitive, literal: Option<&str>) -> Value {
    let native = match kind {
        Primitive::SafeInteger => "a native safe integer",
        Primitive::Integer => "a native bigint integer",
        Primitive::Number => "an exact JsonNumber decimal",
        _ => "the declared native numeric representation",
    };
    let mut description = format!(
        "Supplied as a string holding one exact JSON number token, for example \"-12\" or \"9007199254740993\". The generated codec converts the token to {native}, so a large or high-precision value is never routed through a floating-point number."
    );
    if let Some(value) = literal {
        description.push_str(&format!(
            " The source declares exactly the value {value}; any spelling of that same number is accepted."
        ));
    }
    json!({"type": "string", "pattern": NUMBER_PATTERN, "description": description})
}

/// Admit one more alternative into a projected union, or refuse the union
/// because two alternatives would accept the same JSON value differently.
fn merge(branches: &mut Branches, adapter: Adapter, at: Option<&SchemaId>) -> Result<(), Refusal> {
    match adapter {
        Adapter::Null => branches.null = true,
        Adapter::Boolean => branches.boolean = true,
        Adapter::Text => {
            if branches.number {
                return refuse(
                    at,
                    "this schema admits both a string and a number, so one JSON string at the tool boundary would have two meanings; expose a single concrete type instead",
                );
            }
            branches.text = true;
        }
        Adapter::Number => {
            if branches.text {
                return refuse(
                    at,
                    "this schema admits both a number and a string, so one JSON string at the tool boundary would have two meanings; expose a single concrete type instead",
                );
            }
            branches.number = true;
        }
        Adapter::Array(item) => match &branches.array {
            None => branches.array = Some(Adapter::Array(item)),
            Some(Adapter::Array(existing)) if **existing == *item => {}
            Some(_) => {
                return refuse(
                    at,
                    "this schema admits two different array representations, so one JSON array at the tool boundary would have two meanings",
                );
            }
        },
        object @ Adapter::Object { .. } => match &branches.object {
            None => branches.object = Some(object),
            Some(existing) if *existing == object => {}
            Some(_) => {
                return refuse(
                    at,
                    "this schema admits two different object representations, so one JSON object at the tool boundary would have two meanings",
                );
            }
        },
        Adapter::Union(nested) => {
            let Branches {
                null,
                boolean,
                text,
                number,
                array,
                object,
            } = *nested;
            if null {
                merge(branches, Adapter::Null, at)?;
            }
            if boolean {
                merge(branches, Adapter::Boolean, at)?;
            }
            if text {
                merge(branches, Adapter::Text, at)?;
            }
            if number {
                merge(branches, Adapter::Number, at)?;
            }
            if let Some(array) = array {
                merge(branches, array, at)?;
            }
            if let Some(object) = object {
                merge(branches, object, at)?;
            }
        }
    }
    Ok(())
}

/// A union whose alternatives all reduce to one surface kind is that kind:
/// a closed string enumeration is a string, not a runtime dispatch.
fn collapse(branches: Branches) -> Adapter {
    let kinds = usize::from(branches.null)
        + usize::from(branches.boolean)
        + usize::from(branches.text)
        + usize::from(branches.number)
        + usize::from(branches.array.is_some())
        + usize::from(branches.object.is_some());
    if kinds != 1 {
        return Adapter::Union(Box::new(branches));
    }
    if branches.null {
        return Adapter::Null;
    }
    if branches.boolean {
        return Adapter::Boolean;
    }
    if branches.text {
        return Adapter::Text;
    }
    if branches.number {
        return Adapter::Number;
    }
    branches
        .array
        .or(branches.object)
        .expect("exactly one admitted union branch")
}
