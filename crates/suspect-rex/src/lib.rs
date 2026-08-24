#![deny(missing_docs)]
//! suspect-rex: Arazzo runtime expressions ("rex").
//!
//! A runtime expression is a string beginning with `$` that references data
//! from the running workflow: the HTTP exchange of the current step, the
//! workflow inputs, previous step outputs, or the source descriptions of the
//! Arazzo document. Expressions that do not start with `$` are literal text.
//!
//! Grammar (subset of [Arazzo 1.0.0 §4.4.3]):
//!
//! ```text
//! rex                = "$method" | "$statusCode" | "$url"
//!                    | "$request." location | "$response." location
//!                    | "$inputs." name | "$inputs#" json-pointer-fragment
//!                    | "$steps." step-id ".outputs." name
//!                    | "$steps." step-id ".outputs#" json-pointer-fragment
//!                    | "$sourceDescriptions." name "#" json-pointer-fragment
//! location           = "header." name | "query." name | "path." name
//!                    | "body" [ "#" json-pointer-fragment ]
//! ```
//!
//! Pointer fragments follow RFC 6901 §6: percent escapes (`%XX`) are decoded
//! across the whole fragment first, then the resulting JSON Pointer is
//! evaluated (`~1` decodes to `/`, `~0` to `~`). An empty fragment (`#`)
//! addresses the root document.
//!
//! `$url` is recognized by the Arazzo grammar but has no representation in
//! [`Rex`] and cannot be evaluated from [`RexCtx`]; [`parse_rex`] rejects it
//! with a descriptive error.
//!
//! [Arazzo 1.0.0 §4.4.3]: https://spec.openapis.org/arazzo/v1.0.0#runtime-expressions

use std::collections::HashMap;

use std::fmt;
use std::sync::LazyLock;

use suspect_low::{Pointer, percent_decode_fragment};

#[cfg(test)]
mod tests;

/// Part of an HTTP exchange addressed by a `$request`/`$response` expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// A header value; matched case-insensitively during evaluation.
    Header(String),
    /// A query parameter.
    Query(String),
    /// A path parameter.
    Path(String),
    /// The request/response body, optionally narrowed by a JSON pointer.
    Body,
}

/// A parsed Arazzo runtime expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rex {
    /// `$method` — the HTTP method of the current request.
    Method,
    /// `$statusCode` — the response status code of the current step.
    StatusCode,
    /// `$request.<part>` — data from the outgoing HTTP request.
    Request {
        /// Which part of the exchange is addressed.
        part: Part,
        /// RFC 6901 pointer applied to the body; root for non-body parts.
        pointer: Pointer,
    },
    /// `$response.<part>` — data from the received HTTP response.
    Response {
        /// Which part of the exchange is addressed.
        part: Part,
        /// RFC 6901 pointer applied to the body; root for non-body parts.
        pointer: Pointer,
    },
    /// `$inputs.name` or `$inputs#/pointer` — a workflow input value.
    ///
    /// When the expression addresses a nested value by JSON pointer, `key`
    /// holds the canonical fragment (`#/a/b`) and evaluation resolves it as
    /// an RFC 6901 pointer into the inputs object.
    Inputs {
        /// Input key, or a `#`-prefixed JSON pointer fragment.
        key: String,
    },
    /// `$steps.<step>.outputs.<key>` or `$steps.<step>.outputs#/pointer`.
    Steps {
        /// The referenced step id.
        step: String,
        /// Output key within that step's outputs, or a `#`-prefixed JSON
        /// pointer fragment resolved against the outputs object.
        outputs_key: String,
    },
    /// `$sourceDescriptions.<name>#/pointer`.
    SourceDescriptions {
        /// The source description name.
        name: String,
        /// RFC 6901 pointer into the named description document.
        pointer: Pointer,
    },
    /// Any input not starting with `$`, passed through verbatim.
    Text(String),
}

/// Error produced when a string cannot be parsed as a runtime expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RexError {
    message: String,
}

impl fmt::Display for RexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RexError {}

/// Context used to evaluate a [`Rex`] against one HTTP exchange and the
/// surrounding workflow state.
///
/// Construct with [`RexCtx::default`] (or `Default::default()`) and fill in
/// fields through the builder-style setters:
///
/// ```
/// use suspect_rex::RexCtx;
///
/// let ctx = RexCtx::default()
///     .method("POST")
///     .status(201)
///     .request_body(r#"{"id": 7}"#);
/// ```
#[derive(Debug)]
pub struct RexCtx<'a> {
    method: &'a str,
    status: u16,
    request_headers: &'a [(String, String)],
    response_headers: &'a [(String, String)],
    request_body: &'a str,
    response_body: &'a str,
    inputs: &'a serde_json::Map<String, serde_json::Value>,
    steps_outputs: &'a serde_json::Map<String, serde_json::Value>,
    source_descriptions: &'a HashMap<String, String>,
}

