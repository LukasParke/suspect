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

use regex::Regex;
use rustc_hash::FxHashMap;
use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::Schema;
use crate::config::Config;
use crate::errors::CompileError;

pub(crate) type Prg<'d> = Rc<Program<'d>>;

/// A compiled subschema: an ordered list of keyword checks.
pub(crate) struct Program<'d> {
    /// Absolute pointer to this schema object in the document.
    pub path: Pointer,
    /// `$dynamicAnchor` names declared at this exact schema object.
    pub dyn_anchors: Vec<Rc<str>>,
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
                let name = node.as_str().unwrap_or_default();
                Ok(Self(bit(name, node.byte_range())?))
            }
            ValueKind::Array => {
                let mut acc = Self(0);
                for item in node.items() {
                    let Some(name) = item.as_str() else {
                        return Err(CompileError::Invalid {
                            message: "`type` array entries must be strings".into(),
                            at: item.byte_range(),
                        });
                    };
                    acc.0 |= bit(name, item.byte_range())?;
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

/// A numeric literal from the schema (bound or divisor).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Num {
    I(i64),
    F(f64),
}

impl Num {
    pub(crate) fn as_f64(self) -> f64 {
        match self {
            Num::I(v) => v as f64,
            Num::F(v) => v,
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
    MultipleOf(Num),
    /// Bound plus `exclusive` flag.
    Maximum(Num, bool),
    Minimum(Num, bool),
    MaxLength(usize),
    MinLength(usize),
    Pattern(Rc<Regex>),
    /// Applies to elements at indices >= the sibling `prefixItems` length.
    Items(Prg<'d>, usize),
    PrefixItems(Vec<Prg<'d>>),
    Contains {
        schema: Prg<'d>,
        min: usize,
        max: Option<usize>,
    },
    /// Property names that must be present.
    Required(Vec<&'d str>),
    Properties(Vec<(&'d str, Prg<'d>)>),
    PatternProperties(Vec<(Rc<Regex>, Prg<'d>)>),
    /// Inner `None` means boolean `false`. `except_*` come from the sibling
    /// `properties` / `patternProperties` keywords of the same schema object.
    AdditionalProperties {
        except_keys: Vec<&'d str>,
        except_patterns: Vec<Rc<Regex>>,
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
    DependentSchemas(Vec<(&'d str, Prg<'d>)>),
    DependentRequired(Vec<(&'d str, Vec<Box<str>>)>),
    Ref(RefTarget),
    /// Fragment name after `#`; resolved through the dynamic scope at exec.
    DynamicRef(Rc<str>),
    Format(Rc<str>),
}

/// Document-wide registration tables collected by one iterative sweep.
pub(crate) struct Scan {
    /// Every URI this document answers to: the root base plus each
    /// `$id`-derived resource base. A `$ref` resolving to any of these is
    /// same-document.
    pub doc_bases: Vec<String>,
    /// Resource base URI → pointer of the resource root object.
    pub base_ptrs: FxHashMap<String, Pointer>,
    /// `$anchor` name → pointer of the declaring schema resource.
    pub anchors: FxHashMap<String, Pointer>,
    /// `$dynamicAnchor` name → declaring schema pointer.
    pub dyn_anchors: FxHashMap<String, Pointer>,
    /// Pointer of each `$id`-bearing object → its fully-resolved base URI.
    pub id_bases: FxHashMap<Pointer, String>,
}

struct ScanNode<'d> {
    node: NodeRef<'d>,
    ptr: Pointer,
    base: String,
}

type CompileOutputs<'d> = Result<Vec<(&'d str, Vec<Box<str>>)>, CompileError>;

fn invalid(message: impl Into<String>, node: &NodeRef<'_>) -> CompileError {
    CompileError::Invalid {
        message: message.into(),
        at: node.byte_range(),
    }
}

/// Iteratively sweeps the whole document collecting anchors and `$id` bases
/// (explicit worklist — hostile nesting cannot overflow the native stack).
pub(crate) fn scan_doc(root: NodeRef<'_>, root_base: &str) -> Scan {
    let mut scan = Scan {
        doc_bases: Vec::new(),
        base_ptrs: FxHashMap::default(),
        anchors: FxHashMap::default(),
        dyn_anchors: FxHashMap::default(),
        id_bases: FxHashMap::default(),
    };
    let mut stack = vec![ScanNode {
        node: root,
        ptr: Pointer::root(),
        base: root_base.to_owned(),
    }];
    while let Some(item) = stack.pop() {
        match item.node.kind() {
            ValueKind::Object => {
                let mut base = item.base.clone();
                if let Some(id) = item.node.get("$id")
                    && let Some(raw) = id.as_str()
                    && let Some(joined) = join_uri(&base, raw)
                {
                    base = joined;
                    scan.id_bases.insert(item.ptr.clone(), base.clone());
                }
                for entry in item.node.entries() {
                    if matches!(entry.key, "$anchor" | "$dynamicAnchor")
                        && let Some(name) = entry.value.and_then(|v| v.as_str())
                        && !name.is_empty()
                    {
                        let table = if entry.key == "$anchor" {
                            &mut scan.anchors
                        } else {
                            &mut scan.dyn_anchors
                        };
                        table
                            .entry(name.to_owned())
                            .or_insert_with(|| item.ptr.clone());
                    }
                    let Some(v) = entry.value else { continue };
                    let child_ptr = item.ptr.push(entry.key);
                    push_children(&mut stack, v, child_ptr, &base);
                }
            }
            ValueKind::Array => {
                for (i, it) in item.node.items().into_iter().enumerate() {
                    let child_ptr = item.ptr.push(&i.to_string());
                    push_children(&mut stack, it, child_ptr, &item.base);
                }
            }
            _ => {}
        }
    }
    scan.doc_bases.push(root_base.to_owned());
    for b in scan.id_bases.values() {
        if !scan.doc_bases.contains(b) {
            scan.doc_bases.push(b.clone());
        }
    }
    // First registration wins per base (document-order duplicates are
    // pathological and undefined anyway); root base maps to `/`.
    scan.base_ptrs
        .entry(root_base.to_owned())
        .or_insert_with(Pointer::root);
    let mut pairs: Vec<(String, Pointer)> = scan
        .id_bases
        .iter()
        .map(|(p, b)| (b.clone(), p.clone()))
        .collect();
    pairs.sort_by_key(|a| a.1.to_path());
    for (b, p) in pairs {
        scan.base_ptrs.entry(b).or_insert(p);
    }
    scan
}

/// Pointer of the resource root enclosing `ptr`: the nearest self-or-ancestor
/// `$id` registration, falling back to the document root.
pub(crate) fn resource_root_for(scan: &Scan, ptr: &Pointer) -> Pointer {
    let mut cur = Some(ptr.clone());
    while let Some(p) = cur {
        if let Some(base) = scan.id_bases.get(&p)
            && let Some(rp) = scan.base_ptrs.get(base)
        {
            return rp.clone();
        }
        cur = p.parent();
    }
    Pointer::root()
}

/// Descend through everything: schemas may hide inside arbitrary values
/// (`$defs` under unknown parents, schemas in arrays, …).
fn push_children<'d>(stack: &mut Vec<ScanNode<'d>>, value: NodeRef<'d>, ptr: Pointer, base: &str) {
    if matches!(value.kind(), ValueKind::Object | ValueKind::Array) {
        stack.push(ScanNode {
            node: value,
            ptr,
            base: base.to_owned(),
        });
    }
}

/// Nearest inherited base URI for a pointer: walks up through ancestors,
/// consulting the `$id` registration table, falling back to the root base.
pub(crate) fn base_for_pointer(scan: &Scan, root_base: &str, ptr: &Pointer) -> String {
    let mut cur = Some(ptr.clone());
    while let Some(p) = cur {
        if let Some(b) = scan.id_bases.get(&p) {
            return b.clone();
        }
        cur = p.parent();
    }
    root_base.to_owned()
}

/// Resolves a raw `$ref` string against a base URI. Refs whose document part
/// lands on this very document (its URI or root `$id`) are local; anything
/// else is foreign and yields a clean execution-time error later.
fn resolve_ref_target(
    raw: &str,
    base: &str,
    scan: &Scan,
    res_ptr: &Pointer,
    at_node: &NodeRef<'_>,
) -> Result<RefTarget, CompileError> {
    let (doc_part, frag) = split_ref(raw);
    // Fragments are interpreted against the *owning resource* root; plain
    // names come from the document-wide `$anchor` registry.
    let local_frag = |frag: &str, res: &Pointer| -> Result<Pointer, CompileError> {
        if frag.is_empty() {
            return Ok(res.clone());
        }
        let decoded = percent_decode(frag);
        let text = String::from_utf8_lossy(&decoded).into_owned();
        if let Ok(p) = Pointer::parse(&text) {
            return Ok(res.join(&p));
        }
        // Plain-name fragment: `$anchor` lookup (2020-12 §8.2.3).
        scan.anchors
            .get(&text)
            .cloned()
            .ok_or_else(|| CompileError::Invalid {
                message: format!("unknown anchor `{text}`"),
                at: at_node.byte_range(),
            })
    };
    match doc_part {
        None | Some("") => Ok(RefTarget::Local(local_frag(frag, res_ptr)?)),
        Some(part) => {
            let joined = join_uri(base, part)
                .ok_or_else(|| invalid(format!("unresolvable $ref `{raw}`"), at_node))?;
            // Strip any fragment from the resolved URI before comparing.
            let joined_doc = joined.split('#').next().unwrap_or("").to_owned();
            if scan.doc_bases.iter().any(|b| b == &joined_doc) {
                let res = scan
                    .base_ptrs
                    .get(&joined_doc)
                    .cloned()
                    .unwrap_or_else(Pointer::root);
                Ok(RefTarget::Local(local_frag(frag, &res)?))
            } else {
                Ok(RefTarget::External)
            }
        }
    }
}

/// Splits a `$ref` value into its document part (if any) and the fragment
/// body without `#` — mirrors `suspect_source::Uri::split_ref`, which this
/// crate does not depend on directly.
fn split_ref(value: &str) -> (Option<&str>, &str) {
    match value.find('#') {
        None => (Some(value), ""),
        Some(0) => (None, &value[1..]),
        Some(i) => (Some(&value[..i]), &value[i + 1..]),
    }
}

/// Percent-decodes a URI fragment body (`%XX`); invalid escapes pass through.
fn percent_decode(frag: &str) -> Vec<u8> {
    let b = frag.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 1 && i + 2 < b.len() + 1 {
            let hex = b.get(i + 1..i + 3);
            if let Some(h) = hex.and_then(|h| {
                std::str::from_utf8(h)
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok())
            }) {
                out.push(h);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Minimal RFC 3986 §5 reference resolution against a base URI string,
/// sufficient for same-document detection: absolute references pass through
/// (path-normalized), relative ones merge onto the base path.
fn join_uri(base: &str, reference: &str) -> Option<String> {
    if has_scheme(reference) {
        return Some(with_normalized_path(reference));
    }
    if reference.starts_with("//") {
        let scheme = base.split(':').next()?;
        return Some(format!("{scheme}:{reference}"));
    }
    let colon = base.find(':')?;
    let scheme = &base[..=colon];
    let rest = &base[colon + 1..];
    let (authority, base_path) = match rest.strip_prefix("//") {
        Some(a) => match a.find('/') {
            Some(i) => (&rest[..i + 2], &a[i..]),
            None => (rest, ""),
        },
        None => ("", rest),
    };
    // strip query/fragment of the reference path
    let rpath = reference.split(['?', '#']).next().unwrap_or("");
    let merged = if rpath.is_empty() {
        base_path.to_owned()
    } else if rpath.starts_with('/') {
        rpath.to_owned()
    } else {
        match base_path.rfind('/') {
            Some(i) => format!("{}/{}", &base_path[..=i], rpath),
            None => format!("/{rpath}"),
        }
    };
    Some(format!("{scheme}{authority}{}", normalize_path(&merged)))
}

fn has_scheme(s: &str) -> bool {
    match s.find(':') {
        Some(i) => {
            let mut ch = s[..i].chars();
            matches!(ch.next(), Some(c) if c.is_ascii_alphabetic())
                && ch.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        }
        None => false,
    }
}

fn with_normalized_path(uri: &str) -> String {
    // scheme://authority/path?query#fragment — normalize only the path.
    let (pre, rest) = match uri.find("://") {
        Some(i) => (&uri[..i + 3], &uri[i + 3..]),
        None => ("", uri),
    };
    let (authority, rest) = match rest.find('/') {
        Some(j) => (&rest[..j], &rest[j..]),
        None => (rest, ""),
    };
    let (path, tail) = match rest.find(['?', '#']) {
        Some(k) => (&rest[..k], &rest[k..]),
        None => (rest, ""),
    };
    format!("{pre}{authority}{}{tail}", normalize_path(path))
}

/// Collapses `.` and `..` path segments; preserves a leading slash.
fn normalize_path(path: &str) -> String {
    let mut segs: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    let joined = segs.join("/");
    if path.starts_with('/') {
        format!("/{joined}")
    } else {
        joined
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
        let root_base = match schema.get("$id").and_then(|v| v.as_str()) {
            Some(id) => join_uri(&doc_uri, id)
                .ok_or_else(|| invalid(format!("invalid root $id `{id}`"), &schema))?,
            None => doc_uri,
        };
        let scan = scan_doc(schema, &root_base);
        let program = compile_program(
            self,
            schema,
            &Pointer::root(),
            &root_base,
            &scan,
            0,
            &Pointer::root(),
        )?;
        Ok(Schema::new(
            schema,
            program,
            scan,
            root_base,
            self.config.clone(),
        ))
    }
}

/// Raw keyword slots gathered from one schema object before building checks —
/// order-independent, so `"additionalProperties"` may precede its sibling
/// `"properties"` in the document.
#[derive(Default)]
struct Slots<'d> {
    id: Option<&'d str>,
    r#ref: Option<&'d str>,
    dynamic_ref: Option<&'d str>,
    dynamic_anchor: Option<&'d str>,
    type_: Option<NodeRef<'d>>,
    enum_: Option<NodeRef<'d>>,
    const_: Option<NodeRef<'d>>,
    required: Option<NodeRef<'d>>,
    multiple_of: Option<NodeRef<'d>>,
    maximum: Option<NodeRef<'d>>,
    exclusive_maximum: Option<NodeRef<'d>>,
    minimum: Option<NodeRef<'d>>,
    exclusive_minimum: Option<NodeRef<'d>>,
    max_length: Option<NodeRef<'d>>,
    min_length: Option<NodeRef<'d>>,
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

fn num_of(node: NodeRef<'_>) -> Option<Num> {
    match node.kind() {
        ValueKind::Int => node.as_i64().map(Num::I),
        ValueKind::Float => node.as_f64().map(Num::F),
        _ => None,
    }
}

fn uint_of(node: NodeRef<'_>) -> Option<usize> {
    match node.kind() {
        ValueKind::Int => node.as_u64().map(|v| v as usize),
        _ => None,
    }
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
                dyn_anchors: Vec::new(),
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
    base: &str,
    scan: &Scan,
    depth: usize,
    res_ptr: &Pointer,
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
        match entry.key {
            "$id" => slots.id = v.as_str(),
            "$ref" => slots.r#ref = v.as_str(),
            "$dynamicRef" => slots.dynamic_ref = v.as_str(),
            "$dynamicAnchor" => slots.dynamic_anchor = v.as_str(),
            "type" => slots.type_ = Some(v),
            "enum" => slots.enum_ = Some(v),
            "const" => slots.const_ = Some(v),
            "multipleOf" => slots.multiple_of = Some(v),
            "maximum" => slots.maximum = Some(v),
            "exclusiveMaximum" => slots.exclusive_maximum = Some(v),
            "minimum" => slots.minimum = Some(v),
            "exclusiveMinimum" => slots.exclusive_minimum = Some(v),
            "maxLength" => slots.max_length = Some(v),
            "minLength" => slots.min_length = Some(v),
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

    // Base URI for this resource: an `$id` here re-roots it for every keyword
    // in this object, including its own `$ref`.
    let (this_base, this_res): (std::borrow::Cow<'_, str>, Pointer) = match slots.id {
        Some(id) => (
            std::borrow::Cow::Owned(join_uri(base, id).ok_or_else(|| {
                CompileError::Invalid {
                    message: format!("invalid $id `{id}`"),
                    at: node
                        .get("$id")
                        .map_or_else(|| node.byte_range(), |n| n.byte_range()),
                }
            })?),
            path.clone(),
        ),
        None => (std::borrow::Cow::Borrowed(base), res_ptr.clone()),
    };

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
        if items.is_empty() {
            return Err(invalid("`enum` must contain at least one element", &e));
        }
        emit!("enum", Kind::Enum(items));
    }
    if let Some(v) = slots.const_ {
        emit!("const", Kind::Const(v));
    }

    // -- numeric -------------------------------------------------------------
    if let Some(m) = slots.multiple_of {
        let n = num_of(m).ok_or_else(|| invalid("`multipleOf` must be a number", &m))?;
        if n.as_f64() <= 0.0 {
            return Err(invalid("`multipleOf` must be strictly positive", &m));
        }
        emit!("multipleOf", Kind::MultipleOf(n));
    }
    if let Some(mx) = slots.maximum {
        let n = num_of(mx).ok_or_else(|| invalid("`maximum` must be a number", &mx))?;
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
        let n = num_of(xm).ok_or_else(|| invalid("`exclusiveMaximum` must be a number", &xm))?;
        emit!("exclusiveMaximum", Kind::Maximum(n, true));
    }
    if let Some(mn) = slots.minimum {
        let n = num_of(mn).ok_or_else(|| invalid("`minimum` must be a number", &mn))?;
        emit!("minimum", Kind::Minimum(n, false));
    }
    if let Some(xn) = slots.exclusive_minimum {
        if xn.kind() == ValueKind::Bool {
            return Err(invalid(
                "`exclusiveMinimum` must be a number (the boolean form was removed in 2020-12)",
                &xn,
            ));
        }
        let n = num_of(xn).ok_or_else(|| invalid("`exclusiveMinimum` must be a number", &xn))?;
        emit!("exclusiveMinimum", Kind::Minimum(n, true));
    }

    // -- strings -------------------------------------------------------------
    if let Some(v) = slots.max_length {
        let n =
            uint_of(v).ok_or_else(|| invalid("`maxLength` must be a non-negative integer", &v))?;
        emit!("maxLength", Kind::MaxLength(n));
    }
    if let Some(v) = slots.min_length {
        let n =
            uint_of(v).ok_or_else(|| invalid("`minLength` must be a non-negative integer", &v))?;
        emit!("minLength", Kind::MinLength(n));
    }
    if let Some(v) = slots.pattern {
        let s = v
            .as_str()
            .ok_or_else(|| invalid("`pattern` must be a string", &v))?;
        let re = Regex::new(s).map_err(|e| CompileError::Regex(e.to_string()))?;
        emit!("pattern", Kind::Pattern(Rc::new(re)));
    }

    // -- arrays --------------------------------------------------------------
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
        let sub = compile_sub(c, "items", v, path, &this_base, scan, next, &this_res)?;
        emit!("items", Kind::Items(sub, skip));
    }
    if let Some(v) = slots.prefix_items {
        let subs =
            compile_array_of_schemas(c, "prefixItems", v, path, &this_base, scan, next, &this_res)?;
        emit!("prefixItems", Kind::PrefixItems(subs));
    }
    if let Some(v) = slots.contains {
        let min = match slots.min_contains {
            Some(m) => uint_of(m)
                .ok_or_else(|| invalid("`minContains` must be a non-negative integer", &m))?,
            None => 1,
        };
        let max = match slots.max_contains {
            Some(m) => Some(
                uint_of(m)
                    .ok_or_else(|| invalid("`maxContains` must be a non-negative integer", &m))?,
            ),
            None => None,
        };
        let p = compile_sub(c, "contains", v, path, &this_base, scan, next, &this_res)?;
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
    // Sibling declarations that `additionalProperties` must except.
    let prop_keys: Vec<&'d str> = slots
        .properties
        .filter(|p| p.kind() == ValueKind::Object)
        .map(|p| p.entries().into_iter().map(|e| e.key).collect())
        .unwrap_or_default();
    let pat_res: Vec<Rc<Regex>> = slots
        .pattern_properties
        .iter()
        .filter(|p| p.kind() == ValueKind::Object)
        .flat_map(|p| p.entries())
        .filter_map(|e| Regex::new(e.key).ok().map(Rc::new))
        .collect();

    if let Some(v) = slots.properties {
        if v.kind() != ValueKind::Object {
            return Err(invalid("`properties` must be an object", &v));
        }
        let kw_path = path.push("properties");
        let mut subs = Vec::new();
        for e in v.entries() {
            let Some(sv) = e.value else { continue };
            let p = compile_program(
                c,
                sv,
                &kw_path.push(e.key),
                &this_base,
                scan,
                next,
                &this_res,
            )?;
            subs.push((e.key, p));
        }
        emit!("properties", Kind::Properties(subs));
    }
    if let Some(v) = slots.pattern_properties {
        if v.kind() != ValueKind::Object {
            return Err(invalid("`patternProperties` must be an object", &v));
        }
        let kw_path = path.push("patternProperties");
        let mut subs = Vec::new();
        for e in v.entries() {
            let Some(sv) = e.value else { continue };
            let re = Regex::new(e.key).map_err(|err| CompileError::Regex(err.to_string()))?;
            let p = compile_program(
                c,
                sv,
                &kw_path.push(e.key),
                &this_base,
                scan,
                next,
                &this_res,
            )?;
            subs.push((Rc::new(re), p));
        }
        emit!("patternProperties", Kind::PatternProperties(subs));
    }
    if let Some(v) = slots.additional_properties {
        // Boolean `true` accepts everything — no check needed. Boolean
        // `false` is the never-valid schema (inner `None`).
        let bool_true = v.kind() == ValueKind::Bool && v.as_bool() == Some(true);
        if !bool_true {
            let sub = match v.kind() {
                ValueKind::Bool => None,
                _ => Some(compile_sub(
                    c,
                    "additionalProperties",
                    v,
                    path,
                    &this_base,
                    scan,
                    next,
                    &this_res,
                )?),
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
    }
    if let Some(v) = slots.property_names {
        let p = compile_sub(
            c,
            "propertyNames",
            v,
            path,
            &this_base,
            scan,
            next,
            &this_res,
        )?;
        emit!("propertyNames", Kind::PropertyNames(p));
    }
    if let Some(v) = slots.unevaluated_properties {
        let bool_true = v.kind() == ValueKind::Bool && v.as_bool() == Some(true);
        if !bool_true {
            let sub = match v.kind() {
                ValueKind::Bool => None,
                _ => Some(compile_sub(
                    c,
                    "unevaluatedProperties",
                    v,
                    path,
                    &this_base,
                    scan,
                    next,
                    &this_res,
                )?),
            };
            tail.push(Check {
                at: path.push("unevaluatedProperties"),
                kind: Kind::UnevaluatedProperties(sub),
            });
        }
    }
    if let Some(v) = slots.unevaluated_items {
        let bool_true = v.kind() == ValueKind::Bool && v.as_bool() == Some(true);
        if !bool_true {
            let sub = match v.kind() {
                ValueKind::Bool => None,
                _ => Some(compile_sub(
                    c,
                    "unevaluatedItems",
                    v,
                    path,
                    &this_base,
                    scan,
                    next,
                    &this_res,
                )?),
            };
            tail.push(Check {
                at: path.push("unevaluatedItems"),
                kind: Kind::UnevaluatedItems(sub),
            });
        }
    }

    // -- composition ---------------------------------------------------------
    if let Some(v) = slots.all_of {
        let subs =
            compile_array_of_schemas(c, "allOf", v, path, &this_base, scan, next, &this_res)?;
        emit!("allOf", Kind::AllOf(subs));
    }
    if let Some(v) = slots.any_of {
        let subs =
            compile_array_of_schemas(c, "anyOf", v, path, &this_base, scan, next, &this_res)?;
        emit!("anyOf", Kind::AnyOf(subs));
    }
    if let Some(v) = slots.one_of {
        let subs =
            compile_array_of_schemas(c, "oneOf", v, path, &this_base, scan, next, &this_res)?;
        emit!("oneOf", Kind::OneOf(subs));
    }
    if let Some(v) = slots.not {
        let p = compile_sub(c, "not", v, path, &this_base, scan, next, &this_res)?;
        emit!("not", Kind::Not(p));
    }

    // -- conditional ---------------------------------------------------------
    if let Some(cond_v) = slots.if_ {
        // `if` without `then`/`else` asserts nothing.
        if slots.then.is_some() || slots.else_.is_some() {
            let cond = compile_sub(c, "if", cond_v, path, &this_base, scan, next, &this_res)?;
            let thn = slots
                .then
                .map(|t| compile_sub(c, "then", t, path, &this_base, scan, next, &this_res))
                .transpose()?;
            let els = slots
                .else_
                .map(|e| compile_sub(c, "else", e, path, &this_base, scan, next, &this_res))
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
    }

    // -- required ------------------------------------------------------------
    if let Some(v) = slots.required {
        if v.kind() != ValueKind::Array {
            return Err(invalid("`required` must be an array of strings", &v));
        }
        let mut names = Vec::new();
        for item in v.items() {
            let Some(n) = item.as_str() else {
                return Err(invalid("`required` entries must be strings", &item));
            };
            names.push(n);
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
            let Some(sv) = e.value else { continue };
            let p = compile_program(
                c,
                sv,
                &kw_path.push(e.key),
                &this_base,
                scan,
                next,
                &this_res,
            )?;
            subs.push((e.key, p));
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
            let Some(sv) = e.value else { continue };
            match sv.kind() {
                ValueKind::Object | ValueKind::Bool => {
                    let p = compile_program(
                        c,
                        sv,
                        &kw_path.push(e.key),
                        &this_base,
                        scan,
                        next,
                        &this_res,
                    )?;
                    schemas.push((e.key, p));
                }
                ValueKind::Array => {
                    let mut deps = Vec::new();
                    for item in sv.items() {
                        let Some(s) = item.as_str() else {
                            return Err(invalid(
                                "`dependencies` array entries must be strings",
                                &item,
                            ));
                        };
                        deps.push(Box::<str>::from(s));
                    }
                    required.push((e.key, deps));
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
    if let Some(raw) = slots.r#ref {
        let target = resolve_ref_target(raw, &this_base, scan, &this_res, &node)?;
        emit!("$ref", Kind::Ref(target));
    }
    if let Some(raw) = slots.dynamic_ref {
        let Some(name) = raw.strip_prefix('#') else {
            return Err(invalid(
                "only same-document fragment `$dynamicRef` values are supported",
                &node,
            ));
        };
        emit!("$dynamicRef", Kind::DynamicRef(Rc::from(name)));
    }

    // -- format --------------------------------------------------------------
    if let Some(v) = slots.format
        && c.config.format_assertion
    {
        let name = v
            .as_str()
            .ok_or_else(|| invalid("`format` must be a string", &v))?;
        emit!("format", Kind::Format(Rc::from(name)));
    }

    // -- assemble ------------------------------------------------------------
    let dyn_anchors = slots
        .dynamic_anchor
        .map_or(Vec::new(), |name| vec![Rc::from(name)]);
    Ok(Rc::new(Program {
        path: path.clone(),
        dyn_anchors,
        checks,
        tail,
    }))
}

/// Compiles a `{ name: [names…] }` map (`dependentRequired`).
fn compile_string_map<'d>(value: NodeRef<'d>, kw: &str) -> CompileOutputs<'d> {
    if value.kind() != ValueKind::Object {
        return Err(invalid(format!("{kw} must be an object"), &value));
    }
    let mut out = Vec::new();
    for e in value.entries() {
        let Some(sv) = e.value else { continue };
        if sv.kind() != ValueKind::Array {
            return Err(invalid(
                format!("{kw} values must be arrays of strings"),
                &sv,
            ));
        }
        let mut deps = Vec::new();
        for item in sv.items() {
            let Some(s) = item.as_str() else {
                return Err(invalid(
                    format!("{kw} array entries must be strings"),
                    &item,
                ));
            };
            deps.push(Box::<str>::from(s));
        }
        out.push((e.key, deps));
    }
    Ok(out)
}
