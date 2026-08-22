//! Arazzo 1.0 runtime expressions: full grammar parser + embedded forms.
//!
//! Grammar (Arazzo 1.0 §4.3):
//! - `$method`, `$url`, `$statusCode`
//! - `$request.<part>`, `$response.<part>` where part is
//!   `header.#`, `query.#`, `path.#` (token) or `body` optionally followed
//!   by a JSON-pointer fragment (`#/a/b`)
//! - `$outputs.<name>`, `$inputs.<name>`
//! - `$workflows.<wf>.steps.<step>.outputs.<name>`
//! - `$components.parameters.<name>`, `$components.succeedOn.<name>`,
//!   `$components.failureOn.<name>`, `$components.retryOn.<name>`
//! - Embedded expressions inside strings: `text {$expr} more text`

use std::fmt;

use suspect_low::Pointer;

/// One HTTP-message part reference.
#[derive(Debug, Clone, PartialEq)]
pub enum HttpPart {
    /// `header.#` — a header value; the string is the header name (token).
    Header(String),
    /// `query.#` — a query parameter value; the string is the parameter name.
    Query(String),
    /// `path.#` — a path-template parameter value; the string is the name.
    Path(String),
    /// Body, with an optional JSON Pointer into the payload.
    Body(Option<Pointer>),
}

/// A parsed runtime expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `$method` — HTTP method of the current request.
    Method,
    /// `$url` — fully-resolved URL of the current request.
    Url,
    /// `$statusCode` — response status code of the current step.
    StatusCode,
    /// `$request.<part>` — a part of the current request message.
    Request {
        /// Which part of the message is referenced.
        part: HttpPart,
    },
    /// `$response.<part>` — a part of the current response message.
    Response {
        /// Which part of the message is referenced.
        part: HttpPart,
    },
    /// `$outputs.<name>` — a named output of the current workflow.
    Outputs {
        /// The output name after the dot.
        name: String,
    },
    /// `$inputs.<name>` — a named input of the current workflow.
    Inputs {
        /// The input name after the dot.
        name: String,
    },
    /// `$workflows.<wf>.steps.<step>.outputs.<name>` — an output produced by
    /// another workflow's step.
    WorkflowOutput {
        /// The `workflowId` segment.
        workflow: String,
        /// The `stepId` segment.
        step: String,
        /// The output name segment.
        name: String,
    },
    /// `$components.<kind>.<name>` reusable-object reference.
    Component {
        /// Which component collection to look in.
        kind: ComponentKind,
        /// The component name after the dot.
        name: String,
    },
    /// `$sourceDescriptions.<name>[.<path>]` — names an entry plus an
    /// optional operation path template.
    SourceDescription {
        /// The `sourceDescriptions` entry name.
        name: String,
        /// Remaining path text after the name (e.g. `#/paths/~1pets` or an
        /// operation-path template), kept verbatim.
        path: String,
    },
    /// Literal text (only produced by [`parse_embedded`] for non-expression
    /// spans).
    Text(String),
}

/// Which reusable-component kind a `$components` expression references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    /// `parameters` — reusable parameter objects.
    Parameters,
    /// `succeedOn` — reusable success-criterion collections.
    SucceedOn,
    /// `failureOn` — reusable failure-criterion collections.
    FailureOn,
    /// `retryOn` — reusable retry-criterion collections.
    RetryOn,
}
/// A piece of an embedded-expression string.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprPart {
    /// Literal text between expression spans.
    Text(String),
    /// An embedded `{...}` runtime expression.
    Expr(Expr),
}

/// Expression parse failure.
#[derive(Debug, Clone, PartialEq)]
pub struct ExprError {
    /// The full offending input string.
    pub input: String,
    /// Byte offset into [`Self::input`] where parsing failed.
    pub offset: usize,
    /// Human-readable explanation of the failure.
    pub reason: String,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid runtime expression {:?} at offset {}: {}",
            self.input, self.offset, self.reason
        )
    }
}

impl std::error::Error for ExprError {}

/// Parses a standalone runtime expression (the whole string must be one).
///
/// # Errors
/// [`ExprError`] when the string is not a well-formed expression.
pub fn parse(input: &str) -> Result<Expr, ExprError> {
    let mut p = Parser {
        input: input.as_bytes(),
        pos: 0,
    };
    let expr = p.expr().map_err(|reason| ExprError {
        input: input.to_owned(),
        offset: p.pos,
        reason: reason.to_owned(),
    })?;
    if p.pos != p.input.len() {
        return Err(ExprError {
            input: input.to_owned(),
            offset: p.pos,
            reason: "trailing characters after expression".into(),
        });
    }
    Ok(expr)
}