static EMPTY_MAP: LazyLock<serde_json::Map<String, serde_json::Value>> =
    LazyLock::new(serde_json::Map::new);
static EMPTY_DESCRIPTIONS: LazyLock<HashMap<String, String>> = LazyLock::new(HashMap::new);

impl Default for RexCtx<'_> {
    fn default() -> Self {
        Self {
            method: "",
            status: 0,
            request_headers: &[],
            response_headers: &[],
            request_body: "",
            response_body: "",
            inputs: &EMPTY_MAP,
            steps_outputs: &EMPTY_MAP,
            source_descriptions: &EMPTY_DESCRIPTIONS,
        }
    }
}

impl<'a> RexCtx<'a> {
    /// Sets the HTTP method (e.g. `"POST"`), read by `$method`.
    #[must_use]
    pub fn method(mut self, method: &'a str) -> Self {
        self.method = method;
        self
    }

    /// Sets the response status code, read by `$statusCode`.
    #[must_use]
    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Sets the request headers, read by `$request.header.X`.
    #[must_use]
    pub fn request_headers(mut self, headers: &'a [(String, String)]) -> Self {
        self.request_headers = headers;
        self
    }

    /// Sets the response headers, read by `$response.header.X`.
    #[must_use]
    pub fn response_headers(mut self, headers: &'a [(String, String)]) -> Self {
        self.response_headers = headers;
        self
    }

    /// Sets the raw request body text (JSON when applicable), read by
    /// `$request.body#/...`.
    #[must_use]
    pub fn request_body(mut self, body: &'a str) -> Self {
        self.request_body = body;
        self
    }

    /// Sets the raw response body text (JSON when applicable), read by
    /// `$response.body#/...`.
    #[must_use]
    pub fn response_body(mut self, body: &'a str) -> Self {
        self.response_body = body;
        self
    }

