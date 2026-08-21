use suspect_low::{NodeRef, ValueKind};

use crate::model::{Discriminator, ExternalDocumentation, Xml};
use crate::session::{CycleGuard, Session};

/// A Schema Object view (3.0 dialect or 3.1+ JSON Schema), resolving `$ref`
/// transparently.
#[derive(Clone, Copy)]
pub struct SchemaView<'s> {
    session: &'s Session,
    node: NodeRef<'s>,
    /// Set when a `$ref` chain cycled and we kept the raw view.
    cyclic: bool,
}

impl<'s> SchemaView<'s> {
    pub(crate) fn new(session: &'s Session, node: NodeRef<'s>) -> Self {
        Self { session, node, cyclic: false }
    }

    /// The raw node this view points at (before any `$ref` resolution).
    #[must_use]
    pub fn node(&self) -> NodeRef<'s> {
        self.node
    }

    #[must_use]
    /// True when this view kept its raw form because its `$ref` chain
    /// cycled ([`SchemaView::resolved`] hit a [`CycleGuard`]).
    pub const fn is_cyclic(&self) -> bool {
        self.cyclic
    }

    fn get(&self, key: &str) -> Option<NodeRef<'s>> {
        self.node.get(key)
    }

    /// Follows this schema's own `$ref` (if any) to its target.
    #[must_use]
    pub fn resolved(&self) -> Self {
        if self.cyclic {
            return *self;
        }
        match self.get("$ref") {
            Some(ref_value) => match self.session.resolve(ref_value) {
                Ok(node) => {
                    let out = Self::new(self.session, node);
                    // target may itself be a ref object; callers chain resolved()
                    if out.has_own_ref() && !out.cyclic {
                        out.resolved()
                    } else {
                        out
                    }
                }
                Err(CycleGuard) => Self { cyclic: true, ..*self },
            },
            None => *self,
        }
    }

    fn has_own_ref(&self) -> bool {
        self.get("$ref").is_some()
    }

    /// Declared type set. 3.0 `type` is a single string; 3.1 allows arrays;
    /// 3.0 `nullable: true` folds into the set as NULL.
    #[must_use]
    pub fn type_(&self) -> Option<TypeSet> {
        let r = self.resolved();
        let mut set = TypeSet::empty();
        match r.get("type") {
            Some(t) => match t.kind() {
                ValueKind::Str => {
                    set.insert_str(t.as_str()?);
                }
                ValueKind::Array => {
                    for item in t.items() {
                        if let Some(s) = item.as_str() {
                            set.insert_str(s);
                        }
                    }
                }
                _ => {}
            },
            None => {
                // infer from sibling keywords
                if r.get("properties").is_some()
                    || r.get("additionalProperties").is_some()
                    || r.get("required").is_some()
                    || r.get("patternProperties").is_some()
                {
                    set.insert(TypeSet::OBJECT);
                }
                if r.get("items").is_some() || r.get("prefixItems").is_some() {
                    set.insert(TypeSet::ARRAY);
                }
            }
        }
        if r.nullable() {
            set.insert(TypeSet::NULL);
        }
        (!set.is_empty()).then_some(set)
    }
    /// 3.0 `nullable` flag. On 3.1 documents the keyword does not occur
    /// (nullability lives in `type`), so this reads harmlessly wherever
    /// present.
    #[must_use]
    pub fn nullable(&self) -> bool {
        self.resolved().get("nullable").and_then(|n| n.as_bool()).unwrap_or(false)
    }


    #[must_use]
    /// Prose describing the schema's purpose.
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }

    #[must_use]
    /// Default instance validating against this schema.
    pub fn default(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("default")
    }

    #[must_use]
    /// Single inline example value.
    pub fn example(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("example")
    }

    #[must_use]
    /// Inline example values from the 3.1-style array keyword; empty when
    /// absent or not an array.
    pub fn examples(&self) -> Vec<NodeRef<'s>> {
        self.resolved().get("examples").map(|n| n.items()).unwrap_or_default()
    }

    #[must_use]
    /// Allowed values from the `enum` keyword, in document order.
    pub fn enum_values(&self) -> Vec<NodeRef<'s>> {
        self.resolved().get("enum").map(|n| n.items()).unwrap_or_default()
    }

    #[must_use]
    /// The single allowed value from the `const` keyword.
    pub fn const_value(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("const")
    }

    /// `required` property names.
    #[must_use]
    pub fn required(&self) -> Vec<&'s str> {
        self.resolved()
            .get("required")
            .map(|n| n.items().into_iter().filter_map(|i| i.as_str()).collect())
            .unwrap_or_default()
    }

    /// `(name, schema)` pairs from `properties`.
    #[must_use]
    pub fn properties(&self) -> Vec<(&'s str, SchemaView<'s>)> {
        self.resolved()
            .get("properties")
            .map(|n| {
                n.entries()
                    .into_iter()
                    .filter_map(|e| e.value.map(|v| (e.key, SchemaView::new(self.session, v))))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[must_use]
    /// Subschema of one named property; `None` when absent.
    pub fn property(&self, name: &str) -> Option<SchemaView<'s>> {
        self.resolved()
            .get("properties")?
            .get(name)
            .map(|v| SchemaView::new(self.session, v))
    }

    /// `items` — element subschema (3.0 object form or 3.1 subschema).
    #[must_use]
    pub fn items(&self) -> Option<SchemaView<'s>> {
        self.resolved().get("items").map(|v| SchemaView::new(self.session, v))
    }

    #[must_use]
    /// Tuple-position subschemas from the 3.1 `prefixItems` keyword.
    pub fn prefix_items(&self) -> Vec<SchemaView<'s>> {
        self.resolved()
            .get("prefixItems")
            .map(|n| n.items().into_iter().map(|v| SchemaView::new(self.session, v)).collect())
            .unwrap_or_default()
    }

    fn keyword_schemas(&self, key: &str) -> Vec<SchemaView<'s>> {
        self.resolved()
            .get(key)
            .map(|n| n.items().into_iter().map(|v| SchemaView::new(self.session, v)).collect())
            .unwrap_or_default()
    }

    #[must_use]
    /// Conjunction branches (`allOf`), in document order.
    pub fn all_of(&self) -> Vec<SchemaView<'s>> {
        self.keyword_schemas("allOf")
    }

    #[must_use]
    /// Disjunction branches (`anyOf`), in document order.
    pub fn any_of(&self) -> Vec<SchemaView<'s>> {
        self.keyword_schemas("anyOf")
    }

    #[must_use]
    /// Exclusive-choice branches (`oneOf`), in document order.
    pub fn one_of(&self) -> Vec<SchemaView<'s>> {
        self.keyword_schemas("oneOf")
    }

    #[must_use]
    /// Negated subschema (`not`). `None` when absent.
    pub fn not(&self) -> Option<SchemaView<'s>> {
        self.resolved().get("not").map(|v| SchemaView::new(self.session, v))
    }

    #[must_use]
    /// Polymorphism discriminator attached to this schema.
    pub fn discriminator(&self) -> Option<Discriminator<'s>> {
        self.resolved().get("discriminator").map(|n| Discriminator::new(self.session, n))
    }

    #[must_use]
    /// XML serialization metadata for this schema.
    pub fn xml(&self) -> Option<Xml<'s>> {
        self.resolved().get("xml").map(|n| Xml::new(self.session, n))
    }

    #[must_use]
    /// External documentation reference for this schema.
    pub fn external_docs(&self) -> Option<ExternalDocumentation<'s>> {
        self.resolved().get("externalDocs").map(|n| ExternalDocumentation::new(self.session, n))
    }

    #[must_use]
    /// True when explicitly marked deprecated.
    pub fn deprecated(&self) -> bool {
        self.resolved().get("deprecated").and_then(|n| n.as_bool()).unwrap_or(false)
    }

    #[must_use]
    /// True for response-only properties (`readOnly`).
    pub fn read_only(&self) -> bool {
        self.resolved().get("readOnly").and_then(|n| n.as_bool()).unwrap_or(false)
    }

    #[must_use]
    /// True for request-only properties (`writeOnly`).
    pub fn write_only(&self) -> bool {
        self.resolved().get("writeOnly").and_then(|n| n.as_bool()).unwrap_or(false)
    }

    // numeric bounds
    /// Inclusive upper numeric bound (`maximum`).
    #[must_use]
    pub fn maximum(&self) -> Option<f64> {
        self.resolved().get("maximum").and_then(|n| n.as_f64())
    }
    /// Strict upper numeric bound (`exclusiveMaximum`). Only the numeric
    /// form is reported; the 3.0 boolean-modifier spelling is ignored.
    #[must_use]
    pub fn exclusive_maximum(&self) -> Option<f64> {
        self.resolved().get("exclusiveMaximum").and_then(|n| n.as_f64())
    }
    /// Inclusive lower numeric bound (`minimum`).
    #[must_use]
    pub fn minimum(&self) -> Option<f64> {
        self.resolved().get("minimum").and_then(|n| n.as_f64())
    }
    /// Strict lower numeric bound (`exclusiveMinimum`); see
    /// [`SchemaView::exclusive_maximum`] for the boolean-form caveat.
    #[must_use]
    pub fn exclusive_minimum(&self) -> Option<f64> {
        self.resolved().get("exclusiveMinimum").and_then(|n| n.as_f64())
    }
    /// Step size a valid value must be a multiple of (`multipleOf`).
    #[must_use]
    pub fn multiple_of(&self) -> Option<f64> {
        self.resolved().get("multipleOf").and_then(|n| n.as_f64())
    }
    // string bounds
    /// Maximum string length in characters (`maxLength`).
    #[must_use]
    pub fn max_length(&self) -> Option<u64> {
        self.resolved().get("maxLength").and_then(|n| n.as_u64())
    }
    #[must_use]
    /// Minimum string length in characters (`minLength`).
    pub fn min_length(&self) -> Option<u64> {
        self.resolved().get("minLength").and_then(|n| n.as_u64())
    }
    #[must_use]
    /// ECMA-262 regular expression string values must match (`pattern`).
    pub fn pattern(&self) -> Option<&'s str> {
        self.resolved().get("pattern").and_then(|n| n.as_str())
    }
    // array bounds
    #[must_use]
    /// Maximum array length (`maxItems`).
    pub fn max_items(&self) -> Option<u64> {
        self.resolved().get("maxItems").and_then(|n| n.as_u64())
    }
    #[must_use]
    /// Minimum array length (`minItems`).
    pub fn min_items(&self) -> Option<u64> {
        self.resolved().get("minItems").and_then(|n| n.as_u64())
    }
    #[must_use]
    /// Whether all array items must be distinct (`uniqueItems`).
    pub fn unique_items(&self) -> Option<bool> {
        self.resolved().get("uniqueItems").and_then(|n| n.as_bool())
    }
    // object bounds
    #[must_use]
    /// Maximum object property count (`maxProperties`).
    pub fn max_properties(&self) -> Option<u64> {
        self.resolved().get("maxProperties").and_then(|n| n.as_u64())
    }
    #[must_use]
    /// Minimum object property count (`minProperties`).
    pub fn min_properties(&self) -> Option<u64> {
        self.resolved().get("minProperties").and_then(|n| n.as_u64())
    }

    /// Vendor extension value (`x-*`) on the resolved schema.
    #[must_use]
    pub fn extension(&self, name: &str) -> Option<NodeRef<'s>> {
        self.resolved().get(name)
    }

}

