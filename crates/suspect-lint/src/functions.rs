//! Builtin `then` functions: the Spectral core set plus the native checks
//! the ruleset language cannot express (path parameters, `$ref` siblings,
//! enum typing, duplicate path keys, response presence).
//!
//! Every function receives the matched node and appends findings anchored at
//! the offending node's byte range and pointer.

use regex::Regex;
use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::engine::Finding;
use crate::rule::Rule;

mod typed_enum;

/// Casing conventions accepted by the `casing` function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Casing {
    /// `camelCase`: leading lowercase, inner capitals, no separators.
    Camel,
    /// `PascalCase`: leading capital, no separators.
    Pascal,
    /// `kebab-case`: lowercase words joined by single hyphens.
    Kebab,
    /// `snake_case`: lowercase words joined by single underscores.
    Snake,
    /// `MACRO_CASE`: uppercase words joined by single underscores.
    Macro,
}

/// Continuation characters for lowercase-separated conventions.
fn sep_lower(c: char, sep: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == sep
}

/// Continuation characters for `MACRO_CASE`.
fn sep_upper(c: char, sep: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || c == sep
}

impl Casing {
    pub(crate) fn from_text(text: &str) -> Option<Self> {
        match text {
            "camel" => Some(Self::Camel),
            "pascal" | "Pascal" => Some(Self::Pascal),
            "kebab" => Some(Self::Kebab),
            "snake" => Some(Self::Snake),
            "macro" => Some(Self::Macro),
            _ => None,
        }
    }

    fn matches(self, s: &str) -> bool {
        let mut chars = s.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        let mut rest = chars;
        match self {
            Self::Camel => {
                first.is_ascii_lowercase()
                    && first.is_ascii_alphabetic()
                    && rest.all(|c| c.is_ascii_alphanumeric())
                    && !s.contains(['-', '_', ' '])
            }
            Self::Pascal => {
                first.is_ascii_uppercase()
                    && rest.all(|c| c.is_ascii_alphanumeric())
                    && !s.contains(['-', '_', ' '])
            }
            Self::Kebab => {
                first.is_ascii_lowercase()
                    && rest.all(|c| sep_lower(c, '-'))
                    && !s.ends_with('-')
                    && !s.contains("--")
            }
            Self::Snake => {
                first.is_ascii_lowercase()
                    && rest.all(|c| sep_lower(c, '_'))
                    && !s.ends_with('_')
                    && !s.contains("__")
            }
            Self::Macro => {
                first.is_ascii_uppercase()
                    && rest.all(|c| sep_upper(c, '_'))
                    && !s.ends_with('_')
                    && !s.contains("__")
            }
        }
    }
}

/// One allowed scalar in an `enumeration` option list.
#[derive(Debug, Clone)]
pub(crate) enum EnumValue {
    /// Allowed string.
    Str(Box<str>),
    /// Allowed number (compared as `f64`).
    Num(f64),
    /// Allowed boolean.
    Bool(bool),
    /// Allowed `null`.
    Null,
}

impl EnumValue {
    fn matches(&self, node: &NodeRef<'_>) -> bool {
        let resolved = node.resolved();
        match self {
            Self::Str(s) => {
                resolved.kind() == ValueKind::Str
                    && resolved.as_str().map(|v| v == &**s).unwrap_or(false)
            }
            Self::Num(n) => resolved.as_f64().is_some_and(|v| v == *n),
            Self::Bool(b) => resolved.as_bool() == Some(*b),
            Self::Null => resolved.kind() == ValueKind::Null,
        }
    }
}

