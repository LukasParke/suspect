//! Dependency-free exact JSON runtime emitted into generated Rust packages.
//!
//! It refers only to the generated package's `JsonValue`, `JsonNonNullValue`,
//! `JsonNumber`, and `Nullable` types plus Rust's standard library.

use crate::{JsonNonNullValue, JsonNumber, JsonValue, Nullable};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::str::FromStr as _;

/// Finite resource limits shared by exact JSON parsing and writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonLimits {
    /// Maximum encoded input length, checked before parsing.
    pub max_input_bytes: usize,
    /// Maximum encoded output length, checked before every append.
    pub max_output_bytes: usize,
    /// Maximum collection nesting, additionally capped by the runtime ceiling.
    pub max_depth: usize,
    /// Maximum parser/writer work units for this operation.
    pub max_work: usize,
}

impl Default for JsonLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 8 * 1024 * 1024,
            max_output_bytes: 8 * 1024 * 1024,
            max_depth: 128,
            max_work: 32 * 1024 * 1024,
        }
    }
}

/// Stable categories returned by the exact JSON runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonErrorKind {
    /// The input is not JSON or contains an invalid string escape.
    Syntax,
    /// Byte input is not well-formed UTF-8.
    InvalidUtf8,
    /// An object repeats the same decoded property name.
    DuplicateKey,
    /// A byte, depth, output, or work allowance was exhausted.
    ResourceLimit,
}

/// Bounded JSON failure information which never retains the input document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    /// Stable error category for programmatic handling.
    pub kind: JsonErrorKind,
    /// Input byte offset when the error belongs to a specific input location.
    pub offset: Option<usize>,
    message: &'static str,
}

impl JsonError {
    fn at(kind: JsonErrorKind, offset: usize, message: &'static str) -> Self {
        Self {
            kind,
            offset: Some(offset),
            message,
        }
    }
    fn limit(offset: Option<usize>, message: &'static str) -> Self {
        Self {
            kind: JsonErrorKind::ResourceLimit,
            offset,
            message,
        }
    }
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.offset {
            Some(offset) => write!(f, "{} at byte {}", self.message, offset),
            None => f.write_str(self.message),
        }
    }
}
impl std::error::Error for JsonError {}

/// Parse UTF-8 bytes as one exact JSON value.
///
/// Duplicate keys, invalid UTF-8, malformed escapes, and unpaired escaped
/// UTF-16 surrogates are rejected. Number spellings are retained exactly.
pub fn parse_json_bytes(input: &[u8], limits: JsonLimits) -> Result<JsonValue, JsonError> {
    if input.len() > limits.max_input_bytes {
        return Err(JsonError::limit(None, "JSON input exceeds its byte limit"));
    }
    let text = std::str::from_utf8(input).map_err(|error| {
        JsonError::at(
            JsonErrorKind::InvalidUtf8,
            error.valid_up_to(),
            "JSON input is not UTF-8",
        )
    })?;
    parse_json(text, limits)
}

/// Parse one exact JSON value from a Rust UTF-8 string.
pub fn parse_json(input: &str, limits: JsonLimits) -> Result<JsonValue, JsonError> {
    if input.len() > limits.max_input_bytes {
        return Err(JsonError::limit(None, "JSON input exceeds its byte limit"));
    }
    enforce_depth_ceiling(limits.max_depth)?;
    let mut parser = Parser {
        input: input.as_bytes(),
        at: 0,
        work: limits.max_work,
        limits,
    };
    parser.ws()?;
    let value = parser.value(0)?;
    parser.ws()?;
    if parser.at != parser.input.len() {
        return Err(parser.syntax("unexpected trailing JSON input"));
    }
    Ok(value)
}

/// Write an exact JSON value deterministically. Object keys use `BTreeMap`
/// order and number tokens retain their original spelling.
pub fn stringify_json(value: &JsonValue, limits: JsonLimits) -> Result<String, JsonError> {
    enforce_depth_ceiling(limits.max_depth)?;
    let initial = limits.max_output_bytes.min(4096);
    let mut writer = Writer {
        output: String::with_capacity(initial),
        work: limits.max_work,
        limits,
    };
    writer.value(value, 0)?;
    Ok(writer.output)
}

