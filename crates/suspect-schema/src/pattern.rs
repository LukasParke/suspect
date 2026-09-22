//! A bounded, portable ECMA-262 regular-pattern subset compiled to Thompson NFA.

use serde::Serialize;

/// Portable pattern program version.
pub const PATTERN_VERSION: &str = "suspect.pattern.experimental.v1";
const MAX_BYTES: usize = 4096;
const MAX_DEPTH: usize = 64;
const MAX_STATES: usize = 8192;
const MAX_TOTAL_RANGES: usize = 65_536;
const MAX_REPEAT: usize = 1024;
const MAX_EXPANSION_WORK: usize = 32_768;

/// A compiled portable Thompson NFA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PatternProgram {
    /// Exact pattern-program format discriminator.
    pub version: &'static str,
    /// Initial state index.
    pub start: usize,
    /// Finite instruction graph.
    pub states: Vec<PatternState>,
}

/// One portable pattern instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum PatternState {
    /// Accept the match.
    Match,
    /// Consume one scalar in any normalized inclusive range.
    Char {
        /// Sorted disjoint inclusive Unicode scalar ranges.
        ranges: Vec<[u32; 2]>,
        /// Next state after consuming a scalar.
        target: usize,
    },
    /// Follow two epsilon edges.
    Split {
        /// Preferred epsilon target.
        first: usize,
        /// Alternate epsilon target.
        second: usize,
    },
    /// Follow one epsilon edge.
    Jump {
        /// Epsilon target.
        target: usize,
    },
    /// Require the strict start of the input.
    Start {
        /// Target when positioned at strict input start.
        target: usize,
    },
    /// Require the strict end of the input.
    End {
        /// Target when positioned at strict input end.
        target: usize,
    },
}

/// Pattern compilation failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternErrorKind {
    /// Malformed Unicode pattern grammar.
    Invalid,
    /// Valid syntax outside the portable subset.
    Unsupported,
    /// A declared compile resource limit was exceeded.
    Limit,
}

/// A bounded pattern compilation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternError {
    /// Stable failure category.
    pub kind: PatternErrorKind,
    /// Human-readable explanation.
    pub message: String,
}

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for PatternError {}