/// A compiled `then` function with its options.
#[derive(Debug)]
pub(crate) enum Function {
    /// Passes when the matched node is truthy in Spectral's sense.
    Truthy,
    /// Passes when the matched node is falsy (`null`, `false`, empty string).
    Falsy,
    /// Key must exist; the given query selects the parent object.
    Defined { property: Box<str> },
    /// Passes when the named key does not exist on the matched object.
    Undefined { property: Box<str> },
    /// Passes when a matched string matches the compiled regex.
    Pattern(Regex),
    /// Passes only when a matched string does NOT match the compiled
    /// regex — Spectral's `notMatch` (forbidden-content rules).
    NotMatch(Regex),
    /// Passes when a matched string fits the configured casing convention.
    Casing(Casing),
    /// Passes when the string's character count or array's item count lies
    /// within `[min, max]` bounds; non-sized nodes always pass.
    Length { min: Option<f64>, max: Option<f64> },
    /// Passes when the node equals one of the allowed scalar values.
    Enumeration(Vec<EnumValue>),
    /// Passes when every key of the matched object is alphabetically sorted.
    Alphabetical,
    /// Passes when exactly one of the named properties is truthy.
    Xor { properties: Vec<Box<str>> },
    /// Native: every `{var}` in a path key is declared as an `in: path`
    /// parameter on every operation of the path item.
    PathParams,
    /// Native: `$ref` values carry no siblings besides description/summary.
    RefSiblings,
    /// Native: enum members agree with the schema's explicit type constraint.
    TypedEnum,
    /// Native: no duplicate keys under the selected object.
    DuplicateKeys,
    /// Native: every operation's `responses` has `default` or a 2XX entry.
    DefaultResponse,
    /// Native: every operation's `responses` has at least one 2XX entry.
    SuccessResponse,
    /// Native: every operation's `responses` has an entry in the status
    /// range (e.g. 400–499 for `operation-4xx-response`).
    StatusRange { low: u16, high: u16 },
    /// Native: `operationId` values must be unique across the document.
    UniqueOperationIds,
    /// Native: security requirements must name declared schemes
    /// (`securitySchemes` in 3.x, `securityDefinitions` in 2.0).
    SecurityDefined,
    /// Native: path keys must begin with a leading `/`.
    AbsolutePath,
    /// Native: path keys must not be identical modulo template variables.
    NoIdenticalPaths,
    /// Native: a parameter object must declare `schema` or `content`.
    ParameterSchemaOrContent,
    /// Native: path keys do not end in `/`.
    NoTrailingSlash,
}

impl Function {
    /// Default finding message when the rule carries no description.
    fn default_message(&self) -> &'static str {
        match self {
            Self::Truthy => "property must be truthy",
            Self::Falsy => "property must be falsy",
            Self::Defined { .. } => "property must be defined",
            Self::Undefined { .. } => "property must not be defined",
            Self::Pattern(_) => "value does not match the required pattern",
            Self::NotMatch(_) => "value contains forbidden content",
            Self::Casing(_) => "value does not match the required casing convention",
            Self::Length { .. } => "value length is out of bounds",
            Self::Enumeration(_) => "value is not one of the allowed values",
            Self::Alphabetical => "keys are not alphabetically sorted",
            Self::Xor { .. } => "exactly one of the properties must be present",
            Self::PathParams => "path template variable is not declared",
            Self::RefSiblings => "$ref must not have siblings other than description/summary",
            Self::TypedEnum => "enum member does not match the schema's declared type",
            Self::DuplicateKeys => "duplicate key",
            Self::DefaultResponse => "operation must define a default or 2XX response",
            Self::SuccessResponse => "operation must define at least one 2XX response",
            Self::StatusRange { .. } => {
                "operation must define a response in the required status range"
            }
            Self::UniqueOperationIds => "operationId must be unique",
            Self::SecurityDefined => "security requirement names an undeclared scheme",
            Self::AbsolutePath => "path must begin with a leading slash",
            Self::NoIdenticalPaths => "paths must not be identical modulo template variables",
            Self::ParameterSchemaOrContent => "parameter must define `schema` or `content`",
            Self::NoTrailingSlash => "path must not end with a trailing slash",
        }
    }
}

/// Spectral's truthiness: `null`, `false`, and the empty string are falsy;
/// every other node (including `0`, empty arrays/objects) is truthy.
pub(crate) fn is_truthy(node: &NodeRef<'_>) -> bool {
    let resolved = node.resolved();
    match resolved.kind() {
        ValueKind::Null => false,
        ValueKind::Bool => resolved.as_bool() == Some(true),
        ValueKind::Str => !resolved.as_str().unwrap_or("").is_empty(),
        ValueKind::Object | ValueKind::Array | ValueKind::Int | ValueKind::Float => true,
    }
}

/// The rule's finding message, computed lazily (only when a finding is
/// actually emitted) so hot no-finding loops stay allocation-free.
fn rule_message(rule: &Rule) -> String {
    rule.description
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("{}: {}", rule.code, rule.then.default_message()))
}

