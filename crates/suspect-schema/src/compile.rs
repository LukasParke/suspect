//! Schema compilation: turns a [`NodeRef`] schema tree into an eagerly
//! compiled program of checks. `$ref` targets are *not* followed here — they
//! are resolved lazily at execution time (see [`crate::exec`]), which is what
//! makes recursive schemas possible.
//!
//! Compilation recursion depth equals schema nesting and is capped by
//! [`Config::max_depth`]; the whole document is additionally swept once
//! (iteratively) to register `$anchor`, `$dynamicAnchor`, and `$id`
//! resources.

use std::rc::Rc;

use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::Schema;
use crate::config::Config;
use crate::errors::CompileError;
use crate::keywords::cardinality::CountBound;
use crate::number::{Divisor, ExactNumber, NumberError};
use crate::resources::{self, Scan, resolve_ref_target, scan_doc};
use crate::{PatternError, PatternErrorKind, PatternProgram, compile_pattern};

pub(crate) type Prg<'d> = Rc<Program<'d>>;

/// A compiled subschema: an ordered list of keyword checks.
pub(crate) struct Program<'d> {
    /// Absolute pointer to this schema object in the document.
    pub path: Pointer,
    /// Owning schema resource, independent of the evaluation path.
    pub resource: Pointer,
    /// Keyword checks; `unevaluated*` live in `tail` so annotation tracking
    /// sees every sibling's contribution.
    pub checks: Vec<Check<'d>>,
    pub tail: Vec<Check<'d>>,
}

pub(crate) struct Check<'d> {
    /// Pointer to the keyword value (`#/properties/name/pattern`, …).
    pub at: Pointer,
    pub kind: Kind<'d>,
}

/// Bit set of JSON types accepted by a `type` keyword.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TypeBits(pub u8);

impl TypeBits {
    pub(crate) const NULL: u8 = 1 << 0;
    pub(crate) const BOOL: u8 = 1 << 1;
    pub(crate) const INT: u8 = 1 << 2;
    pub(crate) const NUM: u8 = 1 << 3;
    pub(crate) const STR: u8 = 1 << 4;
    pub(crate) const ARR: u8 = 1 << 5;
    pub(crate) const OBJ: u8 = 1 << 6;

    fn name_bit(name: &str) -> Option<u8> {
        Some(match name {
            "null" => Self::NULL,
            "boolean" => Self::BOOL,
            "integer" => Self::INT,
            "number" => Self::NUM,
            "string" => Self::STR,
            "array" => Self::ARR,
            "object" => Self::OBJ,
            _ => return None,
        })
    }

    fn from_node(node: NodeRef<'_>) -> Result<Self, CompileError> {
        let bit = |s: &str, at: std::ops::Range<usize>| {
            Self::name_bit(s).ok_or_else(|| CompileError::Invalid {
                message: format!("unknown type name `{s}`"),
                at,
            })
        };
        match node.kind() {
            ValueKind::Str => {
                let name = resources::string(node, "type")?;
                Ok(Self(bit(&name, node.byte_range())?))
            }
            ValueKind::Array => {
                let mut acc = Self(0);
                for item in node.items() {
                    let name = resources::string(item, "type entry")?;
                    acc.0 |= bit(&name, item.byte_range())?;
                }
                Ok(acc)
            }
            _ => Err(CompileError::Invalid {
                message: "`type` must be a string or array of strings".into(),
                at: node.byte_range(),
            }),
        }
    }

    /// Does a value of this kind satisfy the bit set? `float_is_int` says
    /// whether a Float value has zero fractional part.
    pub(crate) fn matches(&self, kind: ValueKind, float_is_int: bool) -> bool {
        let b = self.0;
        match kind {
            ValueKind::Null => b & Self::NULL != 0,
            ValueKind::Bool => b & Self::BOOL != 0,
            ValueKind::Int => b & (Self::INT | Self::NUM) != 0,
            // `integer` matches floats with zero fractional part (2020-12).
            ValueKind::Float => b & Self::NUM != 0 || (float_is_int && b & Self::INT != 0),
            ValueKind::Str => b & Self::STR != 0,
            ValueKind::Array => b & Self::ARR != 0,
            ValueKind::Object => b & Self::OBJ != 0,
        }
    }
}