#[derive(Clone)]
enum Ast {
    Empty,
    Char(Vec<[u32; 2]>),
    Start,
    End,
    Group(Box<Ast>),
    Seq(Vec<Ast>),
    Alt(Vec<Ast>),
    Repeat(Box<Ast>, usize, Option<usize>),
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    depth: usize,
    captures: usize,
    backreferences: Vec<usize>,
}
impl Parser {
    fn parse(source: &str) -> Result<Ast, PatternError> {
        if source.len() > MAX_BYTES {
            return Err(err(
                PatternErrorKind::Limit,
                "pattern exceeds 4096 source bytes",
            ));
        }
        let mut p = Self {
            chars: source.chars().collect(),
            at: 0,
            depth: 0,
            captures: 0,
            backreferences: Vec::new(),
        };
        let ast = p.alt()?;
        if p.at != p.chars.len() {
            return Err(err(PatternErrorKind::Invalid, "unexpected pattern token"));
        }
        if let Some(reference) = p.backreferences.iter().copied().max() {
            return Err(err(
                if reference <= p.captures {
                    PatternErrorKind::Unsupported
                } else {
                    PatternErrorKind::Invalid
                },
                if reference <= p.captures {
                    "backreferences are unsupported"
                } else {
                    "backreference does not identify a capture group"
                },
            ));
        }
        Ok(ast)
    }
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }
    fn take(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        Some(c)
    }
    fn alt(&mut self) -> Result<Ast, PatternError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(err(
                PatternErrorKind::Limit,
                "pattern parse depth exceeds 64",
            ));
        }
        let mut xs = vec![self.seq()?];
        while self.peek() == Some('|') {
            self.at += 1;
            xs.push(self.seq()?)
        }
        self.depth -= 1;
        Ok(if xs.len() == 1 {
            xs.pop().unwrap()
        } else {
            Ast::Alt(xs)
        })
    }
    fn seq(&mut self) -> Result<Ast, PatternError> {
        let mut xs = vec![];
        while !matches!(self.peek(), None | Some(')' | '|')) {
            xs.push(self.piece()?)
        }
        Ok(match xs.len() {
            0 => Ast::Empty,
            1 => xs.pop().unwrap(),
            _ => Ast::Seq(xs),
        })
    }
    fn piece(&mut self) -> Result<Ast, PatternError> {
        let atom = self.atom()?;
        let Some(c) = self.peek() else {
            return Ok(atom);
        };
        let q = match c {
            '?' => Some((0, Some(1), 1)),
            '*' => Some((0, None, 1)),
            '+' => Some((1, None, 1)),
            '{' => {
                self.at += 1;
                let min = self.uint()?;
                let max = match self.take() {
                    Some('}') => Some(min),
                    Some(',') => {
                        if self.peek() == Some('}') {
                            self.at += 1;
                            None
                        } else {
                            let n = self.uint()?;
                            if self.take() != Some('}') {
                                return Err(err(PatternErrorKind::Invalid, "invalid repetition"));
                            }
                            Some(n)
                        }
                    }
                    _ => return Err(err(PatternErrorKind::Invalid, "invalid repetition")),
                };
                Some((min, max, 0))
            }
            _ => None,
        };
        if let Some((min, max, advance)) = q {
            self.at += advance;
            if matches!(atom, Ast::Start | Ast::End) {
                return Err(err(
                    PatternErrorKind::Invalid,
                    "bare assertions cannot be quantified",
                ));
            }
            if min > MAX_REPEAT
                || max.is_some_and(|n| n > MAX_REPEAT)
                || max.is_some_and(|n| n < min)
            {
                return Err(err(
                    if min > MAX_REPEAT || max.is_some_and(|n| n > MAX_REPEAT) {
                        PatternErrorKind::Limit
                    } else {
                        PatternErrorKind::Invalid
                    },
                    "invalid or excessive repetition",
                ));
            }
            if self.peek() == Some('?') {
                return Err(err(
                    PatternErrorKind::Unsupported,
                    "lazy quantifiers are unsupported",
                ));
            }
            Ok(Ast::Repeat(Box::new(atom), min, max))
        } else {
            Ok(atom)
        }
    }
    fn uint(&mut self) -> Result<usize, PatternError> {
        let start = self.at;
        let mut n = 0usize;
        while let Some(c) = self.peek().filter(|c| c.is_ascii_digit()) {
            self.at += 1;
            n = n
                .checked_mul(10)
                .and_then(|x| x.checked_add(c as usize - '0' as usize))
                .ok_or_else(|| err(PatternErrorKind::Limit, "repetition is too large"))?;
        }
        if self.at == start {
            return Err(err(
                PatternErrorKind::Invalid,
                "repetition requires a decimal integer",
            ));
        }
        Ok(n)
    }
    fn atom(&mut self) -> Result<Ast, PatternError> {
        match self
            .take()
            .ok_or_else(|| err(PatternErrorKind::Invalid, "missing pattern atom"))?
        {
            '(' => {
                if self.peek() == Some('?') {
                    self.at += 1;
                    if self.take() != Some(':') {
                        return Err(err(
                            PatternErrorKind::Unsupported,
                            "lookarounds and special groups are unsupported",
                        ));
                    }
                } else {
                    self.captures += 1;
                }
                let x = self.alt()?;
                if self.take() != Some(')') {
                    return Err(err(PatternErrorKind::Invalid, "unclosed group"));
                }
                Ok(Ast::Group(Box::new(x)))
            }
            '[' => self.class(),
            '.' => Ok(Ast::Char(vec![
                [0, 9],
                [11, 12],
                [14, 0x2027],
                [0x202a, 0xd7ff],
                [0xe000, 0x10ffff],
            ])),
            '^' => Ok(Ast::Start),
            '$' => Ok(Ast::End),
            '\\' => Ok(Ast::Char(self.escape(false)?)),
            ')' | '|' | '*' | '+' | '?' | '{' | ']' | '}' => Err(err(
                PatternErrorKind::Invalid,
                "unexpected pattern metacharacter",
            )),
            c => Ok(Ast::Char(vec![[c as u32, c as u32]])),
        }
    }
    fn escape(&mut self, in_class: bool) -> Result<Vec<[u32; 2]>, PatternError> {
        let c = self
            .take()
            .ok_or_else(|| err(PatternErrorKind::Invalid, "trailing escape"))?;
        Ok(match c {
            'w' => vec![[48, 57], [65, 90], [95, 95], [97, 122]],
            'W' => complement(vec![[48, 57], [65, 90], [95, 95], [97, 122]]),
            'd' => vec![[48, 57]],
            'D' => complement(vec![[48, 57]]),
            's' => vec![
                [9, 13],
                [32, 32],
                [160, 160],
                [0x1680, 0x1680],
                [0x2000, 0x200a],
                [0x2028, 0x2029],
                [0x202f, 0x202f],
                [0x205f, 0x205f],
                [0x3000, 0x3000],
                [0xfeff, 0xfeff],
            ],
            'S' => complement(vec![
                [9, 13],
                [32, 32],
                [160, 160],
                [0x1680, 0x1680],
                [0x2000, 0x200a],
                [0x2028, 0x2029],
                [0x202f, 0x202f],
                [0x205f, 0x205f],
                [0x3000, 0x3000],
                [0xfeff, 0xfeff],
            ]),
            'n' => vec![[10, 10]],
            'r' => vec![[13, 13]],
            't' => vec![[9, 9]],
            'f' => vec![[12, 12]],
            'v' => vec![[11, 11]],
            'b' if in_class => vec![[8, 8]],
            '0' => {
                if self.peek().is_some_and(|next| next.is_ascii_digit()) {
                    return Err(err(
                        PatternErrorKind::Invalid,
                        "legacy octal escapes are invalid in Unicode mode",
                    ));
                }
                vec![[0, 0]]
            }
            '1'..='9' => {
                if in_class {
                    return Err(err(
                        PatternErrorKind::Invalid,
                        "decimal escapes are invalid in Unicode character classes",
                    ));
                }
                let mut reference = c.to_digit(10).unwrap() as usize;
                while let Some(digit) = self.peek().and_then(|next| next.to_digit(10)) {
                    self.at += 1;
                    reference = reference
                        .checked_mul(10)
                        .and_then(|value| value.checked_add(digit as usize))
                        .ok_or_else(|| {
                            err(PatternErrorKind::Invalid, "invalid decimal backreference")
                        })?;
                }
                self.backreferences.push(reference);
                Vec::new()
            }
            'p' | 'P' | 'k' => {
                return Err(err(
                    PatternErrorKind::Unsupported,
                    "Unicode properties and named backreferences are unsupported",
                ));
            }
            'b' | 'B' => {
                return Err(err(
                    PatternErrorKind::Unsupported,
                    "word-boundary assertions are unsupported",
                ));
            }
            'c' => {
                let letter = self.take().ok_or_else(|| {
                    err(
                        PatternErrorKind::Invalid,
                        "control escape requires an ASCII letter",
                    )
                })?;
                if !letter.is_ascii_alphabetic() {
                    return Err(err(
                        PatternErrorKind::Invalid,
                        "control escape requires an ASCII letter",
                    ));
                }
                let value = (letter.to_ascii_uppercase() as u32) & 0x1f;
                vec![[value, value]]
            }
            'x' => vec![[self.hex(2)?, self.hex_value_back(2)]],
            'u' => {
                let n = if self.peek() == Some('{') {
                    self.at += 1;
                    let start = self.at;
                    let mut value = 0u32;
                    while let Some(digit) = self.peek().and_then(|ch| ch.to_digit(16)) {
                        self.at += 1;
                        value = value
                            .checked_mul(16)
                            .and_then(|v| v.checked_add(digit))
                            .ok_or_else(|| {
                                err(
                                    PatternErrorKind::Invalid,
                                    "invalid Unicode code point escape",
                                )
                            })?;
                    }
                    if self.at == start || self.take() != Some('}') || value > 0x10ffff {
                        return Err(err(
                            PatternErrorKind::Invalid,
                            "invalid Unicode code point escape",
                        ));
                    }
                    value
                } else {
                    self.hex(4)?
                };
                if (0xd800..=0xdfff).contains(&n) {
                    return Err(err(
                        PatternErrorKind::Unsupported,
                        "UTF-16 surrogate escapes are unsupported by the scalar program",
                    ));
                }
                vec![[n, n]]
            }
            c if "^$\\.*+?()[]{}|/".contains(c) || (in_class && c == '-') => {
                vec![[c as u32, c as u32]]
            }
            _ => {
                return Err(err(
                    PatternErrorKind::Invalid,
                    "invalid escape in Unicode pattern",
                ));
            }
        })
    }
    fn hex(&mut self, n: usize) -> Result<u32, PatternError> {
        let mut v = 0;
        for _ in 0..n {
            let c = self
                .take()
                .and_then(|c| c.to_digit(16))
                .ok_or_else(|| err(PatternErrorKind::Invalid, "invalid hexadecimal escape"))?;
            v = v * 16 + c;
        }
        Ok(v)
    }
    fn hex_value_back(&self, n: usize) -> u32 {
        self.chars[self.at - n..self.at]
            .iter()
            .fold(0, |v, c| v * 16 + c.to_digit(16).unwrap())
    }
    fn class(&mut self) -> Result<Ast, PatternError> {
        let neg = if self.peek() == Some('^') {
            self.at += 1;
            true
        } else {
            false
        };
        let mut rs = vec![];
        while let Some(c) = self.peek() {
            if c == ']' {
                self.at += 1;
                let rs = normalize(rs);
                return Ok(Ast::Char(if neg { complement(rs) } else { rs }));
            }
            let a = if self.take() == Some('\\') {
                self.escape(true)?
            } else {
                vec![[c as u32, c as u32]]
            };
            if self.peek() == Some('-') && self.chars.get(self.at + 1) != Some(&']') {
                if a.len() != 1 || a[0][0] != a[0][1] {
                    return Err(err(
                        PatternErrorKind::Invalid,
                        "character class range endpoint must be one scalar",
                    ));
                }
                self.at += 1;
                let d = self
                    .take()
                    .ok_or_else(|| err(PatternErrorKind::Invalid, "unclosed character class"))?;
                let b = if d == '\\' {
                    self.escape(true)?
                } else {
                    vec![[d as u32, d as u32]]
                };
                if b.len() != 1 || b[0][0] != b[0][1] || a[0][0] > b[0][0] {
                    return Err(err(
                        PatternErrorKind::Invalid,
                        "invalid character class range",
                    ));
                }
                rs.push([a[0][0], b[0][0]])
            } else {
                rs.extend(a)
            }
        }
        Err(err(PatternErrorKind::Invalid, "unclosed character class"))
    }
}