/// Bit set of JSON-Schema primitive types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TypeSet(u8);

impl TypeSet {
    /// Accepts `null`.
    pub const NULL: u8 = 1 << 0;
    /// Accepts booleans.
    pub const BOOL: u8 = 1 << 1;
    /// Accepts objects.
    pub const OBJECT: u8 = 1 << 2;
    /// Accepts arrays.
    pub const ARRAY: u8 = 1 << 3;
    /// Accepts numbers (integer values included).
    pub const NUMBER: u8 = 1 << 4;
    /// Accepts integers only.
    pub const INTEGER: u8 = 1 << 5;
    /// Accepts strings.
    pub const STRING: u8 = 1 << 6;

    /// A set accepting no types.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    pub(crate) const fn insert(&mut self, bit: u8) {
        self.0 |= bit;
    }

    pub(crate) fn insert_str(&mut self, s: &str) {
        match s {
            "null" => self.insert(Self::NULL),
            "boolean" => self.insert(Self::BOOL),
            "object" => self.insert(Self::OBJECT),
            "array" => self.insert(Self::ARRAY),
            "number" => self.insert(Self::NUMBER),
            "integer" => self.insert(Self::INTEGER),
            "string" => self.insert(Self::STRING),
            _ => {}
        }
    }

    /// True when `bit` (one of the [`TypeSet`] constants) is accepted.
    #[must_use]
    pub const fn contains(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    /// True when no type is accepted.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Raw bitmask, for tests and diagnostics.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
}
