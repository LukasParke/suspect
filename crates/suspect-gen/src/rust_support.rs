//! Rust source spelling helpers for template filters.

/// Quotes text as a Rust string literal without changing its decoded value.
#[must_use]
pub fn rust_string(text: &str) -> String {
    format!("{text:?}")
}

/// Builds an ASCII Cargo package name from an API display title.
///
/// Non-ASCII letters use explicit code-point segments; an empty title uses
/// `generated-api-sdk`, and digit-leading names gain an `api-` prefix. The
/// original title remains available in generated documentation.
#[must_use]
pub fn rust_package_name(title: &str) -> String {
    let mut segments = Vec::new();
    let mut ascii = String::new();
    for character in crate::filters::to_kebab_case(title).chars() {
        if character.is_ascii_alphanumeric() {
            ascii.push(character);
        } else {
            if !ascii.is_empty() {
                segments.push(std::mem::take(&mut ascii));
            }
            if character != '-' {
                segments.push(format!("u{:x}", character as u32));
            }
        }
    }
    if !ascii.is_empty() {
        segments.push(ascii);
    }
    let mut name = segments.join("-");
    if name.is_empty() {
        name.push_str("generated-api");
    } else if name.as_bytes()[0].is_ascii_digit() {
        name.insert_str(0, "api-");
    }
    name.push_str("-sdk");
    name
}

/// Makes an already case-converted name a legal Rust identifier.
///
/// This handles syntax only; allocating distinct names after normalization is
/// a separate language-planning concern. Wire names must be kept separately.
#[must_use]
pub fn rust_identifier(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        if character == '_' || unicode_ident::is_xid_continue(character) {
            if out.is_empty() && character != '_' && !unicode_ident::is_xid_start(character) {
                out.push('_');
            }
            out.push(character);
        } else {
            out.push_str(&format!("_u{:x}_", character as u32));
        }
    }
    if out.is_empty() || out == "_" {
        return "_generated".into();
    }
    if matches!(
        out.as_str(),
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" |
        "dyn" | "else" | "enum" | "extern" | "false" | "fn" | "for" | "if" |
        "impl" | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" |
        "pub" | "ref" | "return" | "self" | "Self" | "static" | "struct" |
        "super" | "trait" | "true" | "type" | "unsafe" | "use" | "where" |
        "while" | "abstract" | "become" | "box" | "do" | "final" | "gen" |
        "macro" | "override" | "priv" | "try" | "typeof" | "unsized" |
        "virtual" | "yield" | "union" | "macro_rules" |
        // Common constructor, local-variable, and body parameter names.
        "new" | "url" | "parts" | "body"
    ) {
        out.push('_');
    }
    out
}