// Recursive descent stays simple and auditable, but callers cannot turn its
// schema-facing limit into an unbounded native stack request.
const HARD_DEPTH_CEILING: usize = 256;

fn enforce_depth_ceiling(depth: usize) -> Result<(), JsonError> {
    if depth > HARD_DEPTH_CEILING {
        Err(JsonError::limit(
            None,
            "JSON nesting limit exceeds the runtime ceiling",
        ))
    } else {
        Ok(())
    }
}

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
    work: usize,
    limits: JsonLimits,
}

impl Parser<'_> {
    fn spend(&mut self, amount: usize) -> Result<(), JsonError> {
        self.work = self
            .work
            .checked_sub(amount)
            .ok_or_else(|| JsonError::limit(Some(self.at), "JSON work limit exceeded"))?;
        Ok(())
    }
    fn syntax(&self, message: &'static str) -> JsonError {
        JsonError::at(JsonErrorKind::Syntax, self.at, message)
    }
    fn ws(&mut self) -> Result<(), JsonError> {
        while matches!(self.input.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.spend(1)?;
            self.at += 1;
        }
        Ok(())
    }
    fn value(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        self.spend(1)?;
        match self.input.get(self.at).copied() {
            Some(b'n') => {
                self.literal(b"null")?;
                Ok(Nullable::Null)
            }
            Some(b't') => {
                self.literal(b"true")?;
                Ok(Nullable::Value(JsonNonNullValue::Bool(true)))
            }
            Some(b'f') => {
                self.literal(b"false")?;
                Ok(Nullable::Value(JsonNonNullValue::Bool(false)))
            }
            Some(b'"') => Ok(Nullable::Value(JsonNonNullValue::String(self.string()?))),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.syntax("expected a JSON value")),
        }
    }
    fn literal(&mut self, token: &[u8]) -> Result<(), JsonError> {
        self.spend(token.len())?;
        if self.input.get(self.at..self.at + token.len()) != Some(token) {
            return Err(self.syntax("invalid JSON literal"));
        }
        self.at += token.len();
        Ok(())
    }
    fn enter(&self, depth: usize) -> Result<usize, JsonError> {
        let next = depth
            .checked_add(1)
            .ok_or_else(|| JsonError::limit(Some(self.at), "JSON nesting limit exceeded"))?;
        if next > self.limits.max_depth {
            return Err(JsonError::limit(
                Some(self.at),
                "JSON nesting limit exceeded",
            ));
        }
        Ok(next)
    }
    fn array(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        let depth = self.enter(depth)?;
        self.at += 1;
        let mut values = Vec::new();
        self.ws()?;
        if self.input.get(self.at) == Some(&b']') {
            self.at += 1;
            return Ok(Nullable::Value(JsonNonNullValue::Array(values)));
        }
        loop {
            values.push(self.value(depth)?);
            self.ws()?;
            match self.input.get(self.at) {
                Some(b',') => {
                    self.at += 1;
                    self.ws()?;
                }
                Some(b']') => {
                    self.at += 1;
                    break;
                }
                _ => return Err(self.syntax("expected comma or end of JSON array")),
            }
        }
        Ok(Nullable::Value(JsonNonNullValue::Array(values)))
    }
    fn object(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        let depth = self.enter(depth)?;
        self.at += 1;
        let mut values = BTreeMap::new();
        self.ws()?;
        if self.input.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(Nullable::Value(JsonNonNullValue::Object(values)));
        }
        loop {
            let key_at = self.at;
            let key = self.string()?;
            self.ws()?;
            if self.input.get(self.at) != Some(&b':') {
                return Err(self.syntax("expected colon after JSON property name"));
            }
            self.at += 1;
            self.ws()?;
            let value = self.value(depth)?;
            if values.insert(key, value).is_some() {
                return Err(JsonError::at(
                    JsonErrorKind::DuplicateKey,
                    key_at,
                    "duplicate JSON property",
                ));
            }
            self.ws()?;
            match self.input.get(self.at) {
                Some(b',') => {
                    self.at += 1;
                    self.ws()?;
                }
                Some(b'}') => {
                    self.at += 1;
                    break;
                }
                _ => return Err(self.syntax("expected comma or end of JSON object")),
            }
        }
        Ok(Nullable::Value(JsonNonNullValue::Object(values)))
    }
    fn string(&mut self) -> Result<String, JsonError> {
        if self.input.get(self.at) != Some(&b'"') {
            return Err(self.syntax("expected a quoted JSON string"));
        }
        self.at += 1;
        let mut output = String::new();
        loop {
            let byte = *self
                .input
                .get(self.at)
                .ok_or_else(|| self.syntax("unterminated JSON string"))?;
            self.spend(1)?;
            self.at += 1;
            match byte {
                b'"' => return Ok(output),
                0x00..=0x1f => {
                    return Err(self.syntax("unescaped control character in JSON string"));
                }
                b'\\' => self.escape(&mut output)?,
                0x20..=0x7f => output.push(char::from(byte)),
                _ => {
                    let start = self.at - 1;
                    let length = utf8_width(byte);
                    let end = start
                        .checked_add(length)
                        .ok_or_else(|| self.syntax("invalid UTF-8 in JSON string"))?;
                    let text = std::str::from_utf8(
                        self.input
                            .get(start..end)
                            .ok_or_else(|| self.syntax("incomplete UTF-8 in JSON string"))?,
                    )
                    .expect("complete parser input was validated as UTF-8");
                    let scalar = text.chars().next().expect("non-empty UTF-8 suffix");
                    self.spend(length - 1)?;
                    self.at += length - 1;
                    output.push(scalar);
                }
            }
        }
    }
    fn escape(&mut self, output: &mut String) -> Result<(), JsonError> {
        let escape = *self
            .input
            .get(self.at)
            .ok_or_else(|| self.syntax("incomplete JSON string escape"))?;
        self.spend(1)?;
        self.at += 1;
        match escape {
            b'"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'/' => output.push('/'),
            b'b' => output.push('\u{0008}'),
            b'f' => output.push('\u{000c}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' => {
                let first = self.hex4()?;
                let scalar = if (0xd800..=0xdbff).contains(&first) {
                    if self.input.get(self.at..self.at + 2) != Some(b"\\u") {
                        return Err(self.syntax("unpaired high surrogate in JSON string"));
                    }
                    self.spend(2)?;
                    self.at += 2;
                    let second = self.hex4()?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err(self.syntax("invalid UTF-16 surrogate pair in JSON string"));
                    }
                    0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
                } else if (0xdc00..=0xdfff).contains(&first) {
                    return Err(self.syntax("unpaired low surrogate in JSON string"));
                } else {
                    u32::from(first)
                };
                output.push(char::from_u32(scalar).expect("validated Unicode scalar"));
            }
            _ => return Err(self.syntax("invalid JSON string escape")),
        }
        Ok(())
    }
    fn hex4(&mut self) -> Result<u16, JsonError> {
        self.spend(4)?;
        let mut value = 0_u16;
        for _ in 0..4 {
            let byte = *self
                .input
                .get(self.at)
                .ok_or_else(|| self.syntax("incomplete Unicode escape"))?;
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(self.syntax("invalid Unicode escape")),
            };
            value = value * 16 + u16::from(digit);
            self.at += 1;
        }
        Ok(value)
    }
    fn number(&mut self) -> Result<JsonValue, JsonError> {
        let start = self.at;
        if self.input.get(self.at) == Some(&b'-') {
            self.advance_number()?;
        }
        match self.input.get(self.at) {
            Some(b'0') => self.advance_number()?,
            Some(b'1'..=b'9') => {
                self.advance_number()?;
                while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.advance_number()?;
                }
            }
            _ => return Err(self.syntax("invalid JSON number")),
        }
        if self.input.get(self.at) == Some(&b'.') {
            self.advance_number()?;
            let digits = self.at;
            while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.advance_number()?;
            }
            if self.at == digits {
                return Err(self.syntax("invalid JSON number fraction"));
            }
        }
        if matches!(self.input.get(self.at), Some(b'e' | b'E')) {
            self.advance_number()?;
            if matches!(self.input.get(self.at), Some(b'+' | b'-')) {
                self.advance_number()?;
            }
            let digits = self.at;
            while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.advance_number()?;
            }
            if self.at == digits {
                return Err(self.syntax("invalid JSON number exponent"));
            }
        }
        // `JsonNumber` independently validates into temporary digit storage,
        // then owns the original token. Admit both passes/allocations before
        // invoking support code; the initial lexical scan was charged above.
        self.spend(
            (self.at - start)
                .checked_mul(2)
                .ok_or_else(|| JsonError::limit(Some(start), "JSON work limit exceeded"))?,
        )?;
        let token = std::str::from_utf8(&self.input[start..self.at]).expect("ASCII number token");
        let number = JsonNumber::from_str(token)
            .map_err(|_| JsonError::at(JsonErrorKind::Syntax, start, "invalid JSON number"))?;
        Ok(Nullable::Value(JsonNonNullValue::Number(number)))
    }
    fn advance_number(&mut self) -> Result<(), JsonError> {
        self.spend(1)?;
        self.at += 1;
        Ok(())
    }
}

