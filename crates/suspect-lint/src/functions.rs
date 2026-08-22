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
    /// Native: all members of an `enum` array share one scalar kind.
    TypedEnum,
    /// Native: no duplicate keys under the selected object.
    DuplicateKeys,
    /// Native: every operation's `responses` has `default` or a 2XX entry.
    DefaultResponse,
    /// Native: every operation's `responses` has at least one 2XX entry.
    SuccessResponse,
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
            Self::Casing(_) => "value does not match the required casing convention",
            Self::Length { .. } => "value length is out of bounds",
            Self::Enumeration(_) => "value is not one of the allowed values",
            Self::Alphabetical => "keys are not alphabetically sorted",
            Self::Xor { .. } => "exactly one of the properties must be present",
            Self::PathParams => "path template variable is not declared",
            Self::RefSiblings => "$ref must not have siblings other than description/summary",
            Self::TypedEnum => "enum members must share a single scalar type",
            Self::DuplicateKeys => "duplicate key",
            Self::DefaultResponse => "operation must define a default or 2XX response",
            Self::SuccessResponse => "operation must define at least one 2XX response",
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

/// Applies `then` for one node matched by the rule's `given` query.
pub(crate) fn apply<'d>(
    rule: &Rule,
    node: NodeRef<'d>,
    root: NodeRef<'d>,
    out: &mut Vec<Finding<'d>>,
) {
    let message = rule
        .description
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("{}: {}", rule.code, rule.then.default_message()));
    match &rule.then {
        Function::Truthy => {
            if !is_truthy(&node) {
                push(out, rule, &message, &node);
            }
        }
        Function::Falsy => {
            if is_truthy(&node) {
                push(out, rule, &message, &node);
            }
        }
        Function::Defined { property } => {
            if node.get(property).is_none() {
                push(out, rule, &message, &node);
            }
        }
        Function::Undefined { property } => {
            if node.get(property).is_some() {
                push(out, rule, &message, &node);
            }
        }
        Function::Pattern(re) => {
            let resolved = node.resolved();
            if resolved.kind() == ValueKind::Str
                && let Some(s) = resolved.as_str()
                && !re.is_match(s)
            {
                push(out, rule, &message, &node);
            }
        }
        Function::Casing(casing) => {
            let resolved = node.resolved();
            if resolved.kind() == ValueKind::Str
                && let Some(s) = resolved.as_str()
                && !casing.matches(s)
            {
                push(out, rule, &message, &node);
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
                    push(out, rule, &message, &node);
                }
            }
        }
        Function::Enumeration(values) => {
            if !values.iter().any(|v| v.matches(&node)) {
                push(out, rule, &message, &node);
            }
        }
        Function::Alphabetical => check_alphabetical(&node, rule, &message, out),
        Function::Xor { properties } => {
            let count = properties
                .iter()
                .filter(|p| node.get(p).is_some_and(|v| is_truthy(&v)))
                .count();
            if count != 1 {
                push(out, rule, &message, &node);
            }
        }
        Function::PathParams => check_path_params(&node, rule, out),
        Function::RefSiblings => check_ref_siblings(&node, root, rule, &message, out),
        Function::TypedEnum => check_typed_enum(&node, rule, &message, out),
        Function::DuplicateKeys => check_duplicate_keys(&node, rule, &message, out),
        Function::DefaultResponse => check_response(&node, rule, &message, out, true),
        Function::SuccessResponse => check_response(&node, rule, &message, out, false),
        Function::NoTrailingSlash => check_no_trailing_slash(&node, rule, &message, out),
    }
}

fn push<'d>(out: &mut Vec<Finding<'d>>, rule: &Rule, message: &str, node: &NodeRef<'d>) {
    out.push(Finding {
        code: rule.code.clone(),
        severity: rule.severity,
        message: message.to_string(),
        range: node.byte_range(),
        path: node.path_from_root(),
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
    message: &str,
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
        push(out, rule, message, node);
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

fn check_path_params<'d>(node: &NodeRef<'d>, rule: &Rule, out: &mut Vec<Finding<'d>>) {
    let path_ptr = node.path_from_root();
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
    root: NodeRef<'d>,
    rule: &Rule,
    message: &str,
    out: &mut Vec<Finding<'d>>,
) {
    let ref_ptr = node.path_from_root();
    let Some(parent_ptr) = ref_ptr.parent() else {
        return;
    };
    let Some(parent) = root.pointer(&parent_ptr) else {
        return;
    };
    let has_bad_sibling = parent
        .entries()
        .iter()
        .any(|e| e.key != "$ref" && e.key != "description" && e.key != "summary");
    if has_bad_sibling {
        push(out, rule, message, &parent);
    }
}

/// Groups enum members by scalar kind (`Int`/`Float` count as one `number`
/// group); a mixed enum is reported once at the enum node.
fn check_typed_enum<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    message: &str,
    out: &mut Vec<Finding<'d>>,
) {
    let resolved = node.resolved();
    if resolved.kind() != ValueKind::Array {
        return;
    }
    let group = |kind: ValueKind| match kind {
        ValueKind::Int | ValueKind::Float => 'n',
        ValueKind::Null => '0',
        ValueKind::Bool => 'b',
        ValueKind::Str => 's',
        ValueKind::Object => 'o',
        ValueKind::Array => 'a',
    };
    let mut kinds = resolved
        .items()
        .into_iter()
        .map(|i| group(i.resolved().kind()));
    let Some(first) = kinds.next() else { return };
    if kinds.any(|k| k != first) {
        push(out, rule, message, node);
    }
}

fn check_duplicate_keys<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    message: &str,
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
                format!("{message} `{}` collides with `{}`", entry.key, first_key),
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
    message: &str,
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
        push(out, rule, message, node);
    }
}

fn check_no_trailing_slash<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    message: &str,
    out: &mut Vec<Finding<'d>>,
) {
    let ptr = node.path_from_root();
    if let Some(key) = ptr.tokens().last()
        && key.ends_with('/')
    {
        push(out, rule, message, node);
    }
}

#[cfg(test)]
pub(crate) mod tests;