/// Resolution result for a `$ref`.
#[derive(Clone, Debug)]
pub(crate) enum RefTarget {
    /// Same-document target.
    Local(Pointer),
    /// Points outside this document; execution reports a clean error.
    External,
}

pub(crate) enum Kind<'d> {
    /// Boolean schema (`true`/`false`).
    Always(bool),
    Type(TypeBits),
    Enum(Vec<NodeRef<'d>>),
    Const(NodeRef<'d>),
    UniqueItems,
    MultipleOf(Divisor),
    /// Bound plus `exclusive` flag.
    Maximum(ExactNumber, bool),
    Minimum(ExactNumber, bool),
    MaxLength(CountBound),
    MinLength(CountBound),
    MaxItems(CountBound),
    MinItems(CountBound),
    MaxProperties(CountBound),
    MinProperties(CountBound),
    Pattern(Rc<PatternProgram>),
    /// Applies to elements at indices >= the sibling `prefixItems` length.
    Items(Prg<'d>, usize),
    PrefixItems(Vec<Prg<'d>>),
    Contains {
        schema: Prg<'d>,
        min: CountBound,
        max: Option<CountBound>,
    },
    /// Property names that must be present.
    Required(Vec<String>),
    Properties(Vec<(String, Prg<'d>)>),
    PatternProperties(Vec<(Rc<PatternProgram>, Prg<'d>)>),
    /// Inner `None` means boolean `false`. `except_*` come from the sibling
    /// `properties` / `patternProperties` keywords of the same schema object.
    AdditionalProperties {
        except_keys: Vec<String>,
        except_patterns: Vec<Rc<PatternProgram>>,
        schema: Option<Prg<'d>>,
    },
    PropertyNames(Prg<'d>),
    UnevaluatedProperties(Option<Prg<'d>>),
    UnevaluatedItems(Option<Prg<'d>>),
    AllOf(Vec<Prg<'d>>),
    AnyOf(Vec<Prg<'d>>),
    OneOf(Vec<Prg<'d>>),
    Not(Prg<'d>),
    If {
        cond: Prg<'d>,
        then: Option<Prg<'d>>,
        alt: Option<Prg<'d>>,
    },
    DependentSchemas(Vec<(String, Prg<'d>)>),
    DependentRequired(Vec<(String, Vec<Box<str>>)>),
    Ref(RefTarget),
    /// Static resolution happens first; only dynamic-anchor fragments rebind.
    DynamicRef {
        target: RefTarget,
        anchor: Option<Rc<str>>,
    },
    Format(Rc<str>),
}

type CompileOutputs = Result<Vec<(String, Vec<Box<str>>)>, CompileError>;

fn invalid(message: impl Into<String>, node: &NodeRef<'_>) -> CompileError {
    CompileError::Invalid {
        message: message.into(),
        at: node.byte_range(),
    }
}

fn pattern_error(error: PatternError, node: NodeRef<'_>) -> CompileError {
    match error.kind {
        PatternErrorKind::Invalid => CompileError::Invalid {
            message: error.message,
            at: node.byte_range(),
        },
        PatternErrorKind::Unsupported => CompileError::Unsupported {
            message: error.message,
            at: node.byte_range(),
        },
        PatternErrorKind::Limit => CompileError::ResourceLimit {
            message: error.message,
            at: node.byte_range(),
        },
    }
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

/// Compiles schema [`NodeRef`]s into executable [`Schema`] programs.
#[derive(Clone, Debug)]
pub struct Compiler {
    config: Config,
}

impl Compiler {
    #[must_use]
    /// Creates a compiler with the given configuration.
    ///
    /// The compiler borrows nothing mutable; the same value can compile any
    /// number of schemas, and the [`Config`] is cloned into each resulting
    /// [`Schema`](crate::Schema).
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Compiles a schema subtree into a reusable validator.
    ///
    /// # Errors
    /// Returns [`CompileError`] for malformed keyword values, nesting beyond
    /// [`Config::max_depth`], or invalid regular expressions.
    pub fn compile<'d>(&self, schema: NodeRef<'d>) -> Result<Schema<'d>, CompileError> {
        let doc_uri = schema.syntax().doc().uri().as_str().to_owned();
        let scan = scan_doc(schema, &doc_uri, self.config.max_depth)?;
        let root_base = scan.base_for(&Pointer::root()).to_owned();
        let program = compile_program(
            self,
            schema,
            &Pointer::root(),
            &root_base,
            &scan,
            0,
            &Pointer::root(),
        )?;
        Ok(Schema::new(schema, program, scan, self.config.clone()))
    }
}

/// Raw keyword slots gathered from one schema object before building checks —
/// order-independent, so `"additionalProperties"` may precede its sibling
/// `"properties"` in the document.
#[derive(Default)]
struct Slots<'d> {
    r#ref: Option<String>,
    dynamic_ref: Option<String>,
    type_: Option<NodeRef<'d>>,
    enum_: Option<NodeRef<'d>>,
    const_: Option<NodeRef<'d>>,
    unique_items: Option<NodeRef<'d>>,
    required: Option<NodeRef<'d>>,
    multiple_of: Option<NodeRef<'d>>,
    maximum: Option<NodeRef<'d>>,
    exclusive_maximum: Option<NodeRef<'d>>,
    minimum: Option<NodeRef<'d>>,
    exclusive_minimum: Option<NodeRef<'d>>,
    max_length: Option<NodeRef<'d>>,
    min_length: Option<NodeRef<'d>>,
    max_items: Option<NodeRef<'d>>,
    min_items: Option<NodeRef<'d>>,
    max_properties: Option<NodeRef<'d>>,
    min_properties: Option<NodeRef<'d>>,
    pattern: Option<NodeRef<'d>>,
    items: Option<NodeRef<'d>>,
    prefix_items: Option<NodeRef<'d>>,
    contains: Option<NodeRef<'d>>,
    min_contains: Option<NodeRef<'d>>,
    max_contains: Option<NodeRef<'d>>,
    properties: Option<NodeRef<'d>>,
    pattern_properties: Option<NodeRef<'d>>,
    additional_properties: Option<NodeRef<'d>>,
    property_names: Option<NodeRef<'d>>,
    unevaluated_properties: Option<NodeRef<'d>>,
    unevaluated_items: Option<NodeRef<'d>>,
    all_of: Option<NodeRef<'d>>,
    any_of: Option<NodeRef<'d>>,
    one_of: Option<NodeRef<'d>>,
    not: Option<NodeRef<'d>>,
    if_: Option<NodeRef<'d>>,
    then: Option<NodeRef<'d>>,
    else_: Option<NodeRef<'d>>,
    dependent_schemas: Option<NodeRef<'d>>,
    dependent_required: Option<NodeRef<'d>>,
    dependencies: Option<(NodeRef<'d>, NodeRef<'d>)>,
    format: Option<NodeRef<'d>>,
}

/// Keywords that are pure annotations (meta-data / content / identifiers /
/// structural vocabulary). Never asserted (2020-12 §9).
fn is_annotation_only(key: &str) -> bool {
    matches!(
        key,
        "$schema"
            | "$vocabulary"
            | "$comment"
            | "$defs"
            | "defs"
            | "$anchor"
            | "title"
            | "description"
            | "default"
            | "deprecated"
            | "readOnly"
            | "writeOnly"
            | "examples"
            | "contentEncoding"
            | "contentMediaType"
            | "contentSchema"
    )
}

fn exact_num_of(node: NodeRef<'_>, kw: &str, config: &Config) -> Result<ExactNumber, CompileError> {
    if !matches!(node.kind(), ValueKind::Int | ValueKind::Float) {
        return Err(invalid(format!("`{kw}` must be a number"), &node));
    }
    ExactNumber::parse(node.scalar_bytes(), config.max_number_bytes).map_err(|error| match error {
        NumberError::ResourceLimit { .. } => CompileError::ResourceLimit {
            message: error.to_string(),
            at: node.byte_range(),
        },
        NumberError::NonFiniteOrInvalid => invalid(format!("`{kw}`: {error}"), &node),
    })
}

fn count_of(node: NodeRef<'_>, keyword: &str, config: &Config) -> Result<CountBound, CompileError> {
    let value = exact_num_of(node, keyword, config)?;
    if !node.is_integral_number() || value.is_negative() {
        return Err(invalid(
            format!("`{keyword}` must be a non-negative integer"),
            &node,
        ));
    }
    Ok(value
        .to_usize()
        .map_or(CountBound::BeyondAddressable, CountBound::Finite))
}

#[allow(clippy::too_many_arguments)] // keyword-compiler plumbing is uniform
fn compile_array_of_schemas<'d>(
    c: &Compiler,
    kw: &str,
    value: NodeRef<'d>,
    path: &Pointer,
    base: &str,
    scan: &Scan,
    depth: usize,
    res_ptr: &Pointer,
) -> Result<Vec<Prg<'d>>, CompileError> {
    if value.kind() != ValueKind::Array {
        return Err(invalid(
            format!("`{kw}` must be an array of schemas"),
            &value,
        ));
    }
    let kw_path = path.push(kw);
    let mut subs = Vec::new();
    for (i, item) in value.items().into_iter().enumerate() {
        subs.push(compile_program(
            c,
            item,
            &kw_path.push(&i.to_string()),
            base,
            scan,
            depth,
            res_ptr,
        )?);
    }
    Ok(subs)
}

/// Compiles one schema node (object or boolean) into a [`Program`].
///
/// `depth` counts schema nesting; exceeding [`Config::max_depth`] yields
/// [`CompileError::TooDeep`] instead of overflowing the native stack.
pub(crate) fn compile_program<'d>(
    c: &Compiler,
    node: NodeRef<'d>,
    path: &Pointer,
    base: &str,
    scan: &Scan,
    depth: usize,
    res_ptr: &Pointer,
) -> Result<Prg<'d>, CompileError> {
    if depth > c.config.max_depth {
        return Err(CompileError::TooDeep {
            cap: c.config.max_depth,
        });
    }
    match node.kind() {
        ValueKind::Bool => {
            let b = node.as_bool().unwrap_or(false);
            Ok(Rc::new(Program {
                path: path.clone(),
                resource: scan.resource_for(path),
                checks: vec![Check {
                    at: path.clone(),
                    kind: Kind::Always(b),
                }],
                tail: Vec::new(),
            }))
        }
        ValueKind::Object => compile_object(c, node, path, base, scan, depth, res_ptr),
        _ => Err(invalid("a schema must be an object or a boolean", &node)),
    }
}

/// Compiles a keyword-value subschema located at `path/kw`.
#[allow(clippy::too_many_arguments)]
fn compile_sub<'d>(
    c: &Compiler,
    kw: &str,
    value: NodeRef<'d>,
    path: &Pointer,
    base: &str,
    scan: &Scan,
    depth: usize,
    res_ptr: &Pointer,
) -> Result<Prg<'d>, CompileError> {
    compile_program(c, value, &path.push(kw), base, scan, depth, res_ptr)
}

#[allow(clippy::too_many_arguments)]
fn compile_object<'d>(
    c: &Compiler,
    node: NodeRef<'d>,
    path: &Pointer,
    _base: &str,
    scan: &Scan,
    depth: usize,
    _res_ptr: &Pointer,
) -> Result<Prg<'d>, CompileError> {
    let next = depth + 1;
    let mut slots = Slots::default();

    for entry in node.entries() {
        let Some(v) = entry.value else {
            return Err(invalid(
                format!("keyword `{}` has no value", entry.key),
                &node,
            ));
        };
        let keyword = resources::string(entry.key_node, "schema keyword")?;
        match keyword.as_str() {
            "$id" | "$anchor" | "$dynamicAnchor" => {} // indexed lexically
            "$ref" => slots.r#ref = Some(resources::string(v, "$ref")?),
            "$dynamicRef" => slots.dynamic_ref = Some(resources::string(v, "$dynamicRef")?),
            "type" => slots.type_ = Some(v),
            "enum" => slots.enum_ = Some(v),
            "const" => slots.const_ = Some(v),
            "uniqueItems" => slots.unique_items = Some(v),
            "multipleOf" => slots.multiple_of = Some(v),
            "maximum" => slots.maximum = Some(v),
            "exclusiveMaximum" => slots.exclusive_maximum = Some(v),
            "minimum" => slots.minimum = Some(v),
            "exclusiveMinimum" => slots.exclusive_minimum = Some(v),
            "maxLength" => slots.max_length = Some(v),
            "minLength" => slots.min_length = Some(v),
            "maxItems" => slots.max_items = Some(v),
            "minItems" => slots.min_items = Some(v),
            "maxProperties" => slots.max_properties = Some(v),
            "minProperties" => slots.min_properties = Some(v),
            "pattern" => slots.pattern = Some(v),
            "items" => slots.items = Some(v),
            "prefixItems" => slots.prefix_items = Some(v),
            "contains" => slots.contains = Some(v),
            "required" => slots.required = Some(v),
            "minContains" => slots.min_contains = Some(v),
            "maxContains" => slots.max_contains = Some(v),
            "properties" => slots.properties = Some(v),
            "patternProperties" => slots.pattern_properties = Some(v),
            "additionalProperties" => slots.additional_properties = Some(v),
            "propertyNames" => slots.property_names = Some(v),
            "unevaluatedProperties" => slots.unevaluated_properties = Some(v),
            "unevaluatedItems" => slots.unevaluated_items = Some(v),
            "allOf" => slots.all_of = Some(v),
            "anyOf" => slots.any_of = Some(v),
            "oneOf" => slots.one_of = Some(v),
            "not" => slots.not = Some(v),
            "if" => slots.if_ = Some(v),
            "then" => slots.then = Some(v),
            "else" => slots.else_ = Some(v),
            "dependentSchemas" => slots.dependent_schemas = Some(v),
            "dependentRequired" => slots.dependent_required = Some(v),
            "dependencies" => slots.dependencies = Some((node, v)),
            "format" => slots.format = Some(v),
            k if is_annotation_only(k) => {}
            _ => {} // unknown keywords are annotations per 2020-12 §6.1
        }
    }

    // The lexical index resolves each $id once. Lazy compilation must not
    // join a resource's relative $id onto its already-resolved URI again.
    let this_base = scan.base_for(path);
    let this_res = scan.resource_for(path);

    let mut checks: Vec<Check<'d>> = Vec::new();
    let mut tail: Vec<Check<'d>> = Vec::new(); // unevaluated* run last

    macro_rules! emit {
        ($kw:expr, $kind:expr) => {
            checks.push(Check {
                at: path.push($kw),
                kind: $kind,
            });
        };
    }

    // -- type / enum / const -------------------------------------------------
    if let Some(t) = slots.type_ {
        emit!("type", Kind::Type(TypeBits::from_node(t)?));
    }
    if let Some(e) = slots.enum_ {
        if e.kind() != ValueKind::Array {
            return Err(invalid("`enum` must be an array", &e));
        }
        let items = e.items();
        emit!("enum", Kind::Enum(items));
    }
    if let Some(v) = slots.const_ {
        emit!("const", Kind::Const(v));
    }
    if let Some(v) = slots.unique_items {
        match v.as_bool() {
            Some(true) => {
                emit!("uniqueItems", Kind::UniqueItems);
            }
            Some(false) => {}
            None => return Err(invalid("`uniqueItems` must be a boolean", &v)),
        }
    }

    // -- numeric -------------------------------------------------------------
    if let Some(m) = slots.multiple_of {
        let n = exact_num_of(m, "multipleOf", &c.config)?;
        if !n.is_positive() {
            return Err(invalid("`multipleOf` must be strictly positive", &m));
        }
        emit!("multipleOf", Kind::MultipleOf(Divisor::new(n)));
    }
    if let Some(mx) = slots.maximum {
        let n = exact_num_of(mx, "maximum", &c.config)?;
        emit!("maximum", Kind::Maximum(n, false));
    }
    if let Some(xm) = slots.exclusive_maximum {
        if xm.kind() == ValueKind::Bool {
            // Draft-04 boolean form is illegal in 2020-12.
            return Err(invalid(
                "`exclusiveMaximum` must be a number (the boolean form was removed in 2020-12)",
                &xm,
            ));
        }
        let n = exact_num_of(xm, "exclusiveMaximum", &c.config)?;
        emit!("exclusiveMaximum", Kind::Maximum(n, true));
    }
    if let Some(mn) = slots.minimum {
        let n = exact_num_of(mn, "minimum", &c.config)?;
        emit!("minimum", Kind::Minimum(n, false));
    }
    if let Some(xn) = slots.exclusive_minimum {
        if xn.kind() == ValueKind::Bool {
            return Err(invalid(
                "`exclusiveMinimum` must be a number (the boolean form was removed in 2020-12)",
                &xn,
            ));
        }
        let n = exact_num_of(xn, "exclusiveMinimum", &c.config)?;
        emit!("exclusiveMinimum", Kind::Minimum(n, true));
    }

    // -- strings -------------------------------------------------------------
    if let Some(v) = slots.max_length {
        let n = count_of(v, "maxLength", &c.config)?;
        emit!("maxLength", Kind::MaxLength(n));
    }
    if let Some(v) = slots.min_length {
        let n = count_of(v, "minLength", &c.config)?;
        emit!("minLength", Kind::MinLength(n));
    }
    if let Some(v) = slots.pattern {
        let s = resources::string(v, "pattern")?;
        let program = compile_pattern(&s).map_err(|error| pattern_error(error, v))?;
        emit!("pattern", Kind::Pattern(Rc::new(program)));
    }

    // -- arrays --------------------------------------------------------------
    if let Some(v) = slots.max_items {
        emit!(
            "maxItems",
            Kind::MaxItems(count_of(v, "maxItems", &c.config)?)
        );
    }
    if let Some(v) = slots.min_items {
        emit!(
            "minItems",
            Kind::MinItems(count_of(v, "minItems", &c.config)?)
        );
    }
    if let Some(v) = slots.items {
        if v.kind() == ValueKind::Array {
            return Err(invalid(
                "`items` takes a single schema in 2020-12; use `prefixItems` for tuples",
                &v,
            ));
        }
        // 2020-12: `items` applies to elements beyond `prefixItems` only.
        let skip = slots
            .prefix_items
            .filter(|p| p.kind() == ValueKind::Array)
            .map_or(0, |p| p.items().len());
        let sub = compile_sub(c, "items", v, path, this_base, scan, next, &this_res)?;
        emit!("items", Kind::Items(sub, skip));
    }
    if let Some(v) = slots.prefix_items {
        let subs =
            compile_array_of_schemas(c, "prefixItems", v, path, this_base, scan, next, &this_res)?;
        emit!("prefixItems", Kind::PrefixItems(subs));
    }
    let min_contains = slots
        .min_contains
        .map(|node| count_of(node, "minContains", &c.config))
        .transpose()?;
    let max_contains = slots
        .max_contains
        .map(|node| count_of(node, "maxContains", &c.config))
        .transpose()?;
    if let Some(v) = slots.contains {
        let min = min_contains.unwrap_or(CountBound::Finite(1));
        let max = max_contains;
        let p = compile_sub(c, "contains", v, path, this_base, scan, next, &this_res)?;
        emit!(
            "contains",
            Kind::Contains {
                schema: p,
                min,
                max
            }
        );
    }

    // -- objects -------------------------------------------------------------
    if let Some(v) = slots.max_properties {
        emit!(
            "maxProperties",
            Kind::MaxProperties(count_of(v, "maxProperties", &c.config)?)
        );
    }
    if let Some(v) = slots.min_properties {
        emit!(
            "minProperties",
            Kind::MinProperties(count_of(v, "minProperties", &c.config)?)
        );
    }
    // Allocate decoded sibling names once at compilation.
    let prop_keys: Vec<String> = slots
        .properties
        .into_iter()
        .flat_map(|p| p.entries())
        .map(|e| resources::string(e.key_node, "property name"))
        .collect::<Result<_, _>>()?;
    let pat_programs: Vec<(String, Rc<PatternProgram>)> = slots
        .pattern_properties
        .into_iter()
        .flat_map(|p| p.entries())
        .map(|e| {
            let pattern = resources::string(e.key_node, "property pattern")?;
            let program =
                compile_pattern(&pattern).map_err(|error| pattern_error(error, e.key_node))?;
            Ok((pattern, Rc::new(program)))
        })
        .collect::<Result<_, _>>()?;
    let pat_res = pat_programs
        .iter()
        .map(|(_, program)| Rc::clone(program))
        .collect();

    if let Some(v) = slots.properties {
        if v.kind() != ValueKind::Object {
            return Err(invalid("`properties` must be an object", &v));
        }
        let kw_path = path.push("properties");
        let mut subs = Vec::new();
        for e in v.entries() {
            let key = resources::string(e.key_node, "schema property name")?;
            let Some(sv) = e.value else { continue };
            let p = compile_program(c, sv, &kw_path.push(&key), this_base, scan, next, &this_res)?;
            subs.push((key, p));
        }
        emit!("properties", Kind::Properties(subs));
    }
    if let Some(v) = slots.pattern_properties {
        if v.kind() != ValueKind::Object {
            return Err(invalid("`patternProperties` must be an object", &v));
        }
        let kw_path = path.push("patternProperties");
        let mut subs = Vec::new();
        for (e, (key, program)) in v.entries().into_iter().zip(&pat_programs) {
            let Some(sv) = e.value else { continue };
            let p = compile_program(c, sv, &kw_path.push(key), this_base, scan, next, &this_res)?;
            subs.push((Rc::clone(program), p));
        }
        emit!("patternProperties", Kind::PatternProperties(subs));
    }
    if let Some(v) = slots.additional_properties {
        // Explicit true still evaluates additional properties. Omitting its
        // check would lose annotations consumed by an adjacent unevaluated rule.
        let sub = if v.kind() == ValueKind::Bool && v.as_bool() == Some(false) {
            None
        } else {
            Some(compile_sub(
                c,
                "additionalProperties",
                v,
                path,
                this_base,
                scan,
                next,
                &this_res,
            )?)
        };
        checks.push(Check {
            at: path.push("additionalProperties"),
            kind: Kind::AdditionalProperties {
                except_keys: prop_keys,
                except_patterns: pat_res,
                schema: sub,
            },
        });
    }
    if let Some(v) = slots.property_names {
        let p = compile_sub(
            c,
            "propertyNames",
            v,
            path,
            this_base,
            scan,
            next,
            &this_res,
        )?;
        emit!("propertyNames", Kind::PropertyNames(p));
    }
    for (keyword, value, properties) in [
        ("unevaluatedProperties", slots.unevaluated_properties, true),
        ("unevaluatedItems", slots.unevaluated_items, false),
    ] {
        if let Some(v) = value {
            let sub = if v.kind() == ValueKind::Bool && v.as_bool() == Some(false) {
                None
            } else {
                Some(compile_sub(
                    c, keyword, v, path, this_base, scan, next, &this_res,
                )?)
            };
            tail.push(Check {
                at: path.push(keyword),
                kind: if properties {
                    Kind::UnevaluatedProperties(sub)
                } else {
                    Kind::UnevaluatedItems(sub)
                },
            });
        }
    }

    // -- composition ---------------------------------------------------------
    if let Some(v) = slots.all_of {
        let subs = compile_array_of_schemas(c, "allOf", v, path, this_base, scan, next, &this_res)?;
        emit!("allOf", Kind::AllOf(subs));
    }
    if let Some(v) = slots.any_of {
        let subs = compile_array_of_schemas(c, "anyOf", v, path, this_base, scan, next, &this_res)?;
        emit!("anyOf", Kind::AnyOf(subs));
    }
    if let Some(v) = slots.one_of {
        let subs = compile_array_of_schemas(c, "oneOf", v, path, this_base, scan, next, &this_res)?;
        emit!("oneOf", Kind::OneOf(subs));
    }
    if let Some(v) = slots.not {
        let p = compile_sub(c, "not", v, path, this_base, scan, next, &this_res)?;
        emit!("not", Kind::Not(p));
    }

    // -- conditional ---------------------------------------------------------
    if let Some(cond_v) = slots.if_ {
        // Even without then/else, a successful if contributes annotations.
        let cond = compile_sub(c, "if", cond_v, path, this_base, scan, next, &this_res)?;
        let thn = slots
            .then
            .map(|t| compile_sub(c, "then", t, path, this_base, scan, next, &this_res))
            .transpose()?;
        let els = slots
            .else_
            .map(|e| compile_sub(c, "else", e, path, this_base, scan, next, &this_res))
            .transpose()?;
        checks.push(Check {
            at: path.push("if"),
            kind: Kind::If {
                cond,
                then: thn,
                alt: els,
            },
        });
    }

    // -- required ------------------------------------------------------------
    if let Some(v) = slots.required {
        if v.kind() != ValueKind::Array {
            return Err(invalid("`required` must be an array of strings", &v));
        }
        let mut names = Vec::new();
        for item in v.items() {
            names.push(resources::string(item, "required entry")?);
        }
        emit!("required", Kind::Required(names));
    }

    // -- dependencies --------------------------------------------------------
    if let Some(v) = slots.dependent_schemas {
        if v.kind() != ValueKind::Object {
            return Err(invalid("`dependentSchemas` must be an object", &v));
        }
        let kw_path = path.push("dependentSchemas");
        let mut subs = Vec::new();
        for e in v.entries() {
            let key = resources::string(e.key_node, "schema property name")?;
            let Some(sv) = e.value else { continue };
            let p = compile_program(c, sv, &kw_path.push(&key), this_base, scan, next, &this_res)?;
            subs.push((key, p));
        }
        emit!("dependentSchemas", Kind::DependentSchemas(subs));
    }
    if let Some(v) = slots.dependent_required {
        let req = compile_string_map(v, "`dependentRequired`")?;
        emit!("dependentRequired", Kind::DependentRequired(req));
    }
    if let Some((_, v)) = slots.dependencies {
        // Legacy keyword: schema values behave like dependentSchemas, array
        // values like dependentRequired.
        if v.kind() != ValueKind::Object {
            return Err(invalid("`dependencies` must be an object", &v));
        }
        let kw_path = path.push("dependencies");
        let mut schemas = Vec::new();
        let mut required = Vec::new();
        for e in v.entries() {
            let key = resources::string(e.key_node, "schema property name")?;
            let Some(sv) = e.value else { continue };
            match sv.kind() {
                ValueKind::Object | ValueKind::Bool => {
                    let p = compile_program(
                        c,
                        sv,
                        &kw_path.push(&key),
                        this_base,
                        scan,
                        next,
                        &this_res,
                    )?;
                    schemas.push((key, p));
                }
                ValueKind::Array => {
                    let mut deps = Vec::new();
                    for item in sv.items() {
                        deps.push(resources::string(item, "dependency entry")?.into_boxed_str());
                    }
                    required.push((key, deps));
                }
                _ => {
                    return Err(invalid(
                        "`dependencies` values must be schemas or arrays of strings",
                        &sv,
                    ));
                }
            }
        }
        if !schemas.is_empty() {
            checks.push(Check {
                at: kw_path.clone(),
                kind: Kind::DependentSchemas(schemas),
            });
        }
        if !required.is_empty() {
            checks.push(Check {
                at: kw_path.clone(),
                kind: Kind::DependentRequired(required),
            });
        }
    }

    // -- references ----------------------------------------------------------
    if let Some(raw) = slots.r#ref.as_deref() {
        let target = resolve_ref_target(raw, scan, &this_res, node)?;
        emit!("$ref", Kind::Ref(target));
    }
    if let Some(raw) = slots.dynamic_ref.as_deref() {
        let target = resolve_ref_target(raw, scan, &this_res, node)?;
        let anchor = resources::dynamic_name(raw, &target, scan);
        emit!("$dynamicRef", Kind::DynamicRef { target, anchor });
    }

    // -- format --------------------------------------------------------------
    if let Some(v) = slots.format
        && c.config.format_assertion
    {
        let name = resources::string(v, "format")?;
        emit!("format", Kind::Format(Rc::from(name)));
    }

    // -- assemble ------------------------------------------------------------
    Ok(Rc::new(Program {
        path: path.clone(),
        resource: this_res,
        checks,
        tail,
    }))
}

/// Compiles a `{ name: [names…] }` map (`dependentRequired`).
fn compile_string_map<'d>(value: NodeRef<'d>, kw: &str) -> CompileOutputs {
    if value.kind() != ValueKind::Object {
        return Err(invalid(format!("{kw} must be an object"), &value));
    }
    let mut out = Vec::new();
    for e in value.entries() {
        let key = resources::string(e.key_node, "dependency name")?;
        let Some(sv) = e.value else { continue };
        if sv.kind() != ValueKind::Array {
            return Err(invalid(
                format!("{kw} values must be arrays of strings"),
                &sv,
            ));
        }
        let mut deps = Vec::new();
        for item in sv.items() {
            deps.push(resources::string(item, "dependency entry")?.into_boxed_str());
        }
        out.push((key, deps));
    }
    Ok(out)
}
