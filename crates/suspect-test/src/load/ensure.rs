//! SLO checks (`ensure:`) evaluated over the cumulative run metrics.
//!
//! Three forms, mirroring Artillery's `ensure` plugin:
//!
//! - `maxErrorRate: <pct>` — transport + criteria failures over requests.
//! - `thresholds: [{<metric>: <max>}]` — the metric must not exceed `max`.
//! - `conditions: [{expression, strict}]` — a boolean expression.
//!
//! Expression grammar (no arithmetic; metric names are dotted identifiers):
//!
//! ```text
//! expr    = or
//! or      = and ( "or" and )*
//! and     = unary ( "and" unary )*
//! unary   = "not" unary | cmp
//! cmp     = operand ( ( "<" | "<=" | ">" | ">=" | "==" | "!=" ) operand )?
//! operand = number | metric | "(" expr ")"
//! metric  = [A-Za-z_] [A-Za-z0-9_.:-]*
//! ```
//!
//! A bare operand is truthy when non-zero.

use serde::{Deserialize, Serialize};

use super::metrics::MetricsSnapshot;
use super::profile::Ensure;

/// Outcome of one check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    /// Short name (`maxErrorRate`, the metric, or the expression).
    pub name: String,
    /// Human-readable statement of what was required.
    pub requirement: String,
    /// Whether it held.
    pub passed: bool,
    /// Whether failing it fails the run.
    pub strict: bool,
    /// Observed value for single-metric checks.
    pub actual: Option<f64>,
    /// Explanation when the check could not be evaluated.
    pub error: Option<String>,
}

/// All check outcomes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EnsureReport {
    /// Checks in declaration order.
    pub checks: Vec<CheckResult>,
}

impl EnsureReport {
    /// True when no strict check failed (non-strict failures only warn).
    #[must_use]
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.passed || !c.strict)
    }

    /// Number of failed checks, strict or not.
    #[must_use]
    pub fn failed(&self) -> usize {
        self.checks.iter().filter(|c| !c.passed).count()
    }
}

/// Evaluates every configured check against the cumulative snapshot.
#[must_use]
pub fn evaluate(ensure: &Ensure, snapshot: &MetricsSnapshot) -> EnsureReport {
    let mut checks = Vec::new();
    if let Some(max) = ensure.max_error_rate {
        let actual = snapshot.http.error_rate;
        checks.push(CheckResult {
            name: "maxErrorRate".to_owned(),
            requirement: format!("error rate <= {max}%"),
            passed: actual <= max,
            strict: true,
            actual: Some(actual),
            error: None,
        });
    }
    for (metric, max, _) in &ensure.thresholds {
        match snapshot.get(metric) {
            Some(actual) => checks.push(CheckResult {
                name: metric.clone(),
                requirement: format!("{metric} <= {max}"),
                passed: actual <= *max,
                strict: true,
                actual: Some(actual),
                error: None,
            }),
            None => checks.push(CheckResult {
                name: metric.clone(),
                requirement: format!("{metric} <= {max}"),
                passed: false,
                strict: true,
                actual: None,
                error: Some(format!("unknown metric `{metric}`")),
            }),
        }
    }
    for cond in &ensure.conditions {
        let (passed, error) = match parse_expression(&cond.expression) {
            Ok(expr) => match eval(&expr, snapshot) {
                Ok(v) => (v != 0.0, None),
                Err(e) => (false, Some(e)),
            },
            Err(e) => (false, Some(e)),
        };
        checks.push(CheckResult {
            name: cond.expression.clone(),
            requirement: cond.expression.clone(),
            passed,
            strict: cond.strict,
            actual: None,
            error,
        });
    }
    EnsureReport { checks }
}

/// Parsed expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Numeric literal.
    Number(f64),
    /// Metric reference.
    Metric(String),
    /// Comparison.
    Cmp(Box<Expr>, CmpOp, Box<Expr>),
    /// Logical and.
    And(Box<Expr>, Box<Expr>),
    /// Logical or.
    Or(Box<Expr>, Box<Expr>),
    /// Logical not.
    Not(Box<Expr>),
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `==`
    Eq,
    /// `!=`
    Ne,
}

/// Parses a condition expression.
///
/// # Errors
/// Returns a message with the byte offset of the first problem.
pub fn parse_expression(text: &str) -> Result<Expr, String> {
    let tokens = lex(text)?;
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.or()?;
    if p.pos < p.tokens.len() {
        return Err(format!(
            "unexpected `{}` at offset {}",
            p.tokens[p.pos].text, p.tokens[p.pos].offset
        ));
    }
    Ok(expr)
}