fn utf8_width(first: u8) -> usize {
    match first {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 1, // unreachable after whole-input UTF-8 validation
    }
}

struct Writer {
    output: String,
    work: usize,
    limits: JsonLimits,
}
impl Writer {
    fn spend(&mut self, amount: usize) -> Result<(), JsonError> {
        self.work = self
            .work
            .checked_sub(amount)
            .ok_or_else(|| JsonError::limit(None, "JSON work limit exceeded"))?;
        Ok(())
    }
    fn push(&mut self, text: &str) -> Result<(), JsonError> {
        let length = self
            .output
            .len()
            .checked_add(text.len())
            .ok_or_else(|| JsonError::limit(None, "JSON output exceeds its byte limit"))?;
        if length > self.limits.max_output_bytes {
            return Err(JsonError::limit(None, "JSON output exceeds its byte limit"));
        }
        self.spend(text.len())?;
        self.output.push_str(text);
        Ok(())
    }
    fn value(&mut self, value: &JsonValue, depth: usize) -> Result<(), JsonError> {
        self.spend(1)?;
        match value {
            Nullable::Null => self.push("null"),
            Nullable::Value(JsonNonNullValue::Bool(value)) => {
                self.push(if *value { "true" } else { "false" })
            }
            Nullable::Value(JsonNonNullValue::String(value)) => self.string(value),
            Nullable::Value(JsonNonNullValue::Number(value)) => self.push(value.as_str()),
            Nullable::Value(JsonNonNullValue::Array(values)) => {
                let depth = self.enter(depth)?;
                self.push("[")?;
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        self.push(",")?;
                    }
                    self.value(value, depth)?;
                }
                self.push("]")
            }
            Nullable::Value(JsonNonNullValue::Object(values)) => {
                let depth = self.enter(depth)?;
                self.push("{")?;
                for (index, (key, value)) in values.iter().enumerate() {
                    if index != 0 {
                        self.push(",")?;
                    }
                    self.string(key)?;
                    self.push(":")?;
                    self.value(value, depth)?;
                }
                self.push("}")
            }
        }
    }
    fn enter(&self, depth: usize) -> Result<usize, JsonError> {
        let next = depth
            .checked_add(1)
            .ok_or_else(|| JsonError::limit(None, "JSON nesting limit exceeded"))?;
        if next > self.limits.max_depth {
            Err(JsonError::limit(None, "JSON nesting limit exceeded"))
        } else {
            Ok(next)
        }
    }
    fn string(&mut self, value: &str) -> Result<(), JsonError> {
        self.push("\"")?;
        for scalar in value.chars() {
            match scalar {
                '"' => self.push("\\\"")?,
                '\\' => self.push("\\\\")?,
                '\u{0008}' => self.push("\\b")?,
                '\u{000c}' => self.push("\\f")?,
                '\n' => self.push("\\n")?,
                '\r' => self.push("\\r")?,
                '\t' => self.push("\\t")?,
                '\u{0000}'..='\u{001f}' => {
                    let mut escape = String::with_capacity(6);
                    write!(&mut escape, "\\u{:04x}", u32::from(scalar)).expect("writing to String");
                    self.push(&escape)?;
                }
                _ => {
                    let mut bytes = [0; 4];
                    self.push(scalar.encode_utf8(&mut bytes))?;
                }
            }
        }
        self.push("\"")
    }
}
