//! Arazzo `simple` conditions: the boolean expression language of success
//! criteria and action criteria.
//!
//! ```text
//! $statusCode == 200 || $statusCode == 201
//! $response.header.Content-Type == 'application/json' && $response.body#/total >= 1
//! !($response.body#/state == 'failed')
//! ```
//!
//! A condition combines runtime expressions and literals with the
//! operators the Arazzo Criterion Object defines: `==`, `!=`, `<`, `<=`,
//! `>`, `>=`, `!`, `&&`, `||` and parentheses. Literals are `true`,
//! `false`, `null`, numbers and single-quoted strings (`''` is a quote);
//! double-quoted and bare words are accepted as strings too. String
//! comparison is case-insensitive, as the specification requires.
//!
//! Operands are evaluated against one finished exchange through
//! [`RexCtx`], so a condition reads the same values step outputs capture:
//! `$statusCode`, `$method`, `$response.header.<name>`,
//! `$response.body#/<pointer>`, `$request…`, `$inputs.<name>`,
//! `$steps.<id>.outputs.<name>` and `$workflows.<id>.outputs.<name>`. An
//! expression may be written `{$statusCode}`; the braces are optional.

use serde_json::Value;
use suspect_arazzo::rex::{Rex, RexCtx, eval_rex, parse_rex};

/// A parsed condition.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    /// Any member holds.
    Or(Vec<Cond>),
    /// Every member holds.
    And(Vec<Cond>),
    /// The member does not hold.
    Not(Box<Cond>),
    /// A comparison of two operands.
    Compare {
        /// Left operand.
        left: Operand,
        /// The comparison.
        op: CompareOp,
        /// Right operand.
        right: Operand,
    },
    /// A lone operand: holds when it resolves to anything but `null`,
    /// `false` or nothing.
    Truthy(Operand),
}

/// One side of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// A literal from the condition text.
    Literal(Value),
    /// A runtime expression with its source text, for failure messages.
    Expr {
        /// The parsed expression.
        rex: Rex,
        /// The expression as written (without optional braces).
        text: String,
    },
}

/// The comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}

/// Whether `text` uses an operator only the full grammar understands: a
/// logical operator, grouping, or negation outside quotes and pointers.
#[must_use]
pub fn is_compound(text: &str) -> bool {
    tokenize(text).is_ok_and(|tokens| {
        tokens.iter().any(|token| {
            matches!(
                token,
                Token::And | Token::Or | Token::Not | Token::Open | Token::Close
            )
        })
    })
}

/// Parses a condition.
///
/// # Errors
/// A message naming what was found where an operand, operator or closing
/// parenthesis was expected, or the runtime expression that does not
/// parse.
pub fn parse(text: &str) -> Result<Cond, String> {
    let tokens = tokenize(text)?;
    let mut parser = Parser {
        tokens: &tokens,
        position: 0,
    };
    let cond = parser.or()?;
    match parser.peek() {
        None => Ok(cond),
        Some(token) => Err(format!(
            "unexpected {} after the condition",
            token.describe()
        )),
    }
}

impl Cond {
    /// Whether the condition holds for the exchange `ctx` describes.
    #[must_use]
    pub fn holds(&self, ctx: &RexCtx<'_>) -> bool {
        match self {
            Self::Or(members) => members.iter().any(|member| member.holds(ctx)),
            Self::And(members) => members.iter().all(|member| member.holds(ctx)),
            Self::Not(member) => !member.holds(ctx),
            Self::Compare { left, op, right } => {
                compare(left.value(ctx).as_ref(), *op, right.value(ctx).as_ref())
            }
            Self::Truthy(operand) => !matches!(
                operand.value(ctx),
                None | Some(Value::Null | Value::Bool(false))
            ),
        }
    }

    /// The value of every runtime expression the condition reads, as
    /// `$expr = value` pairs in first-use order: what a failure message
    /// shows beside the condition that did not hold.
    #[must_use]
    pub fn observed(&self, ctx: &RexCtx<'_>) -> String {
        let mut seen: Vec<String> = Vec::new();
        self.collect(ctx, &mut seen);
        if seen.is_empty() {
            "the condition is false".to_owned()
        } else {
            seen.join(", ")
        }
    }