/// Metric names referenced by an expression (for validation).
#[must_use]
pub fn metrics_in(expr: &Expr) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(e: &Expr, out: &mut Vec<String>) {
        match e {
            Expr::Number(_) => {}
            Expr::Metric(m) => out.push(m.clone()),
            Expr::Cmp(a, _, b) | Expr::And(a, b) | Expr::Or(a, b) => {
                walk(a, out);
                walk(b, out);
            }
            Expr::Not(a) => walk(a, out),
        }
    }
    walk(expr, &mut out);
    out
}

/// Evaluates an expression; booleans are `1.0` / `0.0`.
///
/// # Errors
/// Returns the first unknown metric name.
pub fn eval(expr: &Expr, snapshot: &MetricsSnapshot) -> Result<f64, String> {
    Ok(match expr {
        Expr::Number(n) => *n,
        Expr::Metric(m) => snapshot
            .get(m)
            .ok_or_else(|| format!("unknown metric `{m}`"))?,
        Expr::Cmp(a, op, b) => {
            let (a, b) = (eval(a, snapshot)?, eval(b, snapshot)?);
            let holds = match op {
                CmpOp::Lt => a < b,
                CmpOp::Le => a <= b,
                CmpOp::Gt => a > b,
                CmpOp::Ge => a >= b,
                CmpOp::Eq => (a - b).abs() < f64::EPSILON,
                CmpOp::Ne => (a - b).abs() >= f64::EPSILON,
            };
            f64::from(u8::from(holds))
        }
        Expr::And(a, b) => f64::from(u8::from(
            eval(a, snapshot)? != 0.0 && eval(b, snapshot)? != 0.0,
        )),
        Expr::Or(a, b) => f64::from(u8::from(
            eval(a, snapshot)? != 0.0 || eval(b, snapshot)? != 0.0,
        )),
        Expr::Not(a) => f64::from(u8::from(eval(a, snapshot)? == 0.0)),
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Number(f64),
    Ident(String),
    Op(CmpOp),
    And,
    Or,
    Not,
    LParen,
    RParen,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    text: String,
    offset: usize,
}

fn lex(text: &str) -> Result<Vec<Token>, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let tok = match c {
            b'(' => {
                i += 1;
                Tok::LParen
            }
            b')' => {
                i += 1;
                Tok::RParen
            }
            b'<' | b'>' | b'=' | b'!' => {
                let two = bytes.get(i + 1) == Some(&b'=');
                i += if two { 2 } else { 1 };
                match (c, two) {
                    (b'<', false) => Tok::Op(CmpOp::Lt),
                    (b'<', true) => Tok::Op(CmpOp::Le),
                    (b'>', false) => Tok::Op(CmpOp::Gt),
                    (b'>', true) => Tok::Op(CmpOp::Ge),
                    (b'=', true) => Tok::Op(CmpOp::Eq),
                    (b'!', true) => Tok::Op(CmpOp::Ne),
                    _ => return Err(format!("unexpected `{}` at offset {start}", c as char)),
                }
            }
            b'0'..=b'9' => {
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                let s = &text[start..i];
                Tok::Number(
                    s.parse()
                        .map_err(|_| format!("invalid number `{s}` at offset {start}"))?,
                )
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || matches!(bytes[i], b'_' | b'.' | b':' | b'-'))
                {
                    i += 1;
                }
                match &text[start..i] {
                    "and" | "AND" => Tok::And,
                    "or" | "OR" => Tok::Or,
                    "not" | "NOT" => Tok::Not,
                    ident => Tok::Ident(ident.to_owned()),
                }
            }
            other => {
                return Err(format!("unexpected `{}` at offset {start}", other as char));
            }
        };
        out.push(Token {
            tok,
            text: text[start..i].to_owned(),
            offset: start,
        });
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|t| &t.tok)
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.peek() == Some(&Tok::Or) {
            self.bump();
            let right = self.and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        while self.peek() == Some(&Tok::And) {
            self.bump();
            let right = self.unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.peek() == Some(&Tok::Not) {
            self.bump();
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.cmp()
    }

    fn cmp(&mut self) -> Result<Expr, String> {
        let left = self.operand()?;
        if let Some(Tok::Op(op)) = self.peek().cloned() {
            self.bump();
            let right = self.operand()?;
            return Ok(Expr::Cmp(Box::new(left), op, Box::new(right)));
        }
        Ok(left)
    }

    fn operand(&mut self) -> Result<Expr, String> {
        match self.bump() {
            Some(Token {
                tok: Tok::Number(n),
                ..
            }) => Ok(Expr::Number(n)),
            Some(Token {
                tok: Tok::Ident(m), ..
            }) => Ok(Expr::Metric(m)),
            Some(Token {
                tok: Tok::LParen, ..
            }) => {
                let inner = self.or()?;
                match self.bump() {
                    Some(Token {
                        tok: Tok::RParen, ..
                    }) => Ok(inner),
                    Some(t) => Err(format!(
                        "expected `)` at offset {}, found `{}`",
                        t.offset, t.text
                    )),
                    None => Err("expected `)` at end of expression".to_owned()),
                }
            }
            Some(t) => Err(format!(
                "expected a metric or number at offset {}, found `{}`",
                t.offset, t.text
            )),
            None => Err("unexpected end of expression".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::metrics::Quantiles;

    fn snap() -> MetricsSnapshot {
        let mut s = MetricsSnapshot::default();
        s.http.requests = 1000;
        s.http.request_rate = 120.0;
        s.http.error_rate = 0.5;
        s.http.response_time = Quantiles {
            count: 1000,
            p95: 180.0,
            p99: 420.0,
            ..Quantiles::default()
        };
        s.vusers.failed = 0;
        s.http.codes.insert(503, 2);
        s
    }

    #[test]
    fn grammar_precedence_and_eval() {
        let s = snap();
        let truthy = |e: &str| eval(&parse_expression(e).unwrap(), &s).unwrap() != 0.0;
        assert!(truthy("http.response_time.p99 < 500"));
        assert!(!truthy("http.response_time.p99 < 400"));
        assert!(truthy("http.request_rate > 100 and vusers.failed == 0"));
        assert!(truthy("http.request_rate > 1000 or vusers.failed == 0"));
        // `and` binds tighter than `or`.
        assert!(truthy(
            "http.request_rate > 1000 or vusers.failed == 0 and http.codes.503 <= 2"
        ));
        assert!(!truthy(
            "(http.request_rate > 1000 or vusers.failed == 0) and http.codes.503 < 2"
        ));
        assert!(truthy("not http.codes.504"));
        assert!(truthy("not (http.response_time.p95 >= 200)"));
        assert!(truthy("http.codes.503 != 0"));
    }

    #[test]
    fn errors_name_offsets_and_unknown_metrics() {
        assert!(
            parse_expression("http.x <")
                .unwrap_err()
                .contains("end of expression")
        );
        assert!(parse_expression("1 2").unwrap_err().contains("offset 2"));
        assert!(parse_expression("(a < 1").unwrap_err().contains("`)`"));
        assert!(parse_expression("a $ b").unwrap_err().contains("offset 2"));
        let e = parse_expression("made.up.metric > 1").unwrap();
        assert_eq!(metrics_in(&e), vec!["made.up.metric"]);
        assert!(eval(&e, &snap()).unwrap_err().contains("unknown metric"));
    }

    #[test]
    fn evaluate_collects_all_three_forms() {
        let ensure = Ensure {
            max_error_rate: Some(1.0),
            thresholds: vec![
                ("http.response_time.p95".into(), 250.0, 0..0),
                ("http.response_time.p99".into(), 400.0, 0..0),
                ("bogus.metric".into(), 1.0, 0..0),
            ],
            conditions: vec![
                crate::load::profile::Condition {
                    expression: "http.request_rate > 90".into(),
                    strict: true,
                    range: 0..0,
                },
                crate::load::profile::Condition {
                    expression: "http.codes.503 == 0".into(),
                    strict: false,
                    range: 0..0,
                },
            ],
        };
        let report = evaluate(&ensure, &snap());
        let by_name: std::collections::BTreeMap<&str, &CheckResult> =
            report.checks.iter().map(|c| (c.name.as_str(), c)).collect();
        assert!(by_name["maxErrorRate"].passed);
        assert!(by_name["http.response_time.p95"].passed);
        assert!(!by_name["http.response_time.p99"].passed);
        assert!(!by_name["bogus.metric"].passed);
        assert!(by_name["bogus.metric"].error.is_some());
        assert!(by_name["http.request_rate > 90"].passed);
        assert!(!by_name["http.codes.503 == 0"].passed);
        assert!(!by_name["http.codes.503 == 0"].strict);
        assert_eq!(report.failed(), 3);
        assert!(!report.ok(), "strict p99 and bogus failures fail the run");
    }
}