fn err(kind: PatternErrorKind, message: &str) -> PatternError {
    PatternError {
        kind,
        message: message.into(),
    }
}
fn normalize(rs: Vec<[u32; 2]>) -> Vec<[u32; 2]> {
    let mut scalar_ranges = Vec::with_capacity(rs.len().saturating_add(1));
    for [lo, hi] in rs.into_iter().filter(|range| range[0] <= range[1]) {
        if lo < 0xd800 {
            scalar_ranges.push([lo, hi.min(0xd7ff)]);
        }
        if hi > 0xdfff {
            scalar_ranges.push([lo.max(0xe000), hi]);
        }
    }
    scalar_ranges.sort_unstable();
    let mut out: Vec<[u32; 2]> = vec![];
    for r in scalar_ranges {
        if let Some(last) = out.last_mut().filter(|x| r[0] <= x[1].saturating_add(1)) {
            last[1] = last[1].max(r[1])
        } else {
            out.push(r)
        }
    }
    out
}
fn complement(rs: Vec<[u32; 2]>) -> Vec<[u32; 2]> {
    let rs = normalize(rs);
    let mut out = vec![];
    for [lo, hi] in [[0, 0xd7ff], [0xe000, 0x10ffff]] {
        let mut at = lo;
        for r in &rs {
            if r[1] < lo || r[0] > hi {
                continue;
            }
            if at < r[0] {
                out.push([at, r[0] - 1])
            }
            at = at.max(r[1].saturating_add(1));
        }
        if at <= hi {
            out.push([at, hi])
        }
    }
    out
}