    fn collect(&self, ctx: &RexCtx<'_>, seen: &mut Vec<String>) {
        let mut note = |operand: &Operand| {
            if let Operand::Expr { text, .. } = operand {
                let rendered = operand
                    .value(ctx)
                    .map_or_else(|| "<missing>".to_owned(), |value| value.to_string());
                let line = format!("{text} = {rendered}");
                if !seen.contains(&line) {
                    seen.push(line);
                }
            }
        };
        match self {
            Self::Or(members) | Self::And(members) => {
                for member in members {
                    member.collect(ctx, seen);
                }
            }
            Self::Not(member) => member.collect(ctx, seen),
            Self::Compare { left, right, .. } => {
                note(left);
                note(right);
            }
            Self::Truthy(operand) => note(operand),
        }
    }
}

impl Operand {
    /// The operand's value; `None` when an expression resolves to nothing.
    fn value(&self, ctx: &RexCtx<'_>) -> Option<Value> {
        match self {
            Self::Literal(value) => Some(value.clone()),
            Self::Expr { rex, .. } => eval_rex(rex, ctx),
        }
    }
}

/// A number, when the value is one or is a string spelling one. Header
/// values and path parameters arrive as text; `$response.header.X-Total
/// > 5` compares the number the text spells.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Applies a comparison. A missing operand equals only `null`; ordered
/// comparisons with a missing or unorderable operand do not hold.
fn compare(left: Option<&Value>, op: CompareOp, right: Option<&Value>) -> bool {
    match op {
        CompareOp::Eq => equal(left, right),
        CompareOp::Ne => !equal(left, right),
        CompareOp::Lt | CompareOp::Le | CompareOp::Gt | CompareOp::Ge => {
            let (Some(left), Some(right)) = (left, right) else {
                return false;
            };
            let ordering = match (left, right) {
                (Value::String(a), Value::String(b))
                    if number(left).is_none() || number(right).is_none() =>
                {
                    Some(a.to_lowercase().cmp(&b.to_lowercase()))
                }
                _ => number(left)
                    .zip(number(right))
                    .and_then(|(a, b)| a.partial_cmp(&b)),
            };
            ordering.is_some_and(|ordering| match op {
                CompareOp::Lt => ordering.is_lt(),
                CompareOp::Le => ordering.is_le(),
                CompareOp::Gt => ordering.is_gt(),
                _ => ordering.is_ge(),
            })
        }
    }
}

/// Equality of two present values, as conditions define it: strings
/// compare case-insensitively and a number equals the text that spells it.
#[must_use]
pub fn values_equal(left: &Value, right: &Value) -> bool {
    equal(Some(left), Some(right))
}

