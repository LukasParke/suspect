//! `format` assertion (only compiled when `Config::format_assertion` is set;
//! otherwise `format` is annotation-only per 2020-12 §7.2).
//!
//! Hand-written validators, no external date/time crates. Unknown format
//! names are annotation-only and always pass.

use std::rc::Rc;

use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::exec::Ctx;
use crate::exec::Stack;

pub(crate) fn check_format<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    name: &Rc<str>,
) -> bool {
    if inst.kind() != ValueKind::Str {
        return true;
    }
    let Some(s) = inst.as_str() else { return true };
    if validate(name, s) {
        true
    } else {
        ctx.emit(st, at, format!("string is not a valid `{name}`"));
        false
    }
}

pub(crate) fn validate(name: &str, s: &str) -> bool {
    match name {
        "date" => date(s),
        "time" => time(s),
        "date-time" => date_time(s),
        "email" => email(s),
        "hostname" => hostname(s),
        "ipv4" => ipv4(s),
        "ipv6" => ipv6(s),
        "uri" => uri(s, true),
        "uri-reference" => uri(s, false),
        "uuid" => uuid(s),
        "regex" => regex::Regex::new(s).is_ok(),
        "json-pointer" => json_pointer(s),
        "duration" => duration(s),
        // Unknown formats never assert.
        _ => true,
    }
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Fixed-width ASCII digit run.
fn digits(b: &[u8]) -> Option<i64> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut v = 0i64;
    for &c in b {
        v = v * 10 + i64::from(c - b'0');
    }
    Some(v)
}

fn date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let (Some(y), Some(m), Some(d)) = (digits(&b[0..4]), digits(&b[5..7]), digits(&b[8..10]))
    else {
        return false;
    };
    if !(1..=12).contains(&m) {
        return false;
    }
    let dim = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    };
    (1..=dim).contains(&d)
}

/// RFC 3339 `full-time`: `HH:MM:SS[.f+](Z|z|±HH:MM)`; leap second 60 allowed.
fn time(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 9 || b[2] != b':' || b[5] != b':' {
        return false;
    }
    let (Some(h), Some(m), Some(sec)) = (digits(&b[0..2]), digits(&b[3..5]), digits(&b[6..8]))
    else {
        return false;
    };
    if h > 23 || m > 59 || sec > 60 {
        return false;
    }
    let mut i = 8;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    match b.get(i) {
        Some(b'Z') | Some(b'z') => i + 1 == b.len(),
        Some(b'+') | Some(b'-') => {
            let rest = &b[i + 1..];
            rest.len() == 5
                && rest[2] == b':'
                && digits(&rest[0..2]).is_some_and(|oh| oh <= 23)
                && digits(&rest[3..5]).is_some_and(|om| om <= 59)
        }
        _ => false,
    }
}

fn date_time(s: &str) -> bool {
    let Some(t) = s.find(['T', 't']) else {
        return false;
    };
    date(&s[..t]) && time(&s[t + 1..])
}

fn email(s: &str) -> bool {
    let Some(at) = s.rfind('@') else { return false };
    if s.contains('@') && s.find('@') != Some(at) {
        return false; // more than one `@`
    }
    let (local, domain) = (&s[..at], &s[at + 1..]);
    if local.is_empty()
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local.bytes().all(|c| c.is_ascii_graphic())
    {
        return false;
    }
    if domain.is_empty()
        || !domain
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
    {
        return false;
    }
    domain
        .split('.')
        .all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-'))
}