/// Splits a string into literal text and embedded expressions.
#[must_use]
pub fn parse_embedded(input: &str) -> Vec<ExprPart> {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{'
            && bytes[i + 1] == b'$'
            && let Some(close) = input[i..].find('}')
        {
            let close = i + close;
            if start < i {
                out.push(ExprPart::Text(input[start..i].to_owned()));
            }
            match parse(&input[i + 1..close]) {
                Ok(e) => out.push(ExprPart::Expr(e)),
                Err(_) => {
                    // not a valid expression: treat as literal text
                    if let Some(last_text) = out.last_mut() {
                        if let ExprPart::Text(t) = last_text {
                            t.push_str(&input[i..=close]);
                        }
                    } else {
                        out.push(ExprPart::Text(input[start..=close].to_owned()));
                    }
                }
            }
            i = close + 1;
            start = i;
            continue;
        }
        i += 1;
    }
    if start < input.len() {
        out.push(ExprPart::Text(input[start..].to_owned()));
    }
    out
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn eat(&mut self, b: u8) -> Result<(), &'static str> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err("unexpected character")
        }
    }

    fn ident(&mut self) -> Result<String, &'static str> {
        // root keywords only: no dots/dashes (those belong to later segments)
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err("expected identifier");
        }
        Ok(String::from_utf8_lossy(&self.input[start..self.pos]).to_string())
    }

    fn expr(&mut self) -> Result<Expr, &'static str> {
        self.eat(b'$')?;
        let head = self.ident()?;
        match head.as_str() {
            "method" => return Ok(Expr::Method),
            "url" => return Ok(Expr::Url),
            "statusCode" => return Ok(Expr::StatusCode),
            "outputs" => {
                let name = self.dotted_name()?;
                return Ok(Expr::Outputs { name });
            }
            "inputs" => {
                let name = self.dotted_name()?;
                return Ok(Expr::Inputs { name });
            }
            _ => {}
        }
        if head == "request" || head == "response" {
            self.eat(b'.')?;
            let part = self.http_part()?;
            return Ok(if head == "request" {
                Expr::Request { part }
            } else {
                Expr::Response { part }
            });
        }
        if head == "sourceDescriptions" {
            self.eat(b'.')?;
            let name = self.segment()?;
            // optional path template tail: ./pets/{id} or /pets — kept verbatim
            let mut path = String::new();
            while self.peek().is_some() {
                path.push(self.peek().unwrap() as char);
                self.pos += 1;
            }
            return Ok(Expr::SourceDescription { name, path });
        }
        if head == "workflows" {
            self.eat(b'.')?;
            let workflow = self.segment()?;
            expect_str(self, ".steps.")?;
            let step = self.segment()?;
            expect_str(self, ".outputs.")?;
            let name = self.segment()?;
            return Ok(Expr::WorkflowOutput {
                workflow,
                step,
                name,
            });
        }
        if head == "components" {
            self.eat(b'.')?;
            let kind = self.ident()?;
            let parsed = match kind.as_str() {
                "parameters" => ComponentKind::Parameters,
                "succeedOn" => ComponentKind::SucceedOn,
                "failureOn" => ComponentKind::FailureOn,
                "retryOn" => ComponentKind::RetryOn,
                _ => return Err("unknown components kind"),
            };
            self.eat(b'.')?;
            let name = self.segment()?;
            return Ok(Expr::Component { kind: parsed, name });
        }
        Err("unknown expression root")
    }

    fn dotted_name(&mut self) -> Result<String, &'static str> {
        self.eat(b'.')?;
        self.segment()
    }

    fn segment(&mut self) -> Result<String, &'static str> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err("expected name segment");
        }
        Ok(String::from_utf8_lossy(&self.input[start..self.pos]).to_string())
    }

    fn http_part(&mut self) -> Result<HttpPart, &'static str> {
        let kind = self.ident()?;
        match kind.as_str() {
            "header" | "query" | "path" => {
                self.eat(b'.')?;
                self.eat(b'#')?;
                let name = self.segment()?;
                Ok(match kind.as_str() {
                    "header" => HttpPart::Header(name),
                    "query" => HttpPart::Query(name),
                    _ => HttpPart::Path(name),
                })
            }
            "body" => {
                if self.peek() == Some(b'#') {
                    self.pos += 1; // '#'
                    let frag_start = self.pos;
                    while self.peek().is_some() {
                        self.pos += 1;
                    }
                    let frag = String::from_utf8_lossy(&self.input[frag_start..]).to_string();
                    let pointer = Pointer::parse(&frag)
                        .map_err(|_| "invalid JSON pointer in body fragment")?;
                    Ok(HttpPart::Body(Some(pointer)))
                } else {
                    Ok(HttpPart::Body(None))
                }
            }
            _ => Err("expected header/query/path/body"),
        }
    }
}

fn expect_str(p: &mut Parser<'_>, s: &str) -> Result<(), &'static str> {
    if p.input[p.pos..].starts_with(s.as_bytes()) {
        p.pos += s.len();
        Ok(())
    } else {
        Err("malformed workflows expression")
    }
}