    /// Sets the workflow inputs map, read by `$inputs...`.
    #[must_use]
    pub fn inputs(mut self, inputs: &'a serde_json::Map<String, serde_json::Value>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Sets the accumulated step outputs, keyed by step id; each value is
    /// that step's outputs object. Read by `$steps...`.
    #[must_use]
    pub fn steps_outputs(
        mut self,
        steps_outputs: &'a serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        self.steps_outputs = steps_outputs;
        self
    }

    /// Sets the source description documents, keyed by name with JSON text
    /// values. Read by `$sourceDescriptions...`.
    #[must_use]
    pub fn source_descriptions(mut self, source_descriptions: &'a HashMap<String, String>) -> Self {
        self.source_descriptions = source_descriptions;
        self
    }

    /// The configured HTTP method.
    #[must_use]
    pub fn get_method(&self) -> &str {
        self.method
    }
}

/// Parses an Arazzo runtime expression.
/// Inputs not starting with `$` parse to [`Rex::Text`]. See the crate docs
/// for the supported grammar and error conditions.
///
/// # Errors
/// Returns a [`RexError`] describing the position and reason when a `$`-led
/// input does not match the grammar (unknown expression, empty names,
/// malformed pointers, trailing garbage).
pub fn parse_rex(input: &str) -> Result<Rex, RexError> {
    let Some(rest) = input.strip_prefix('$') else {
        return Ok(Rex::Text(input.to_owned()));
    };
    let err = |reason: String| Err(error(input, &reason));

    if rest == "method" {
        return Ok(Rex::Method);
    }
    if rest == "statusCode" {
        return Ok(Rex::StatusCode);
    }
    if rest == "url" {
        return err("`$url` is recognized by the Arazzo grammar but has no \
                    Rex representation in this crate"
            .to_owned());
    }

    // The head segment ends at the first `.` or `#`, whichever comes first:
    // `$inputs#/a/b` carries no dot at the boundary, while dots may occur
    // inside keys, step ids and source description names.
    let cut = match (rest.find('.'), rest.find('#')) {
        (Some(dot), Some(hash)) => dot.min(hash),
        (Some(dot), None) => dot,
        (None, Some(hash)) => hash,
        (None, None) => {
            return err(format!(
                "expected `method`, `statusCode`, `url`, `request`, \
                 `response`, `inputs`, `steps` or `sourceDescriptions` after \
                 `$`, found `{rest}`"
            ));
        }
    };
    let (head, tail) = (&rest[..cut], &rest[cut..]);
    if head.is_empty() {
        return err(format!("missing expression name before `{head}{tail}`"));
    }

    match head {
        "request" | "response" => {
            let (part, pointer_src) = parse_exchange_part(tail)
                .ok_or_else(|| error(input, &format!("invalid `{head}` location `{tail}`")))?;
            let pointer = match pointer_src {
                Some(frag) => parse_pointer_fragment(input, frag)?,
                None => Pointer::root(),
            };
            Ok(if head == "request" {
                Rex::Request { part, pointer }
            } else {
                Rex::Response { part, pointer }
            })
        }
        "inputs" => {
            let key = match tail.strip_prefix('#') {
                Some(frag) => format!("#{}", parse_pointer_fragment(input, frag)?.to_path()),
                None => match tail.strip_prefix('.') {
                    Some(key) if !key.is_empty() => key.to_owned(),
                    _ => return err("missing input key after `$inputs.`".to_owned()),
                },
            };
            Ok(Rex::Inputs { key })
        }
        "steps" => {
            let Some(after_head) = tail.strip_prefix('.') else {
                return err(format!("missing `.outputs.<key>` after `$steps{tail}`"));
            };
            let Some((step, after_step)) = after_head.split_once('.') else {
                return err(format!(
                    "missing `.outputs.<key>` after `$steps.{after_head}`"
                ));
            };
            let outputs_key = if let Some(key) = after_step.strip_prefix("outputs.") {
                if key.is_empty() {
                    return err("missing output key after `$steps.<id>.outputs.`".to_owned());
                }
                key.to_owned()
            } else if let Some(frag) = after_step.strip_prefix("outputs#") {
                format!("#{}", parse_pointer_fragment(input, frag)?.to_path())
            } else {
                return err(format!(
                    "expected `.outputs.<key>` or `.outputs#/pointer` after \
                     `$steps.{step}.`, found `.{after_step}`"
                ));
            };
            if step.is_empty() || step.contains('#') {
                return err(format!("invalid step id `{step}`"));
            }
            Ok(Rex::Steps {
                step: step.to_owned(),
                outputs_key,
            })
        }
        "sourceDescriptions" => {
            let Some(name_and_frag) = tail.strip_prefix('.') else {
                return err("missing source description name".to_owned());
            };
            let Some((name, frag)) = name_and_frag.split_once('#') else {
                return err("`$sourceDescriptions` requires a `#/pointer` fragment".to_owned());
            };
            if name.is_empty() {
                return err("missing source description name".to_owned());
            }
            let pointer = parse_pointer_fragment(input, frag)?;
            Ok(Rex::SourceDescriptions {
                name: name.to_owned(),
                pointer,
            })
        }
        other => err(format!(
            "unknown runtime expression `${other}`; expected `method`, \
             `statusCode`, `url`, `request`, `response`, `inputs`, `steps` \
             or `sourceDescriptions`"
        )),
    }
}

/// Parses `.header.X` / `#fragment` following `$request` / `$response`;
/// `tail` starts at the delimiter after the head segment.
fn parse_exchange_part(tail: &str) -> Option<(Part, Option<&str>)> {
    let tail = tail.strip_prefix('.').unwrap_or(tail);
    if let Some(name) = tail.strip_prefix("header.") {
        return (!name.is_empty()).then(|| (Part::Header(name.to_owned()), None));
    }
    if let Some(name) = tail.strip_prefix("query.") {
        return (!name.is_empty()).then(|| (Part::Query(name.to_owned()), None));
    }
    if let Some(name) = tail.strip_prefix("path.") {
        return (!name.is_empty()).then(|| (Part::Path(name.to_owned()), None));
    }
    if let Some(frag) = tail.strip_prefix("body#") {
        return Some((Part::Body, Some(frag)));
    }
    (tail == "body").then_some((Part::Body, None))
}

/// Parses and validates a `#/pointer` fragment body.
fn parse_pointer_fragment(full: &str, frag: &str) -> Result<Pointer, RexError> {
    validate_fragment_chars(full, frag)?;
    // RFC 6901 §6: percent-decode the entire fragment first; the resulting
    // JSON Pointer is then ~-unescaped during parsing.
    let decoded_bytes = percent_decode_fragment(frag);
    let decoded = String::from_utf8_lossy(&decoded_bytes);
    Pointer::parse(&decoded).map_err(|_| error(full, &format!("invalid JSON pointer `#{frag}`")))
}

/// Rejects characters that cannot appear in a URI fragment / JSON pointer:
/// control characters, whitespace, `[`, `]` and a nested `#`.
fn validate_fragment_chars(full: &str, frag: &str) -> Result<(), RexError> {
    for ch in frag.chars() {
        if ch.is_control() || matches!(ch, ' ' | '[' | ']' | '#') {
            return Err(error(
                full,
                &format!("invalid character {ch:?} in pointer fragment `#{frag}`"),
            ));
        }
    }
    Ok(())
}

fn error(input: &str, reason: &str) -> RexError {
    RexError {
        message: format!("invalid runtime expression `{input}` at offset 0: {reason}"),
    }
}

/// Evaluates a parsed expression against `ctx`, returning `None` when the
/// addressed value does not exist (missing header/key, non-JSON body,
/// unresolvable pointer).
#[must_use]
pub fn eval_rex(rex: &Rex, ctx: &RexCtx<'_>) -> Option<serde_json::Value> {
    match rex {
        Rex::Method => Some(serde_json::Value::String(ctx.method.to_owned())),
        Rex::StatusCode => Some(serde_json::Value::from(ctx.status)),
        Rex::Request { part, pointer } => eval_part(part, pointer, ctx, Side::Request),
        Rex::Response { part, pointer } => eval_part(part, pointer, ctx, Side::Response),
        Rex::Inputs { key } => match key.strip_prefix('#') {
            Some(path) => resolve_in_inputs(ctx.inputs, path),
            None => ctx.inputs.get(key).cloned(),
        },
        Rex::Steps { step, outputs_key } => {
            let outputs = ctx.steps_outputs.get(step)?;
            match outputs_key.strip_prefix('#') {
                Some(path) => resolve_path(outputs, path),
                None => outputs.get(outputs_key).cloned(),
            }
        }
        Rex::SourceDescriptions { name, pointer } => {
            let doc = parse_json(ctx.source_descriptions.get(name)?)?;
            resolve_pointer(&doc, pointer).cloned()
        }
        Rex::Text(text) => Some(serde_json::Value::String(text.clone())),
    }
}

/// Resolves a `/a/b` path against any JSON value root.
fn resolve_path(value: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    let pointer = Pointer::parse(path).ok()?;
    resolve_pointer(value, &pointer).cloned()
}

enum Side {
    Request,
    Response,
}

fn eval_part(
    part: &Part,
    pointer: &Pointer,
    ctx: &RexCtx<'_>,
    side: Side,
) -> Option<serde_json::Value> {
    match part {
        Part::Header(name) => {
            let headers = match side {
                Side::Request => ctx.request_headers,
                Side::Response => ctx.response_headers,
            };
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| serde_json::Value::String(value.clone()))
        }
        // Query/path parameters are not carried on `RexCtx`; nothing to
        // evaluate until the executor passes them in.
        Part::Query(_) | Part::Path(_) => None,
        Part::Body => {
            let body = match side {
                Side::Request => ctx.request_body,
                Side::Response => ctx.response_body,
            };
            let doc = parse_json(body)?;
            resolve_pointer(&doc, pointer).cloned()
        }
    }
}