/// Applies `then` for one node matched by the rule's `given` query.
pub(crate) fn apply<'d>(
    rule: &Rule,
    node: NodeRef<'d>,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    match &rule.then {
        Function::Truthy => {
            if !is_truthy(&node) {
                push(out, rule, &node, ptrs);
            }
        }
        Function::Falsy => {
            if is_truthy(&node) {
                push(out, rule, &node, ptrs);
            }
        }
        Function::Defined { property } => {
            if node.get(property).is_none() {
                push(out, rule, &node, ptrs);
            }
        }
        Function::Undefined { property } => {
            if node.get(property).is_some() {
                push(out, rule, &node, ptrs);
            }
        }
        Function::Pattern(re) => {
            let resolved = node.resolved();
            if resolved.kind() == ValueKind::Str
                && let Some(s) = resolved.as_str()
                && !re.is_match(s)
            {
                push(out, rule, &node, ptrs);
            }
        }
        Function::NotMatch(re) => {
            let resolved = node.resolved();
            if resolved.kind() == ValueKind::Str
                && let Some(s) = resolved.as_str()
                && re.is_match(s)
            {
                push(out, rule, &node, ptrs);
            }
        }
        Function::Casing(casing) => {
            let resolved = node.resolved();
            if resolved.kind() == ValueKind::Str
                && let Some(s) = resolved.as_str()
                && !casing.matches(s)
            {
                push(out, rule, &node, ptrs);
            }
        }
        Function::Length { min, max } => {
            let resolved = node.resolved();
            let len = match resolved.kind() {
                ValueKind::Str => resolved.as_str().map(|s| s.chars().count() as f64),
                ValueKind::Array => Some(resolved.items().len() as f64),
                _ => None,
            };
            if let Some(len) = len {
                let too_small = min.is_some_and(|m| len < m);
                let too_large = max.is_some_and(|m| len > m);
                if too_small || too_large {
                    push(out, rule, &node, ptrs);
                }
            }
        }
        Function::Enumeration(values) => {
            if !values.iter().any(|v| v.matches(&node)) {
                push(out, rule, &node, ptrs);
            }
        }
        Function::Alphabetical => check_alphabetical(&node, rule, ptrs, out),
        Function::Xor { properties } => {
            let count = properties
                .iter()
                .filter(|p| node.get(p).is_some_and(|v| is_truthy(&v)))
                .count();
            if count != 1 {
                push(out, rule, &node, ptrs);
            }
        }
        Function::PathParams => check_path_params(&node, rule, ptrs, out),
        Function::RefSiblings => check_ref_siblings(&node, rule, ptrs, out),
        Function::TypedEnum => typed_enum::check(&node, rule, ptrs, out),
        Function::DuplicateKeys => check_duplicate_keys(&node, rule, ptrs, out),
        Function::DefaultResponse => check_response(&node, rule, ptrs, out, true),
        Function::SuccessResponse => check_response(&node, rule, ptrs, out, false),
        Function::StatusRange { low, high } => {
            check_status_range(&node, rule, ptrs, out, *low, *high);
        }
        Function::UniqueOperationIds => check_unique_operation_ids(&node, rule, ptrs, out),
        Function::SecurityDefined => check_security_defined(&node, rule, ptrs, out),
        Function::AbsolutePath => check_absolute_path(&node, rule, ptrs, out),
        Function::NoIdenticalPaths => check_no_identical_paths(&node, rule, ptrs, out),
        Function::ParameterSchemaOrContent => {
            check_parameter_schema_or_content(&node, rule, ptrs, out);
        }
        Function::NoTrailingSlash => check_no_trailing_slash(&node, rule, ptrs, out),
    }
}

fn push<'d>(
    out: &mut Vec<Finding<'d>>,
    rule: &Rule,
    node: &NodeRef<'d>,
    ptrs: &super::fast::PtrMap,
) {
    let path = ptrs
        .pointer_for(node)
        .unwrap_or_else(|| node.path_from_root());
    out.push(Finding {
        code: rule.code.clone(),
        severity: rule.severity,
        message: rule_message(rule),
        range: node.byte_range(),
        path,
        _marker: std::marker::PhantomData,
    });
}

fn push_at<'d>(
    out: &mut Vec<Finding<'d>>,
    rule: &Rule,
    message: String,
    range: std::ops::Range<usize>,
    path: Pointer,
) {
    out.push(Finding {
        code: rule.code.clone(),
        severity: rule.severity,
        message,
        range,
        path,
        _marker: std::marker::PhantomData,
    });
}

fn check_alphabetical<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    let resolved = node.resolved();
    let sorted = match resolved.kind() {
        ValueKind::Object => {
            let keys: Vec<&str> = resolved.entries().iter().map(|e| e.key).collect();
            keys.windows(2).all(|w| w[0] <= w[1])
        }
        ValueKind::Array => {
            let items = resolved.items();
            if items.iter().all(|i| i.resolved().kind() == ValueKind::Str) {
                let values: Vec<Option<&str>> = items.iter().map(NodeRef::as_str).collect();
                values.windows(2).all(|w| w[0] <= w[1])
            } else {
                return;
            }
        }
        _ => return,
    };
    if !sorted {
        push(out, rule, node, ptrs);
    }
}