struct Builder {
    states: Vec<PatternState>,
    work: usize,
    total_ranges: usize,
}
impl Builder {
    fn spend(&mut self) -> Result<(), PatternError> {
        self.work += 1;
        if self.work > MAX_EXPANSION_WORK {
            return Err(err(
                PatternErrorKind::Limit,
                "pattern expansion exceeds 32768 bounded visits",
            ));
        }
        Ok(())
    }
    fn push(&mut self, s: PatternState) -> Result<usize, PatternError> {
        if self.states.len() >= MAX_STATES {
            return Err(err(
                PatternErrorKind::Limit,
                "expanded pattern exceeds 8192 states",
            ));
        }
        let i = self.states.len();
        self.states.push(s);
        Ok(i)
    }
    fn admit_ranges(&mut self, count: usize) -> Result<(), PatternError> {
        self.total_ranges = self.total_ranges.checked_add(count).ok_or_else(|| {
            err(
                PatternErrorKind::Limit,
                "pattern range count exceeds portable bounds",
            )
        })?;
        if self.total_ranges > MAX_TOTAL_RANGES {
            return Err(err(
                PatternErrorKind::Limit,
                "pattern exceeds 65536 total character ranges",
            ));
        }
        Ok(())
    }
    fn build(&mut self, a: &Ast, next: usize) -> Result<usize, PatternError> {
        if let Ast::Char(ranges) = a {
            self.admit_ranges(ranges.len())?;
            self.spend()?;
            return self.push(PatternState::Char {
                ranges: ranges.clone(),
                target: next,
            });
        }
        self.spend()?;
        match a {
            Ast::Empty => Ok(next),
            Ast::Char(_) => unreachable!("character states return after aggregate admission"),
            Ast::Start => self.push(PatternState::Start { target: next }),
            Ast::End => self.push(PatternState::End { target: next }),
            Ast::Group(inner) => self.build(inner, next),
            Ast::Seq(xs) => {
                let mut n = next;
                for x in xs.iter().rev() {
                    n = self.build(x, n)?
                }
                Ok(n)
            }
            Ast::Alt(xs) => {
                let mut starts = Vec::new();
                for x in xs {
                    starts.push(self.build(x, next)?)
                }
                let mut n = starts.pop().unwrap();
                for s in starts.into_iter().rev() {
                    n = self.push(PatternState::Split {
                        first: s,
                        second: n,
                    })?
                }
                Ok(n)
            }
            Ast::Repeat(x, min, max) => {
                let mut n = next;
                match max {
                    Some(mx) => {
                        for _ in *min..*mx {
                            self.spend()?;
                            let s = self.build(x, n)?;
                            n = self.push(PatternState::Split {
                                first: s,
                                second: n,
                            })?
                        }
                    }
                    None => {
                        let split = self.push(PatternState::Jump { target: 0 })?;
                        let s = self.build(x, split)?;
                        self.states[split] = PatternState::Split {
                            first: s,
                            second: n,
                        };
                        n = split
                    }
                }
                for _ in 0..*min {
                    self.spend()?;
                    n = self.build(x, n)?
                }
                Ok(n)
            }
        }
    }
}

