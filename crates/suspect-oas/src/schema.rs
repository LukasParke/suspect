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
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn default(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("default")
    }

    #[must_use]
    pub fn example(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("example")
    }

    #[must_use]
    pub fn examples(&self) -> Vec<NodeRef<'s>> {
        self.resolved().get("examples").map(|n| n.items()).unwrap_or_default()
    }

    #[must_use]
    pub fn enum_values(&self) -> Vec<NodeRef<'s>> {
        self.resolved().get("enum").map(|n| n.items()).unwrap_or_default()
    }

    #[must_use]
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
    pub fn all_of(&self) -> Vec<SchemaView<'s>> {
        self.keyword_schemas("allOf")
    }

    #[must_use]
    pub fn any_of(&self) -> Vec<SchemaView<'s>> {
        self.keyword_schemas("anyOf")
    }

    #[must_use]
    pub fn one_of(&self) -> Vec<SchemaView<'s>> {
        self.keyword_schemas("oneOf")
    }

    #[must_use]
    pub fn not(&self) -> Option<SchemaView<'s>> {
        self.resolved().get("not").map(|v| SchemaView::new(self.session, v))
    }

    #[must_use]
    pub fn discriminator(&self) -> Option<Discriminator<'s>> {
        self.resolved().get("discriminator").map(|n| Discriminator::new(self.session, n))
    }

    #[must_use]
    pub fn xml(&self) -> Option<Xml<'s>> {
        self.resolved().get("xml").map(|n| Xml::new(self.session, n))
    }

    #[must_use]
    pub fn external_docs(&self) -> Option<ExternalDocumentation<'s>> {
        self.resolved().get("externalDocs").map(|n| ExternalDocumentation::new(self.session, n))
    }

    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.resolved().get("deprecated").and_then(|n| n.as_bool()).unwrap_or(false)
    }

    #[must_use]
    pub fn read_only(&self) -> bool {
        self.resolved().get("readOnly").and_then(|n| n.as_bool()).unwrap_or(false)
    }

    #[must_use]
    pub fn write_only(&self) -> bool {
        self.resolved().get("writeOnly").and_then(|n| n.as_bool()).unwrap_or(false)
    }

    // numeric bounds
    #[must_use]
    pub fn maximum(&self) -> Option<f64> {
        self.resolved().get("maximum").and_then(|n| n.as_f64())
    }
    #[must_use]
    pub fn exclusive_maximum(&self) -> Option<f64> {
        self.resolved().get("exclusiveMaximum").and_then(|n| n.as_f64())
    }
    #[must_use]
    pub fn minimum(&self) -> Option<f64> {
        self.resolved().get("minimum").and_then(|n| n.as_f64())
    }
    #[must_use]
    pub fn exclusive_minimum(&self) -> Option<f64> {
        self.resolved().get("exclusiveMinimum").and_then(|n| n.as_f64())
    }
    #[must_use]
    pub fn multiple_of(&self) -> Option<f64> {
        self.resolved().get("multipleOf").and_then(|n| n.as_f64())
    }
    // string bounds
    #[must_use]
    pub fn max_length(&self) -> Option<u64> {
        self.resolved().get("maxLength").and_then(|n| n.as_u64())
    }
    #[must_use]
    pub fn min_length(&self) -> Option<u64> {
        self.resolved().get("minLength").and_then(|n| n.as_u64())
    }
    #[must_use]
    pub fn pattern(&self) -> Option<&'s str> {
        self.resolved().get("pattern").and_then(|n| n.as_str())
    }
    // array bounds
    #[must_use]
    pub fn max_items(&self) -> Option<u64> {
        self.resolved().get("maxItems").and_then(|n| n.as_u64())
    }
    #[must_use]
    pub fn min_items(&self) -> Option<u64> {
        self.resolved().get("minItems").and_then(|n| n.as_u64())
    }
    #[must_use]
    pub fn unique_items(&self) -> Option<bool> {
        self.resolved().get("uniqueItems").and_then(|n| n.as_bool())
    }
    // object bounds
    #[must_use]
    pub fn max_properties(&self) -> Option<u64> {
        self.resolved().get("maxProperties").and_then(|n| n.as_u64())
    }
    #[must_use]
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
    pub const NULL: u8 = 1 << 0;
    pub const BOOL: u8 = 1 << 1;
    pub const OBJECT: u8 = 1 << 2;
    pub const ARRAY: u8 = 1 << 3;
    pub const NUMBER: u8 = 1 << 4;
    pub const INTEGER: u8 = 1 << 5;
    pub const STRING: u8 = 1 << 6;

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

    #[must_use]
    pub const fn contains(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
}