fn parse_json(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str(text).ok()
}

/// Array index accepted by RFC 6901 evaluation: `"0"` or `[1-9][0-9]*` —
/// no sign and no leading zeros.
fn array_index(token: &str) -> Option<usize> {
    let well_formed = match token.as_bytes() {
        b"0" => true,
        [first, rest @ ..] if first.is_ascii_digit() && *first != b'0' => {
            rest.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    };
    if well_formed {
        token.parse().ok()
    } else {
        None
    }
}

/// RFC 6901 resolution over a pointer parsed from a fragment.
fn resolve_pointer<'v>(
    value: &'v serde_json::Value,
    pointer: &Pointer,
) -> Option<&'v serde_json::Value> {
    let mut current = value;
    for token in pointer.tokens() {
        current = match current {
            serde_json::Value::Object(map) => map.get(token.as_ref())?,
            serde_json::Value::Array(items) => items.get(array_index(token)?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Resolves a `/a/b` path (already validated at parse time) against the
/// workflow inputs object, which is the document root.
fn resolve_in_inputs(
    root: &serde_json::Map<String, serde_json::Value>,
    path: &str,
) -> Option<serde_json::Value> {
    let tokens = Pointer::parse(path).ok()?;
    let Some((first, rest)) = tokens.tokens().split_first() else {
        // Root fragment: the whole inputs object.
        return Some(serde_json::Value::Object(root.clone()));
    };
    let mut current = root.get(first.as_ref())?;
    for token in rest {
        current = match current {
            serde_json::Value::Object(map) => map.get(token.as_ref())?,
            serde_json::Value::Array(items) => items.get(array_index(token)?)?,
            _ => return None,
        };
    }
    Some(current.clone())
}