const HTTP_METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Collects `{var}` template names from a path key.
fn template_vars(key: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut rest = key;
    while let Some(open) = rest.find('{') {
        let Some(close_rel) = rest[open + 1..].find('}') else {
            break;
        };
        let var = rest[open + 1..][..close_rel].trim();
        if !var.is_empty() {
            vars.push(var.to_string());
        }
        rest = &rest[open + 1 + close_rel + 1..];
    }
    vars
}

fn declared_path_params(params: Option<NodeRef<'_>>) -> Vec<String> {
    let Some(params) = params else {
        return Vec::new();
    };
    params
        .items()
        .into_iter()
        .filter_map(|p| {
            let p = p.resolved();
            if p.get("in").and_then(|v| v.as_str()) == Some("path") {
                p.get("name").and_then(|n| n.as_str()).map(str::to_string)
            } else {
                None
            }
        })
        .collect()
}

fn check_path_params(
    node: &NodeRef<'_>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'_>>,
) {
    let Some(path_ptr) = ptrs.pointer_for(node) else {
        return;
    };
    let Some(key_token) = path_ptr.tokens().last() else {
        return;
    };
    let key: &str = key_token;
    let vars = template_vars(key);
    if vars.is_empty() {
        return;
    }
    let path_level = declared_path_params(node.get("parameters"));
    for method in HTTP_METHODS {
        let Some(op) = node.get(method) else { continue };
        let mut declared = path_level.clone();
        declared.extend(declared_path_params(op.get("parameters")));
        for var in &vars {
            if !declared.iter().any(|d| d == var) {
                push_at(
                    out,
                    rule,
                    format!(
                        "Operation `{method}` of \"{key}\" does not declare path parameter {{{var}}}"
                    ),
                    op.byte_range(),
                    op.path_from_root(),
                );
            }
        }
    }
}

fn check_ref_siblings<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    // The `$ref` match is the value scalar; its parent object is the nearest
    // strict object ancestor. Climbing the syntax tree directly avoids the
    // O(depth x width) pointer round-trip per match, which dominated lint
    // time on reference-heavy documents.
    let Some(start) = node.resolved().syntax().parent() else {
        return;
    };
    let mut cur = NodeRef::new(start);
    while cur.kind() != ValueKind::Object {
        let Some(parent) = cur.syntax().parent() else {
            return;
        };
        cur = NodeRef::new(parent);
    }
    let parent = cur;
    let has_bad_sibling = parent
        .entries()
        .iter()
        .any(|e| e.key != "$ref" && e.key != "description" && e.key != "summary");
    if has_bad_sibling {
        push(out, rule, &parent, ptrs);
    }
}

fn check_duplicate_keys<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    _ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    let base = node.path_from_root();
    let resolved = node.resolved();
    if resolved.kind() != ValueKind::Object {
        return;
    }
    // Keys collide when identical, or when their `{var}` templates normalize
    // to the same shape (`/pets/{id}` vs `/pets/{name}`).
    let mut first_of_shape: rustc_hash::FxHashMap<String, (String, std::ops::Range<usize>)> =
        rustc_hash::FxHashMap::default();
    for entry in resolved.entries() {
        let shape = normalized_key(entry.key);
        let range = entry
            .value
            .map_or_else(|| node.byte_range(), |v| v.byte_range());
        if let Some((first_key, _)) = first_of_shape.get(&shape) {
            push_at(
                out,
                rule,
                format!(
                    "{} `{}` collides with `{}`",
                    rule_message(rule),
                    entry.key,
                    first_key
                ),
                range,
                base.push(entry.key),
            );
        } else {
            first_of_shape.insert(shape, (entry.key.to_string(), range));
        }
    }
}

/// Collapses `{...}` path templates to a single canonical `{}` segment.
fn normalized_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut rest = key;
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}').map_or(rest.len(), |c| open + c + 1);
        out.push_str(&rest[..open]);
        out.push_str("{}");
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

/// 2XX-shaped status keys: `200`-`299` plus `2XX`/`2xx` wildcard spellings.
fn is_2xx_key(key: &str) -> bool {
    key.len() == 3
        && key.starts_with('2')
        && key[1..]
            .chars()
            .all(|c| c.is_ascii_digit() || c == 'X' || c == 'x')
}