fn hostname(s: &str) -> bool {
    if s.is_empty() || s.len() > 253 {
        return false;
    }
    s.trim_end_matches('.').split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

fn ipv4(s: &str) -> bool {
    let mut count = 0usize;
    let ok = s.split('.').all(|p| {
        count += 1;
        !p.is_empty()
            && p.len() <= 3
            && p.bytes().all(|c| c.is_ascii_digit())
            && (p == "0" || !p.starts_with('0'))
            && p.parse::<u32>().is_ok_and(|v| v <= 255)
    });
    ok && count == 4
}

fn ipv6(s: &str) -> bool {
    if !s.is_ascii() || s.matches("::").count() > 1 {
        return false;
    }
    let (head, tail) = match s.find("::") {
        Some(i) => (&s[..i], Some(&s[i + 2..])),
        None => (s, None),
    };
    let groups = |part: &str, out: &mut Vec<usize>| -> bool {
        if part.is_empty() {
            return true;
        }
        for g in part.split(':') {
            if g.contains('.') {
                // Embedded IPv4 counts as two groups and must be last; the
                // split structure guarantees it is.
                if !ipv4(g) {
                    return false;
                }
                out.push(0);
                out.push(0);
            } else if g.is_empty() || g.len() > 4 || !g.bytes().all(|c| c.is_ascii_hexdigit()) {
                return false;
            } else {
                out.push(1);
            }
        }
        true
    };
    let mut left = Vec::new();
    let mut right = Vec::new();
    if !groups(head, &mut left) {
        return false;
    }
    match tail {
        Some(t) => {
            if !groups(t, &mut right) {
                return false;
            }
            left.len() + right.len() <= 7
        }
        None => left.len() == 8,
    }
}

fn uri(s: &str, require_scheme: bool) -> bool {
    if !s.is_ascii() {
        return false;
    }
    let rest = match s.split_once(':') {
        Some((scheme, rest)) => {
            let mut ch = scheme.chars();
            if !matches!(ch.next(), Some(c) if c.is_ascii_alphabetic()) {
                return false;
            }
            if !ch.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
                return false;
            }
            rest
        }
        None => {
            if require_scheme {
                return false;
            }
            s
        }
    };
    let _ = rest;
    // Forbidden characters anywhere in the reference.
    !s.bytes().any(|c| {
        c <= 0x20
            || c >= 0x7f
            || matches!(
                c,
                b'"' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'{' | b'|' | b'}'
            )
    })
}

fn uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 8 || i == 13 || i == 18 || i == 23 || c.is_ascii_hexdigit())
}

fn json_pointer(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if !s.starts_with('/') {
        return false;
    }
    let b = s.as_bytes();
    let mut i = 1;
    while i < b.len() {
        if b[i] == b'~' {
            match b.get(i + 1) {
                Some(b'0') | Some(b'1') => i += 2,
                _ => return false,
            }
        } else {
            i += 1;
        }
    }
    true
}

/// ISO 8601 duration: `P[n]Y[n]M[n]W[n]D[T[n]H[n]M[n(.f)?S]]` with at least
/// one component, fixed component order, no repeats.
fn duration(s: &str) -> bool {
    let b = s.as_bytes();
    if b.first() != Some(&b'P') {
        return false;
    }
    let mut i = 1;
    let mut any = false;

    // Date components: Y M W D in this exact order.
    let mut last = 0u8;
    while i < b.len() && b[i] != b'T' {
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || i >= b.len() {
            return false;
        }
        let idx = match b[i] {
            b'Y' => 1,
            b'M' => 2,
            b'W' => 3,
            b'D' => 4,
            _ => return false,
        };
        if idx <= last {
            return false; // out of order or repeated
        }
        last = idx;
        i += 1;
        any = true;
    }

    // Time components: H M S in this exact order; S may carry a fraction.
    if i < b.len() {
        if b[i] != b'T' {
            return false;
        }
        i += 1;
        let mut t_any = false;
        let mut t_last = 0u8;
        while i < b.len() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == start {
                return false;
            }
            let mut frac = false;
            if b[i - 1..].len() > 1 && i < b.len() && b[i] == b'.' {
                // fraction only allowed on seconds, checked via unit below
                let fs = i + 1;
                let mut j = fs;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j == fs {
                    return false;
                }
                frac = true;
                i = j;
            }
            if i >= b.len() {
                return false;
            }
            let idx = match b[i] {
                b'H' => 1,
                b'M' => 2,
                b'S' => 3,
                _ => return false,
            };
            if idx <= t_last || (frac && idx != 3) {
                return false;
            }
            t_last = idx;
            i += 1;
            t_any = true;
        }
        if !t_any {
            return false;
        }
        any = true;
    }

    any && i == b.len()
}
