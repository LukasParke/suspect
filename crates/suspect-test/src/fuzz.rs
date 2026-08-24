//! Deterministic schema-driven mutant generation for `suspect fuzz`.
//!
//! A *mutant* replaces one scalar input field's value with a pathological
//! value (wrong type, empty, oversize, negative, unicode bomb, or null).
//! Generation is fully deterministic: fields are walked in schema order and
//! a run counter cycles `(field, kind)` pairs — no randomness, so a failing
//! mutant can always be replayed exactly.
//!
//! Object/array schemas recurse **one level deep only**: nested properties'
//! own properties become additional targets (`/outer/inner`) but nothing
//! below that.

use serde_json::Value;

/// The six mutation kinds cycled per field, in application order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MutantKind {
    /// Value whose JSON type differs from the schema type.
    WrongType,
    /// Empty string / zero / empty container.
    Empty,
    /// 512-character lowercase-`a` string.
    Oversize,
    /// `-1`.
    Negative,
    /// 64 repetitions of the 🧊 emoji.
    UnicodeBomb,
    /// JSON `null` where the field is declared required.
    NullRequired,
}

/// All [`MutantKind`] variants in cycle order.
pub const MUTANT_KINDS: [MutantKind; 6] = [
    MutantKind::WrongType,
    MutantKind::Empty,
    MutantKind::Oversize,
    MutantKind::Negative,
    MutantKind::UnicodeBomb,
    MutantKind::NullRequired,
];

impl MutantKind {
    /// Stable machine-readable name (used in reports).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::WrongType => "wrong-type",
            Self::Empty => "empty",
            Self::Oversize => "oversize",
            Self::Negative => "negative",
            Self::UnicodeBomb => "unicode-bomb",
            Self::NullRequired => "null-required",
        }
    }
}

/// One scalar leaf target extracted from an input schema.
///
/// `pointer` is the JSON-pointer-style location of the field relative to its
/// request part (`""`-rooted objects yield `/field`, one recursion level
/// yields `/outer/inner`); `name` is the same pointer kept as an opaque key
/// for reports. Path/query parameters use their parameter name.
#[derive(Debug, Clone, PartialEq)]
pub struct ScalarField {
    /// Field address (parameter name or `/`-prefixed body pointer).
    pub name: String,
    /// Schema of the field, used to pick mutant and default values.
    pub schema: Value,
    /// Whether the source schema declares the field required.
    pub required: bool,
}

/// One generated mutation: which field, which kind, which value.
#[derive(Debug, Clone, PartialEq)]
pub struct Mutant {
    /// Address of the mutated field (matches [`ScalarField::name`]).
    pub field: String,
    /// Mutation applied.
    pub kind: MutantKind,
    /// Replacement value to place at the field.
    pub value: Value,
}

/// Extracts scalar leaf fields from `schema`, recursing into object
/// properties and array items exactly one level: top-level fields come from
/// the root schema, and each nested object/array property contributes its
/// own immediate leaves (`/outer/inner`) — nothing below that.
#[must_use]
pub fn scalar_fields(schema: &Value) -> Vec<ScalarField> {
    let mut out = Vec::new();
    collect("", schema, 2, true, &mut out);
    out
}

fn collect(
    prefix: &str,
    schema: &Value,
    depth_left: u32,
    required: bool,
    out: &mut Vec<ScalarField>,
) {
    if depth_left == 0 {
        return;
    }
    let ty = schema_type(schema);
    match ty {
        "object" => {
            let Some(props) = schema.get("properties").and_then(Value::as_object) else {
                return;
            };
            let required_names: Vec<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            for (key, child) in props {
                let child_prefix = format!("{prefix}/{key}");
                let child_ty = schema_type(child);
                if matches!(child_ty, "object" | "array") {
                    if depth_left > 1 {
                        collect(
                            &child_prefix,
                            child,
                            depth_left - 1,
                            required_names.contains(&key.as_str()),
                            out,
                        );
                    }
                } else {
                    out.push(ScalarField {
                        name: child_prefix,
                        schema: child.clone(),
                        required: required_names.contains(&key.as_str()),
                    });
                }
            }
        }
        // An array is transparent: descending through it into `items`
        // counts as the same nesting hop as the array itself.
        "array" => {
            if let Some(items) = schema.get("items") {
                let items_ty = schema_type(items);
                if matches!(items_ty, "object" | "array") {
                    collect(&format!("{prefix}/0"), items, depth_left, required, out);
                } else {
                    out.push(ScalarField {
                        name: format!("{prefix}/0"),
                        schema: items.clone(),
                        required,
                    });
                }
            }
        }
        _ => {
            out.push(ScalarField {
                name: prefix.to_owned(),
                schema: schema.clone(),
                required,
            });
        }
    }
}