/// Equality: strings compare case-insensitively, a number equals the text
/// that spells it, and nothing equals `null`.
fn equal(left: Option<&Value>, right: Option<&Value>) -> bool {
    let null = Value::Null;
    let (left, right) = (left.unwrap_or(&null), right.unwrap_or(&null));
    match (left, right) {
        (Value::String(a), Value::String(b)) => a.to_lowercase() == b.to_lowercase(),
        (Value::Number(_), Value::String(_)) | (Value::String(_), Value::Number(_)) => {
            number(left).zip(number(right)).is_some_and(|(a, b)| a == b)
        }
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (a, b) => a == b,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Open,
    Close,
    Not,
    And,
    Or,
    Compare(CompareOp),
    Operand(Operand),
}

impl Token {
    fn describe(&self) -> String {
        match self {
            Self::Open => "`(`".to_owned(),
            Self::Close => "`)`".to_owned(),
            Self::Not => "`!`".to_owned(),
            Self::And => "`&&`".to_owned(),
            Self::Or => "`||`".to_owned(),
            Self::Compare(_) => "a comparison operator".to_owned(),
            Self::Operand(Operand::Literal(value)) => format!("`{value}`"),
            Self::Operand(Operand::Expr { text, .. }) => format!("`{text}`"),
        }
    }
}

/// The operator spelled at the start of `rest`, with its length.
fn operator_at(rest: &str) -> Option<(Token, usize)> {
    [
        ("&&", Token::And),
        ("||", Token::Or),
        ("==", Token::Compare(CompareOp::Eq)),
        ("!=", Token::Compare(CompareOp::Ne)),
        ("<=", Token::Compare(CompareOp::Le)),
        (">=", Token::Compare(CompareOp::Ge)),
        ("<", Token::Compare(CompareOp::Lt)),
        (">", Token::Compare(CompareOp::Gt)),
        ("!", Token::Not),
        ("(", Token::Open),
        (")", Token::Close),
    ]
    .into_iter()
    .find(|(spelling, _)| rest.starts_with(spelling))
    .map(|(spelling, token)| (token, spelling.len()))
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        if let Some((token, length)) = operator_at(rest) {
            tokens.push(token);
            rest = rest[length..].trim_start();
            continue;
        }
        let first = rest.chars().next().unwrap_or_default();
        if first == '\'' || first == '"' {
            let (value, length) = quoted(rest, first)?;
            tokens.push(Token::Operand(Operand::Literal(Value::String(value))));
            rest = rest[length..].trim_start();
            continue;
        }
        // `{$expr}`: the braces delimit the expression.
        if let Some(inner) = rest.strip_prefix("{$") {
            let Some(end) = inner.find('}') else {
                return Err(format!("unclosed `{{` in `{rest}`"));
            };
            tokens.push(Token::Operand(expression(&format!("${}", &inner[..end]))?));
            rest = rest[2 + end + 1..].trim_start();
            continue;
        }
        let length = word_length(rest);
        let word = &rest[..length];
        tokens.push(Token::Operand(if word.starts_with('$') {
            expression(word)?
        } else {
            Operand::Literal(literal(word))
        }));
        rest = rest[length..].trim_start();
    }
    Ok(tokens)
}

/// The length of the bare word at the start of `rest`. A word ends at
/// whitespace, and — outside the pointer text after a `#` — at an operator
/// or parenthesis. Pointer text runs to the next whitespace, so
/// `$response.body#/a==b` is one expression; it keeps the parentheses it
/// opens (`#/f(x)`) and ends at one it did not, so `($response.body#/ok)`
/// is a group around an expression.
fn word_length(rest: &str) -> usize {
    let mut in_pointer = false;
    let mut open = 0usize;
    for (index, ch) in rest.char_indices() {
        if ch.is_whitespace() {
            return index;
        }
        if ch == '#' {
            in_pointer = true;
        }
        if in_pointer {
            match ch {
                '(' => open += 1,
                ')' if open == 0 => return index,
                ')' => open -= 1,
                _ => {}
            }
        } else if index > 0 && operator_at(&rest[index..]).is_some() {
            return index;
        }
    }
    rest.len()
}

/// A quoted string starting at `rest`: its value and consumed length. A
/// doubled quote is a literal quote.
fn quoted(rest: &str, quote: char) -> Result<(String, usize), String> {
    let mut value = String::new();
    let mut chars = rest.char_indices().skip(1).peekable();
    while let Some((index, ch)) = chars.next() {
        if ch != quote {
            value.push(ch);
            continue;
        }
        if chars.peek().is_some_and(|(_, next)| *next == quote) {
            value.push(quote);
            chars.next();
            continue;
        }
        return Ok((value, index + ch.len_utf8()));
    }
    Err(format!("unterminated string in `{rest}`"))
}

fn expression(text: &str) -> Result<Operand, String> {
    let rex = parse_rex(text).map_err(|error| error.to_string())?;
    Ok(Operand::Expr {
        rex,
        text: text.to_owned(),
    })
}

/// An unquoted literal: `true`, `false`, `null`, a number, else the word
/// itself as a string.
fn literal(word: &str) -> Value {
    match word {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => word
            .parse::<serde_json::Number>()
            .map_or_else(|_| Value::String(word.to_owned()), Value::Number),
    }
}