fn check_response<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
    allow_default: bool,
) {
    let ok = node.get("responses").is_some_and(|responses| {
        let resolved = responses.resolved();
        resolved.kind() == ValueKind::Object
            && resolved
                .entries()
                .iter()
                .any(|e| (e.key == "default" && allow_default) || is_2xx_key(e.key))
    });
    if !ok {
        push(out, rule, node, ptrs);
    }
}

fn check_status_range<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
    low: u16,
    high: u16,
) {
    let ok = node.get("responses").is_some_and(|responses| {
        let resolved = responses.resolved();
        resolved.kind() == ValueKind::Object
            && resolved.entries().iter().any(|e| {
                e.key
                    .parse::<u16>()
                    .is_ok_and(|status| (low..=high).contains(&status))
            })
    });
    if !ok {
        push(out, rule, node, ptrs);
    }
}

/// First occurrence of each `operationId`; later duplicates are findings.
fn check_unique_operation_ids<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    // `node` is the `paths` mapping; walk every operation under it.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path_entry in node.resolved().entries() {
        let Some(path_item) = path_entry.value else {
            continue;
        };
        for method in HTTP_METHODS {
            let Some(op) = path_item.get(method) else {
                continue;
            };
            let Some(id) = op.get("operationId") else {
                continue;
            };
            let Some(text) = id.resolved().as_str() else {
                continue;
            };
            if !seen.insert(text.to_owned()) {
                push(out, rule, &id, ptrs);
            }
        }
    }
}

/// Every scheme named in the security array must exist under the doc's
/// declared schemes (`components/securitySchemes` in 3.x,
/// `securityDefinitions` in 2.0).
fn check_security_defined<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    let doc_root = NodeRef::new(node.syntax().doc().root());
    let Some(known) = doc_root
        .get("components")
        .and_then(|c| c.get("securitySchemes"))
        .or_else(|| doc_root.get("securityDefinitions"))
        .map(|schemes| {
            schemes
                .resolved()
                .entries()
                .into_iter()
                .map(|e| e.key.to_owned())
                .collect::<Vec<_>>()
        })
    else {
        return; // no declared schemes at all: nothing can be checked here
    };
    let resolved = node.resolved();
    if resolved.kind() != ValueKind::Array {
        return;
    }
    for requirement in resolved.items() {
        for entry in requirement.resolved().entries() {
            if !known.iter().any(|k| k == entry.key) {
                push(out, rule, &entry.key_node, ptrs);
            }
        }
    }
}

/// Path keys must begin with `/`.
fn check_absolute_path<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    let starts_ok = match ptrs.own_key(node) {
        Some(k) => k.first() == Some(&b'/'),
        None => node
            .path_from_root()
            .tokens()
            .last()
            .is_some_and(|k| k.starts_with('/')),
    };
    if !starts_ok {
        push(out, rule, node, ptrs);
    }
}

/// Path keys must not collapse to the same shape when `{var}` templates
/// are erased (`/pets/{id}` vs `/pets/{petId}`).
fn check_no_identical_paths<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    let resolved = node.resolved();
    if resolved.kind() != ValueKind::Object {
        return;
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in resolved.entries() {
        let normalized = normalize_path_key(entry.key);
        if !seen.insert(normalized) {
            push(out, rule, &entry.key_node, ptrs);
        }
    }
}

/// Erases `{var}` templates so `/pets/{id}` and `/pets/{petId}` compare
/// equal.
fn normalize_path_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut rest = key;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close_rel) = rest[open + 1..].find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        out.push_str("{}");
        rest = &rest[open + 1 + close_rel + 1..];
    }
    out.push_str(rest);
    out
}

/// A parameter object must declare `schema` (3.0/2.0 style) or `content`
/// (3.1 style); `$ref` parameters pass (their target carries the shape).
fn check_parameter_schema_or_content<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    if node.get("$ref").is_some() || node.get("schema").is_some() || node.get("content").is_some() {
        return;
    }
    push(out, rule, node, ptrs);
}

fn check_no_trailing_slash<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &super::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    // Cheap pre-filter: the node's own addressing key from the pointer map
    // (O(1)); fall back to the full pointer computation when unvisited.
    let ends_with_slash = match ptrs.own_key(node) {
        Some(k) => k.ends_with(b"/"),
        None => node
            .path_from_root()
            .tokens()
            .last()
            .is_some_and(|k| k.ends_with('/')),
    };
    if ends_with_slash {
        push(out, rule, node, ptrs);
    }
}

#[cfg(test)]
pub(crate) mod tests;