fn schema_type(schema: &Value) -> &str {
    schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("string")
}

/// The mutant replacement value for one field schema + kind.
#[must_use]
pub fn mutant_value(schema: &Value, kind: MutantKind) -> Value {
    match kind {
        MutantKind::WrongType => match schema_type(schema) {
            "integer" | "number" => Value::String("fuzz".into()),
            "boolean" => Value::String("yes".into()),
            "array" | "object" => Value::from(7),
            _ => Value::from(42),
        },
        MutantKind::Empty => match schema_type(schema) {
            "integer" | "number" => Value::from(0),
            "boolean" => Value::Bool(false),
            "array" => Value::Array(Vec::new()),
            "object" => Value::Object(serde_json::Map::new()),
            _ => Value::String(String::new()),
        },
        // An oversize string against any schema: string-typed fields get the
        // classic length violation, everything else additionally wrong-typed.
        MutantKind::Oversize => Value::String("a".repeat(512)),
        MutantKind::Negative => Value::from(-1),
        MutantKind::UnicodeBomb => Value::String("\u{1F9CA}".repeat(64)),
        MutantKind::NullRequired => Value::Null,
    }
}

/// Benign default used for fields not targeted by the current mutant.
#[must_use]
pub fn default_value(schema: &Value) -> Value {
    match schema_type(schema) {
        "integer" | "number" => Value::from(1),
        "boolean" => Value::Bool(true),
        "array" => Value::Array(Vec::new()),
        "object" => Value::Object(serde_json::Map::new()),
        _ => Value::String("suspect".into()),
    }
}

/// Generates `runs` mutants cycling deterministically through
/// `(fields[i % n], MUTANT_KINDS[i / n % 6])`. Returns an empty vector when
/// `fields` is empty.
#[must_use]
pub fn generate_mutants(fields: &[ScalarField], runs: usize) -> Vec<Mutant> {
    if fields.is_empty() {
        return Vec::new();
    }
    let n = fields.len();
    (0..runs)
        .map(|i| {
            let field = &fields[i % n];
            let kind = MUTANT_KINDS[(i / n) % MUTANT_KINDS.len()];
            Mutant {
                field: field.name.clone(),
                kind,
                value: mutant_value(&field.schema, kind),
            }
        })
        .collect()
}

/// Builds a complete payload object for one run: every field of `fields`
/// gets its benign default except the mutated field, which receives
/// [`Mutant::value`] at its pointer (one nesting level supported). A
/// null-valued mutant is inserted verbatim — sending `null` *is* the
/// mutation under test.
#[must_use]
pub fn payload(fields: &[ScalarField], mutant: &Mutant) -> Value {
    let mut root = serde_json::Map::new();
    for f in fields {
        let mut segments = f.name.split('/').filter(|s| !s.is_empty());
        let Some(top) = segments.next() else { continue };
        let v = if f.name == mutant.field {
            mutant.value.clone()
        } else {
            default_value(&f.schema)
        };
        match segments.next() {
            None => {
                root.insert(top.to_owned(), v);
            }
            Some(inner) => {
                let entry = root
                    .entry(top.to_owned())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !entry.is_object() {
                    *entry = Value::Object(serde_json::Map::new());
                }
                entry
                    .as_object_mut()
                    .expect("just ensured object")
                    .insert(inner.to_owned(), v);
            }
        }
    }
    Value::Object(root)
}
