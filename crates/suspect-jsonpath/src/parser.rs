//! Recursive-descent, char-cursor parser for RFC 9535 JSONPath queries.

use crate::ast::{
    Comparable, Comparator, FArg, FuncName, FunctionCall, Lit, LogicalExpr, QueryAst, Segment,
    Selector, Testable,
};
use crate::PathError;

pub(crate) fn parse(input: &str) -> Result<QueryAst, PathError> {
    let mut p = Parser::new(input);
    p.ws();
    if !p.eat_byte(b'$') {
        return Err(p.err("expected '$' at start of query"));
    }
    let segments = p.parse_segments(false)?;
    p.ws();
    if p.pos != input.len() {
        return Err(p.err("trailing characters after query"));
    }
    Ok(QueryAst { absolute: true, segments })
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self { s, b: s.as_bytes(), pos: 0 }
    }

    fn err(&self, reason: &str) -> PathError {
        PathError { input: self.s.to_owned(), offset: self.pos, reason: reason.to_owned() }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.b.get(self.pos + off).copied()
    }

    fn eat_byte(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Skips spaces and tabs only (RFC 9535 WS); newlines are not legal.
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.pos += 1;
        }
    }

    fn ident_start(c: u8) -> bool {
        c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
    }

    fn ident_rest(c: u8) -> bool {
        c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c >= 0x80
    }

    // ---- segments -------------------------------------------------------

    /// Parses segments until a character that cannot continue a query
    /// (anything other than `.`, `[`, or the `..` prefix). Used both at top
    /// level and inside filter expressions.
    fn parse_segments(&mut self, in_filter: bool) -> Result<Vec<Segment>, PathError> {
        let mut segments = Vec::new();
        loop {
            match self.peek() {
                Some(b'.') => {
                    self.pos += 1;
                    let descendant = self.eat_byte(b'.');
                    match self.peek() {
                        Some(b'*') => {
                            self.pos += 1;
                            segments.push(Segment {
                                descendant,
                                selectors: vec![Selector::Wildcard],
                            });
                        }
                        Some(c) if Self::ident_start(c) => {
                            let name = self.parse_shorthand_name()?;
                            segments.push(Segment { descendant, selectors: vec![Selector::Name(name)] });
                        }
                        Some(b'[') if descendant || in_filter => {
                            // `.[` is not RFC shorthand, but accepting it is a
                            // harmless superset; `..[` is required by RFC.
                            let seg = self.parse_bracket(descendant)?;
                            segments.push(seg);
                        }
                        _ => {
                            return Err(self.err(if descendant {
                                "expected name, '*', or '[' after '..'"
                            } else {
                                "expected name, '*', or '[' after '.'"
                            }));
                        }
                    }
                }
                Some(b'[') => {
                    let seg = self.parse_bracket(false)?;
                    segments.push(seg);
                }
                _ => break,
            }
        }
        Ok(segments)
    }

    fn parse_bracket(&mut self, descendant: bool) -> Result<Segment, PathError> {
        debug_assert_eq!(self.peek(), Some(b'['));
        self.pos += 1;
        let mut selectors = Vec::new();
        loop {
            self.ws();
            if self.eat_byte(b']') {
                if selectors.is_empty() {
                    return Err(self.err("empty bracket selector"));
                }
                break;
            }
            selectors.push(self.parse_selector()?);
            self.ws();
            if self.eat_byte(b',') {
                continue;
            }
            if self.eat_byte(b']') {
                break;
            }
            return Err(self.err("expected ',' or ']' in bracket selector"));
        }
        Ok(Segment { descendant, selectors })
    }

    fn parse_selector(&mut self) -> Result<Selector, PathError> {
        match self.peek() {
            Some(b'*') => {
                self.pos += 1;
                Ok(Selector::Wildcard)
            }
            Some(b'?') => {
                self.pos += 1;
                self.ws();
                let expr = self.parse_or()?;
                Ok(Selector::Filter(expr))
            }
            Some(b'\'') => Ok(Selector::Name(self.parse_quoted_string()?.into())),
            Some(c) if c.is_ascii_digit() || c == b'-' || c == b':' => self.parse_index_or_slice(),
            Some(_) => Err(self.err("unexpected character in bracket selector")),
            None => Err(self.err("unterminated bracket selector")),
        }
    }

    fn parse_index_or_slice(&mut self) -> Result<Selector, PathError> {
        let first = self.parse_opt_i64()?;
        self.ws();
        if !self.eat_byte(b':') {
            return match first {
                Some(i) => Ok(Selector::Index(i)),
                None => Err(self.err("expected array index or slice")),
            };
        }
        self.ws();
        let second = self.parse_opt_i64()?;
        self.ws();
        let step = if self.eat_byte(b':') {
            self.ws();
            let step = self.parse_opt_i64()?.unwrap_or(1);
            if step == 0 {
                return Err(self.err("slice step must not be zero"));
            }
            step
        } else {
            1
        };
        Ok(Selector::Slice { start: first, end: second, step })
    }

    fn parse_i64(&mut self) -> Result<i64, PathError> {
        let start = self.pos;
        if self.eat_byte(b'-') && !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            return Err(self.err("expected digit after '-'"));
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start || (self.b[start] == b'-' && self.pos == start + 1) {
            return Err(self.err("expected number"));
        }
        self.s[start..self.pos].parse::<i64>().map_err(|_| self.err_at(start, "number out of range"))
    }

    fn parse_opt_i64(&mut self) -> Result<Option<i64>, PathError> {
        if matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == b'-') {
            self.parse_i64().map(Some)
        } else {
            Ok(None)
        }
    }

    fn parse_shorthand_name(&mut self) -> Result<Box<str>, PathError> {
        let start = self.pos;
        self.pos += 1; // ident_start already verified
        while matches!(self.peek(), Some(c) if Self::ident_rest(c)) {
            self.pos += 1;
        }
        Ok(self.s[start..self.pos].into())
    }

    /// Single-quoted string literal with JSON-style escapes (`\'`, `\\`,
    /// `\n`, `\t`, `\r`, `\b`, `\f`, `\/`, `\uXXXX` incl. surrogate pairs).
    fn parse_quoted_string(&mut self) -> Result<String, PathError> {
        debug_assert_eq!(self.peek(), Some(b'\''));
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(b'\'') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.peek().ok_or_else(|| self.err("unterminated string literal"))?;
                    self.pos += 1;
                    match esc {
                        b'\'' => out.push('\''),
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.parse_hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&hi) {
                                // surrogate pair
                                if !(self.eat_byte(b'\\') && self.eat_byte(b'u')) {
                                    return Err(self.err("lone high surrogate in \\u escape"));
                                }
                                let lo = self.parse_hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("invalid low surrogate in \\u escape"));
                                }
                                let cp =
                                    0x10000 + (((hi - 0xD800) as u32) << 10) + (lo - 0xDC00) as u32;
                                char::from_u32(cp)
                                    .ok_or_else(|| self.err("invalid \\u escape"))?
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.err("lone low surrogate in \\u escape"));
                            } else {
                                char::from_u32(hi as u32)
                                    .ok_or_else(|| self.err("invalid \\u escape"))?
                            };
                            out.push(ch);
                        }
                        // Regex metacharacter escapes (`\d`, `\w`, `\s`,
                        // `\b`, ...) pass through verbatim so `match`/
                        // `search` patterns keep their ECMA meaning.
                        _ => {
                            out.push('\\');
                            out.push(esc as char);
                        }
                    }
                }
                Some(_) => {
                    // copy one UTF-8 scalar
                    let ch_len = utf8_len(self.b[self.pos]);
                    let end = (self.pos + ch_len).min(self.b.len());
                    match std::str::from_utf8(&self.b[self.pos..end]) {
                        Ok(chunk) => {
                            out.push_str(chunk);
                            self.pos = end;
                        }
                        Err(_) => return Err(self.err("invalid UTF-8 in string literal")),
                    }
                }
                None => return Err(self.err("unterminated string literal")),
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, PathError> {
        if self.pos + 4 > self.b.len() {
            return Err(self.err("truncated \\u escape"));
        }
        let hex = &self.s[self.pos..self.pos + 4];
        let v = u16::from_str_radix(hex, 16).map_err(|_| self.err("invalid \\u escape"))?;
        self.pos += 4;
        Ok(v)
    }

    // ---- filter expressions ---------------------------------------------

    fn parse_or(&mut self) -> Result<LogicalExpr, PathError> {
        let mut left = self.parse_and()?;
        loop {
            self.ws();
            if self.peek_at(1) == Some(b'|') && self.peek() == Some(b'|') {
                self.pos += 2;
                let right = self.parse_and()?;
                left = LogicalExpr::Or(Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn err_at(&self, offset: usize, reason: &str) -> PathError {
        PathError { input: self.s.to_owned(), offset, reason: reason.to_owned() }
    }

    fn parse_and(&mut self) -> Result<LogicalExpr, PathError> {
        let mut left = self.parse_unary()?;
        loop {
            self.ws();
            if self.peek() == Some(b'&') && self.peek_at(1) == Some(b'&') {
                self.pos += 2;
                let right = self.parse_unary()?;
                left = LogicalExpr::And(Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn parse_unary(&mut self) -> Result<LogicalExpr, PathError> {
        self.ws();
        if self.eat_byte(b'!') {
            let inner = self.parse_unary()?;
            return Ok(LogicalExpr::Not(Box::new(inner)));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<LogicalExpr, PathError> {
        self.ws();
        match self.peek() {
            None => Err(self.err("unexpected end of filter expression")),
            Some(b'(') => {
                self.pos += 1;
                let inner = self.parse_or()?;
                self.ws();
                if !self.eat_byte(b')') {
                    return Err(self.err("expected ')'"));
                }
                Ok(inner)
            }
            Some(b'$') | Some(b'@') => {
                let q = self.parse_query_ast()?;
                self.finish_query_atom(q)
            }
            Some(c) if c.is_ascii_digit() || matches!(c, b'\'' | b't' | b'f' | b'n' | b'-') => {
                let lit = self.parse_literal()?;
                self.finish_comparable_lhs(Comparable::Lit(lit))
            }
            Some(c) if Self::func_ident_start(c) => match self.try_parse_function()? {
                Some(f) => self.finish_func_atom(f),
                None => Err(self.err("unknown function extension")),
            },
            Some(_) => Err(self.err("unexpected character in filter expression")),
        }
    }

    /// Query atom: comparison left-hand side (must be singular per RFC 9535)
    /// or a bare existence test.
    fn finish_query_atom(&mut self, q: QueryAst) -> Result<LogicalExpr, PathError> {
        self.ws();
        if let Some(op) = self.try_comparator() {
            if !q.is_singular() {
                return Err(self.err("comparand query must be singular"));
            }
            let rhs = self.parse_rhs_comparable()?;
            Ok(LogicalExpr::Compare(Comparable::Query(q), op, rhs))
        } else {
            Ok(LogicalExpr::Test(Testable::Query(q)))
        }
    }

    /// Function-call atom: comparison left-hand side or a bare logical test.
    fn finish_func_atom(&mut self, f: FunctionCall) -> Result<LogicalExpr, PathError> {
        self.ws();
        if let Some(op) = self.try_comparator() {
            let rhs = self.parse_rhs_comparable()?;
            Ok(LogicalExpr::Compare(Comparable::Func(f), op, rhs))
        } else {
            Ok(LogicalExpr::Test(Testable::Func(f)))
        }
    }

    fn finish_comparable_lhs(&mut self, lhs: Comparable) -> Result<LogicalExpr, PathError> {
        self.ws();
        if let Some(op) = self.try_comparator() {
            let rhs = self.parse_rhs_comparable()?;
            Ok(LogicalExpr::Compare(lhs, op, rhs))
        } else {
            Err(self.err("a literal alone is not a valid filter test"))
        }
    }

    fn parse_rhs_comparable(&mut self) -> Result<Comparable, PathError> {
        self.ws();
        match self.peek() {
            Some(b'$') | Some(b'@') => {
                let q = self.parse_query_ast()?;
                if !q.is_singular() {
                    return Err(self.err("comparand query must be singular"));
                }
                Ok(Comparable::Query(q))
            }
            Some(c) if c.is_ascii_digit() || matches!(c, b'\'' | b't' | b'f' | b'n' | b'-') => {
                Ok(Comparable::Lit(self.parse_literal()?))
            }
            Some(c) if Self::func_ident_start(c) => match self.try_parse_function()? {
                Some(f) => Ok(Comparable::Func(f)),
                None => Err(self.err("unknown function extension")),
            },
            _ => Err(self.err("expected comparand")),
        }
    }

    fn try_comparator(&mut self) -> Option<Comparator> {
        let two = |p: &Self, a: u8, b: u8| p.peek() == Some(a) && p.peek_at(1) == Some(b);
        let (op, len) = if two(self, b'=', b'=') {
            (Some(Comparator::Eq), 2)
        } else if two(self, b'!', b'=') {
            (Some(Comparator::Ne), 2)
        } else if two(self, b'<', b'=') {
            (Some(Comparator::Le), 2)
        } else if two(self, b'>', b'=') {
            (Some(Comparator::Ge), 2)
        } else if self.peek() == Some(b'<') {
            (Some(Comparator::Lt), 1)
        } else if self.peek() == Some(b'>') {
            (Some(Comparator::Gt), 1)
        } else {
            (None, 0)
        };
        self.pos += len;
        op
    }

    /// Parses `$...`/`@...` starting at the root identifier.
    fn parse_query_ast(&mut self) -> Result<QueryAst, PathError> {
        let absolute = match self.peek() {
            Some(b'$') => true,
            Some(b'@') => false,
            _ => return Err(self.err("expected '$' or '@'")),
        };
        self.pos += 1;
        let segments = self.parse_segments(true)?;
        Ok(QueryAst { absolute, segments })
    }

    // ---- literals and functions ------------------------------------------

    fn parse_literal(&mut self) -> Result<Lit, PathError> {
        match self.peek() {
            Some(b'\'') => Ok(Lit::Str(self.parse_quoted_string()?)),
            Some(b't') if self.s[self.pos..].starts_with("true") => {
                self.pos += 4;
                Ok(Lit::Bool(true))
            }
            Some(b'f') if self.s[self.pos..].starts_with("false") => {
                self.pos += 5;
                Ok(Lit::Bool(false))
            }
            Some(b'n') if self.s[self.pos..].starts_with("null") => {
                self.pos += 4;
                Ok(Lit::Null)
            }
            Some(c) if c.is_ascii_digit() || c == b'-' => self.parse_number(),
            _ => Err(self.err("expected literal")),
        }
    }

    /// `-?digits(.digits)?([eE][+-]?digits)?`; integral forms become `Int`.
    fn parse_number(&mut self) -> Result<Lit, PathError> {
        let start = self.pos;
        self.eat_byte(b'-');
        if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            return Err(self.err("expected digit"));
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("expected digit after '.'"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("expected exponent digits"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text = &self.s[start..self.pos];
        if !is_float
            && let Ok(i) = text.parse::<i64>() {
                return Ok(Lit::Int(i));
            }
        text.parse::<f64>().map(Lit::Float).map_err(|_| self.err_at(start, "bad number"))
    }

    fn func_ident_start(c: u8) -> bool {
        c.is_ascii_lowercase() || c == b'_'
    }

    fn read_func_ident(&mut self) -> (String, usize) {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_') {
            self.pos += 1;
        }
        (self.s[start..self.pos].to_owned(), start)
    }

    /// Parses `name(args...)` when `name` is a known extension; returns
    /// `Ok(None)` for unknown names (caller decides whether that is fatal).
    fn try_parse_function(&mut self) -> Result<Option<FunctionCall>, PathError> {
        let save = self.pos;
        let (name, start) = self.read_func_ident();
        let fname = match name.as_str() {
            "length" => FuncName::Length,
            "count" => FuncName::Count,
            "value" => FuncName::Value,
            "match" => FuncName::Match,
            "search" => FuncName::Search,
            _ => {
                self.pos = save;
                return Ok(None);
            }
        };
        if !self.eat_byte(b'(') {
            return Err(self.err_at(start, "expected '(' after function name"));
        }
        let mut args = Vec::new();
        let mut regex = None;
        loop {
            self.ws();
            if args.is_empty() && self.eat_byte(b')') {
                break;
            }
            let arg = self.parse_farg()?;
            args.push(arg);
            self.ws();
            if self.eat_byte(b',') {
                continue;
            }
            if self.eat_byte(b')') {
                break;
            }
            return Err(self.err("expected ',' or ')' in function arguments"));
        }
        // arity + argument validation; compile regexes now so evaluation
        // never pays for it.
        match fname {
            FuncName::Length | FuncName::Count | FuncName::Value => {
                if args.len() != 1 {
                    return Err(self.err_at(start, "function takes exactly one argument"));
                }
            }
            FuncName::Match | FuncName::Search => {
                if args.len() != 2 {
                    return Err(self.err_at(start, "match/search take exactly two arguments"));
                }
                let pat = match &args[1] {
                    FArg::Comparable(Comparable::Lit(Lit::Str(s))) => s.clone(),
                    _ => {
                        return Err(
                            self.err_at(start, "match/search pattern must be a string literal")
                        );
                    }
                };
                let re = regex::Regex::new(&pat)
                    .map_err(|e| self.err_at(start, &format!("invalid regex: {e}")))?;
                regex = Some(re);
            }
        }
        Ok(Some(FunctionCall { name: fname, args, regex }))
    }

    fn parse_farg(&mut self) -> Result<FArg, PathError> {
        self.ws();
        match self.peek() {
            Some(b'$') | Some(b'@') => self.parse_query_ast().map(FArg::Query),
            Some(b'\'') => Ok(FArg::Comparable(Comparable::Lit(Lit::Str(
                self.parse_quoted_string()?,
            )))),
            Some(c) if c.is_ascii_digit() || matches!(c, b't' | b'f' | b'n' | b'-') => {
                self.parse_literal().map(|l| FArg::Comparable(Comparable::Lit(l)))
            }
            Some(c) if Self::func_ident_start(c) => match self.try_parse_function()? {
                Some(f) => Ok(FArg::Comparable(Comparable::Func(f))),
                None => Err(self.err("unknown function extension")),
            },
            _ => self.parse_or().map(FArg::Logical),
        }
    }
}


fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}