/// Compile the supported ECMA-262 Unicode regular subset.
pub fn compile_pattern(source: &str) -> Result<PatternProgram, PatternError> {
    let ast = Parser::parse(source)?;
    let mut b = Builder {
        states: vec![],
        work: 0,
        total_ranges: 0,
    };
    let accept = b.push(PatternState::Match)?;
    let start = b.build(&ast, accept)?;
    let program = PatternProgram {
        version: PATTERN_VERSION,
        start,
        states: b.states,
    };
    program.check()?;
    Ok(program)
}

impl PatternProgram {
    /// Validate version, graph targets, and normalized Unicode scalar ranges.
    pub fn check(&self) -> Result<(), PatternError> {
        if self.version != PATTERN_VERSION {
            return Err(err(
                PatternErrorKind::Unsupported,
                "unsupported pattern program version",
            ));
        }
        if self.states.is_empty()
            || self.start >= self.states.len()
            || self.states.len() > MAX_STATES
        {
            return Err(err(
                PatternErrorKind::Invalid,
                "invalid finite pattern graph",
            ));
        }
        let mut total_ranges = 0usize;
        for state in &self.states {
            if let PatternState::Char { ranges, .. } = state {
                total_ranges = total_ranges.checked_add(ranges.len()).ok_or_else(|| {
                    err(
                        PatternErrorKind::Limit,
                        "pattern range count exceeds portable bounds",
                    )
                })?;
                if total_ranges > MAX_TOTAL_RANGES {
                    return Err(err(
                        PatternErrorKind::Limit,
                        "pattern exceeds 65536 total character ranges",
                    ));
                }
            }
        }
        for s in &self.states {
            match s {
                PatternState::Char { ranges, target } => {
                    if ranges.len() > MAX_STATES {
                        return Err(err(
                            PatternErrorKind::Invalid,
                            "pattern character ranges exceed portable bounds",
                        ));
                    }
                    if *target >= self.states.len() || normalize(ranges.clone()) != *ranges {
                        return Err(err(
                            PatternErrorKind::Invalid,
                            "invalid pattern character ranges or target",
                        ));
                    }
                    if ranges.iter().any(|range| range[1] > 0x10ffff) {
                        return Err(err(
                            PatternErrorKind::Invalid,
                            "pattern character ranges exceed portable bounds",
                        ));
                    }
                }
                PatternState::Split { first, second }
                    if *first >= self.states.len() || *second >= self.states.len() =>
                {
                    return Err(err(
                        PatternErrorKind::Invalid,
                        "pattern target is outside graph",
                    ));
                }
                PatternState::Jump { target }
                | PatternState::Start { target }
                | PatternState::End { target }
                    if *target >= self.states.len() =>
                {
                    return Err(err(
                        PatternErrorKind::Invalid,
                        "pattern target is outside graph",
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Evaluate with a caller-owned shared work budget.
pub(crate) fn is_match(
    program: &PatternProgram,
    input: &str,
    remaining: &mut usize,
) -> Result<bool, ()> {
    fn charge(r: &mut usize) -> Result<(), ()> {
        if *r == 0 {
            Err(())
        } else {
            *r -= 1;
            Ok(())
        }
    }
    fn enqueue(
        index: usize,
        epoch: u32,
        seen: &mut [u32],
        stack: &mut Vec<usize>,
        remaining: &mut usize,
    ) -> Result<(), ()> {
        charge(remaining)?;
        if seen[index] != epoch {
            seen[index] = epoch;
            stack.push(index);
        }
        Ok(())
    }
    let mut seen = vec![0u32; program.states.len()];
    let mut stack = Vec::with_capacity(program.states.len());
    let mut active = Vec::with_capacity(program.states.len());
    let mut seeds = Vec::with_capacity(program.states.len());
    let mut next_seeds = Vec::with_capacity(program.states.len());
    let mut scalars = input.char_indices();
    let mut offset = 0usize;
    let mut epoch = 0u32;
    loop {
        charge(remaining)?;
        epoch = epoch.wrapping_add(1);
        if epoch == 0 {
            seen.fill(0);
            epoch = 1;
        }
        stack.clear();
        active.clear();
        enqueue(program.start, epoch, &mut seen, &mut stack, remaining)?;
        for seed in seeds.drain(..) {
            enqueue(seed, epoch, &mut seen, &mut stack, remaining)?;
        }
        while let Some(index) = stack.pop() {
            charge(remaining)?;
            match &program.states[index] {
                PatternState::Match => return Ok(true),
                PatternState::Split { first, second } => {
                    enqueue(*second, epoch, &mut seen, &mut stack, remaining)?;
                    enqueue(*first, epoch, &mut seen, &mut stack, remaining)?;
                }
                PatternState::Jump { target } => {
                    enqueue(*target, epoch, &mut seen, &mut stack, remaining)?
                }
                PatternState::Start { target } if offset == 0 => {
                    enqueue(*target, epoch, &mut seen, &mut stack, remaining)?
                }
                PatternState::End { target } if offset == input.len() => {
                    enqueue(*target, epoch, &mut seen, &mut stack, remaining)?
                }
                PatternState::Char { .. } => active.push(index),
                _ => {}
            }
        }
        let Some((byte_offset, scalar)) = scalars.next() else {
            return Ok(false);
        };
        debug_assert_eq!(byte_offset, offset);
        offset += scalar.len_utf8();
        next_seeds.clear();
        let scalar = scalar as u32;
        for index in active.drain(..) {
            if let PatternState::Char { ranges, target } = &program.states[index] {
                for range in ranges {
                    charge(remaining)?;
                    if scalar < range[0] {
                        break;
                    }
                    if scalar <= range[1] {
                        next_seeds.push(*target);
                        break;
                    }
                }
            }
        }
        std::mem::swap(&mut seeds, &mut next_seeds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn openrouter_patterns() {
        for p in [
            "^[a-zA-Z0-9 _-]+$",
            r"^[a-z0-9*]([a-z0-9*-]{0,61}[a-z0-9*])?(\.[a-z0-9*]([a-z0-9*-]{0,61}[a-z0-9*])?)*$",
            r"^[\w-]+$",
        ] {
            let n = compile_pattern(p).unwrap();
            n.check().unwrap();
            let mut budget = 100000;
            assert!(
                is_match(
                    &n,
                    if p.contains("\\.") {
                        "pypi.org"
                    } else {
                        "sess_abc123"
                    },
                    &mut budget
                )
                .unwrap()
            );
        }
    }
    #[test]
    fn unicode_and_budget() {
        let n = compile_pattern("é|x+$").unwrap();
        let mut b = 1000;
        assert!(is_match(&n, "aéz", &mut b).unwrap());
        let mut b = 0;
        assert!(is_match(&n, "x", &mut b).is_err());
    }

    #[test]
    fn expansion_work_is_bounded_even_when_repeats_emit_no_states() {
        let error = compile_pattern("(?:(){1024}){1024}").unwrap_err();
        assert_eq!(error.kind, PatternErrorKind::Limit);
    }

    #[test]
    fn unicode_mode_syntax_and_capability_failures_are_distinct() {
        for source in ["]", "}", "^*", "$+", r"\-", r"\01"] {
            assert_eq!(
                compile_pattern(source).unwrap_err().kind,
                PatternErrorKind::Invalid,
                "{source:?}"
            );
        }
        for source in [r"\b", r"\B", r"\uD800", r"\uD83D\uDCA9"] {
            assert_eq!(
                compile_pattern(source).unwrap_err().kind,
                PatternErrorKind::Unsupported,
                "{source:?}"
            );
        }
        compile_pattern(r"(^)*").unwrap();
        compile_pattern(r"^\u{1F4A9}$").unwrap();
    }

    #[test]
    fn normalization_merges_crossing_surrogate_ranges_and_admission_preflights_caps() {
        assert_eq!(
            normalize(vec![[0, 10], [5, 0xe001]]),
            vec![[0, 0xd7ff], [0xe000, 0xe001]]
        );
        let mut program = compile_pattern("a").unwrap();
        let PatternState::Char { ranges, .. } = &mut program.states[1] else {
            panic!("char state")
        };
        *ranges = vec![[0, 0]; MAX_STATES + 1];
        assert!(program.check().is_err());
    }

    #[test]
    fn aggregate_character_ranges_are_bounded_before_normalization() {
        let ranges = (0..500)
            .map(|index| format!(r"\u{{{:x}}}", 0x100 + index * 2))
            .collect::<String>();
        let error = compile_pattern(&format!("[{ranges}]{{132}}")).unwrap_err();
        assert_eq!(error.kind, PatternErrorKind::Limit);

        let ranges: Vec<[u32; 2]> = (0..9).map(|index| [index * 2, index * 2]).collect();
        let program = PatternProgram {
            version: PATTERN_VERSION,
            start: 0,
            states: (0..MAX_STATES)
                .map(|_| PatternState::Char {
                    ranges: ranges.clone(),
                    target: 0,
                })
                .collect(),
        };
        assert_eq!(program.check().unwrap_err().kind, PatternErrorKind::Limit);
    }

    #[test]
    fn zero_budget_does_not_scan_a_large_input() {
        let program = compile_pattern("z$").unwrap();
        let input = "x".repeat(100_000);
        let mut budget = 0;
        assert!(is_match(&program, &input, &mut budget).is_err());
    }

    #[test]
    fn control_escapes_and_decimal_backreferences_follow_unicode_grammar() {
        for (source, scalar) in [
            (r"^\cA$", '\u{1}'),
            (r"^\cZ$", '\u{1a}'),
            (r"^[\ca]$", '\u{1}'),
        ] {
            let program = compile_pattern(source).unwrap();
            let mut budget = 1_000;
            assert!(is_match(&program, &scalar.to_string(), &mut budget).unwrap());
        }
        for source in [r"\c0", r"\c_", r"\1", r"\8", r"[\1]"] {
            assert_eq!(
                compile_pattern(source).unwrap_err().kind,
                PatternErrorKind::Invalid,
                "{source}"
            );
        }
        for source in [r"(.)\1", r"\1(.)", r"(.)(.)\2"] {
            assert_eq!(
                compile_pattern(source).unwrap_err().kind,
                PatternErrorKind::Unsupported,
                "{source}"
            );
        }
    }
}