struct Parser<'t> {
    tokens: &'t [Token],
    position: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn next(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.position);
        self.position += 1;
        token
    }

    fn or(&mut self) -> Result<Cond, String> {
        let mut members = vec![self.and()?];
        while self.peek() == Some(&Token::Or) {
            self.position += 1;
            members.push(self.and()?);
        }
        Ok(single_or(members, Cond::Or))
    }

    fn and(&mut self) -> Result<Cond, String> {
        let mut members = vec![self.unary()?];
        while self.peek() == Some(&Token::And) {
            self.position += 1;
            members.push(self.unary()?);
        }
        Ok(single_or(members, Cond::And))
    }

    fn unary(&mut self) -> Result<Cond, String> {
        if self.peek() == Some(&Token::Not) {
            self.position += 1;
            return Ok(Cond::Not(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Cond, String> {
        match self.next().cloned() {
            Some(Token::Open) => {
                let inner = self.or()?;
                match self.next() {
                    Some(Token::Close) => Ok(inner),
                    Some(other) => Err(format!("expected `)`, found {}", other.describe())),
                    None => Err("expected `)` before the end of the condition".to_owned()),
                }
            }
            Some(Token::Operand(left)) => {
                let Some(Token::Compare(op)) = self.peek().cloned() else {
                    return Ok(Cond::Truthy(left));
                };
                self.position += 1;
                match self.next().cloned() {
                    Some(Token::Operand(right)) => Ok(Cond::Compare { left, op, right }),
                    Some(other) => Err(format!(
                        "expected a value after the comparison, found {}",
                        other.describe()
                    )),
                    None => Err("expected a value after the comparison".to_owned()),
                }
            }
            Some(other) => Err(format!("expected a value, found {}", other.describe())),
            None => Err("the condition is empty".to_owned()),
        }
    }
}

/// A one-member list is that member, not a group.
fn single_or(mut members: Vec<Cond>, group: fn(Vec<Cond>) -> Cond) -> Cond {
    if members.len() == 1 {
        members.remove(0)
    } else {
        group(members)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Evaluates `text` against a `201` reply with a JSON body and two
    /// headers, and inputs `{expected: "Rex", limit: 2}`.
    fn holds(text: &str) -> bool {
        let inputs: serde_json::Map<String, Value> =
            serde_json::from_str(r#"{"expected": "Rex", "limit": 2}"#).unwrap();
        let steps: serde_json::Map<String, Value> =
            serde_json::from_str(r#"{"create": {"petId": "p1"}}"#).unwrap();
        let headers = [
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("X-Total".to_owned(), "12".to_owned()),
        ];
        let ctx = RexCtx::default()
            .method("POST")
            .status(201)
            .response_headers(&headers)
            .response_body(r#"{"id": "p1", "name": "Rex", "tags": ["a", "b"], "count": 3, "ok": false, "none": null}"#)
            .inputs(&inputs)
            .steps_outputs(&steps);
        parse(text)
            .unwrap_or_else(|error| panic!("`{text}` parses: {error}"))
            .holds(&ctx)
    }

    #[test]
    fn status_comparisons() {
        assert!(holds("$statusCode == 201"));
        assert!(holds("{$statusCode} == 201"));
        assert!(!holds("$statusCode == 200"));
        assert!(holds("$statusCode != 200"));
        assert!(holds("$statusCode >= 200 && $statusCode < 300"));
        assert!(!holds("$statusCode > 201"));
        assert!(holds("$statusCode <= 201"));
    }

    #[test]
    fn logical_operators_and_grouping() {
        assert!(holds("$statusCode == 200 || $statusCode == 201"));
        assert!(!holds("$statusCode == 200 || $statusCode == 204"));
        assert!(!holds("$statusCode == 201 && $response.body#/count > 5"));
        assert!(holds("!($statusCode == 200)"));
        assert!(holds("!$response.body#/ok"));
        // `&&` binds tighter than `||`.
        assert!(holds(
            "$statusCode == 201 || $statusCode == 200 && $response.body#/count > 5"
        ));
        assert!(!holds(
            "($statusCode == 201 || $statusCode == 200) && $response.body#/count > 5"
        ));
    }

    #[test]
    fn body_values_compare_by_type() {
        assert!(holds("$response.body#/count == 3"));
        assert!(holds("$response.body#/count >= 3"));
        assert!(holds("$response.body#/name == 'Rex'"));
        assert!(holds("$response.body#/name != 'Tom'"));
        assert!(holds("$response.body#/tags/1 == 'b'"));
        assert!(holds("$response.body#/ok == false"));
    }

    #[test]
    fn string_comparison_is_case_insensitive() {
        assert!(holds("$response.body#/name == 'rex'"));
        assert!(holds("$response.header.Content-Type == 'APPLICATION/JSON'"));
        assert!(!holds("$response.body#/name != 'REX'"));
    }

    #[test]
    fn headers_are_text_that_may_spell_a_number() {
        assert!(holds("$response.header.Content-Type == 'application/json'"));
        assert!(holds("$response.header.content-type == 'application/json'"));
        assert!(holds("$response.header.X-Total > 5"));
        assert!(holds("$response.header.X-Total == 12"));
        assert!(!holds("$response.header.X-Total < 5"));
    }

    #[test]
    fn missing_and_null_values() {
        assert!(holds("$response.body#/none == null"));
        assert!(holds("$response.body#/absent == null"));
        assert!(holds("$response.body#/name != null"));
        assert!(!holds("$response.body#/absent != null"));
        assert!(!holds("$response.body#/absent > 0"));
        assert!(!holds("$response.header.X-Missing == 'x'"));
        // A lone operand holds when it resolves to something truthy.
        assert!(holds("$response.body#/name"));
        assert!(!holds("$response.body#/absent"));
        assert!(!holds("$response.body#/none"));
    }

    #[test]
    fn inputs_and_step_outputs_are_operands() {
        assert!(holds("$response.body#/name == $inputs.expected"));
        assert!(holds("$response.body#/id == $steps.create.outputs.petId"));
        assert!(holds("$response.body#/count > $inputs.limit"));
        assert!(holds("$method == 'POST'"));
    }

    #[test]
    fn string_literal_spellings() {
        assert!(holds(r#"$response.body#/name == "Rex""#));
        assert!(holds("$response.body#/name == Rex"));
        let ctx = RexCtx::default().response_body(r#"{"q": "it's"}"#);
        assert!(parse("$response.body#/q == 'it''s'").unwrap().holds(&ctx));
    }

    #[test]
    fn operators_inside_pointer_text_are_part_of_the_pointer() {
        let ctx = RexCtx::default().response_body(r#"{"a==b": 1, "x": 2}"#);
        assert!(parse("$response.body#/a==b == 1").unwrap().holds(&ctx));
        // `x==2` is pointer text here, and the object has no such member.
        assert!(!parse("$response.body#/x==2").unwrap().holds(&ctx));
    }

    #[test]
    fn failures_show_what_the_expressions_were() {
        let headers = [("X-Total".to_owned(), "12".to_owned())];
        let ctx = RexCtx::default()
            .status(404)
            .response_headers(&headers)
            .response_body("{}");
        let cond = parse("$statusCode == 200 && $response.body#/id != null").unwrap();
        assert!(!cond.holds(&ctx));
        assert_eq!(
            cond.observed(&ctx),
            "$statusCode = 404, $response.body#/id = <missing>"
        );
    }

    #[test]
    fn malformed_conditions_are_errors() {
        for text in [
            "",
            "$statusCode ==",
            "($statusCode == 200",
            "$statusCode == 200 &&",
            "$statusCode == 200 200",
            "== 200",
            "$nonsense == 1",
            "$response.body#/a == 'unterminated",
        ] {
            assert!(parse(text).is_err(), "`{text}` must not parse");
        }
    }

    #[test]
    fn compound_conditions_are_recognized() {
        assert!(is_compound("$statusCode == 200 || $statusCode == 201"));
        assert!(is_compound("!($statusCode == 200)"));
        assert!(!is_compound("$statusCode == 200"));
        assert!(!is_compound("$response.body#/a||b == 1"));
        assert!(!is_compound("$response.body#/name == 'a && b'"));
    }
}
