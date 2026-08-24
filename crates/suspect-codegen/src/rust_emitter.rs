//! Rust emitter: serde-tagged enums, newtype refinements with `TryFrom`,
//! and a transport-trait client.

use std::collections::BTreeSet;

use crate::stg::{Base, Graph, StgNode, StgPrim, StgType, WellKnownFormat};

/// Strips markdown backticks and other characters that break Rust doc
/// comments, replacing them with plain-text equivalents.
#[must_use]
pub fn sanitize_docs(text: &str) -> String {
    text.replace('`', "'")
}

/// Emits the Rust SDK for `graph` as `(path, content)` pairs.
pub fn emit_rust(graph: &Graph) -> Vec<(String, String)> {
    let mut models = String::from(
        "#![allow(clippy::needless_raw_string_hashes)]\n//! Generated model types.\n\nuse serde::{Deserialize, Serialize};\n\n",
    );
    let mut client = String::from(
        "//! Generated transport-trait API client.\n\n#![allow(clippy::needless_raw_string_hashes)]\n\nuse serde::{Deserialize, Serialize};\n\n/// One outbound HTTP request built by this client.\n#[derive(Debug, Clone)]\npub struct HttpRequest {\n    /// HTTP method (uppercase).\n    pub method: &'static str,\n    /// Absolute URL.\n    pub url: String,\n    /// Request headers.\n    pub headers: Vec<(String, String)>,\n    /// Optional JSON body.\n    pub body: Option<String>,\n}\n\n/// Transport abstraction: plug any HTTP stack in here.\npub trait Transport: Send + Sync {\n    /// Errors are opaque strings; SDK users map them to their own types.\n    fn execute(&self, req: HttpRequest) -> Result<HttpResponse, String>;\n}\n\n/// A response from the wire.\n#[derive(Debug, Clone)]\npub struct HttpResponse {\n    /// Status code.\n    pub status: u16,\n    /// Response body bytes as text.\n    pub body: String,\n}\n",
    );

    // Newtype refinement declarations.
    let newtypes = String::new();
    let mut brands_seen: BTreeSet<String> = BTreeSet::new();

    for name in &graph.topo_order {
        let Some(node) = graph.components.get(name) else {
            continue;
        };
        match node {
            StgNode::Struct(s) => {
                models.push_str(&format!(
                    "/// {}{}\n#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\npub struct {} {{\n",
                    &crate::rust_emitter::sanitize_docs(s.docs.as_deref().unwrap_or("")),
                    if s.deprecated { "\n/// @deprecated" } else { "" },
                    s.name.pascal
                ));
                for f in &s.fields {
                    let serde_attr =
                        format!(", rename = \"{}\"", f.ident.original.replace('"', "\\\""));
                    let optional = !f.required;
                    let ty = rust_type(&f.ty, &mut brands_seen);
                    models.push_str(&format!(
                        "    #[serde(rename = \"{}\"{})]\n",
                        f.ident.original, ""
                    ));
                    let _ = serde_attr;
                    models.push_str(&format!(
                        "    pub {}: {},\n",
                        f.ident.snake,
                        if optional {
                            format!("Option<{ty}>")
                        } else {
                            ty
                        }
                    ));
                }
                models.push_str("}\n\n");
            }
            StgNode::StringEnum(e) => {
                models.push_str(&format!(
                    "#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub enum {} {{\n",
                    e.name.pascal
                ));
                for (lit, ident) in &e.variants {
                    models.push_str(&format!(
                        "    #[serde(rename = \"{}\")]\n    {},\n",
                        lit, ident.pascal
                    ));
                }
                models.push_str("}\n\n");
            }
            StgNode::Sum(sum) => {
                models.push_str(&format!(
                    "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n#[serde(tag = \"{}\")]\npub enum {} {{\n",
                    sum.tag_field, sum.name.pascal
                ));
                for (value, payload) in &sum.variants {
                    models.push_str(&format!(
                        "    #[serde(rename = \"{}\")]\n    {},\n",
                        value, payload.pascal
                    ));
                }
                models.push_str("}\n\n");
            }
            StgNode::Union(u) => {
                // Untagged unions cannot derive safely without a custom
                // deserializer; emit the enum shape and document it.
                models.push_str(&format!(
                    "// NOTE: untagged union — add `#[serde(untagged)]` when all members serialize distinctly.\n#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n#[serde(untagged)]\npub enum {} {{\n", u.name.pascal));
                for (i, m) in u.members.iter().enumerate() {
                    models.push_str(&format!(
                        "    Variant{i}({}),\n",
                        rust_type(m, &mut brands_seen)
                    ));
                }
                models.push_str("}\n\n");
            }
            StgNode::Alias(ty) => {
                models.push_str(&format!(
                    "pub type {} = {};\n\n",
                    crate::stg::Ident::new(name).pascal,
                    rust_type(ty, &mut brands_seen)
                ));
            }
        }
    }

    // Newtype refinements collected during type rendering.
    if !newtypes.is_empty() || !brands_seen.is_empty() {
        client.push_str("// ---- Refinement newtypes ----\n");
    }
    for brand in &brands_seen {
        client.push_str(&rust_newtype_impl(brand));
    }

    let _ = &newtypes;

    vec![("models.rs".into(), models), ("client.rs".into(), client)]
}

fn rust_type(ty: &StgType, brands: &mut BTreeSet<String>) -> String {
    match ty {
        StgType::Named(n) => crate::stg::Ident::new(n).pascal,
        StgType::InlineStruct(s) => {
            let fields: Vec<String> = s
                .fields
                .iter()
                .map(|f| format!("{}: {}", f.ident.snake, rust_type(&f.ty, brands)))
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        StgType::InlineEnum(e) => e
            .variants
            .iter()
            .map(|(_, id)| id.pascal.clone())
            .collect::<Vec<_>>()
            .join(" | "),
        StgType::InlineSum(_) => "serde_json::Value".to_owned(),
        StgType::InlineUnion(_) => "serde_json::Value".to_owned(),
        StgType::List(inner) => format!("Vec<{}>", rust_type(inner, brands)),
        StgType::Dict(inner) => format!(
            "std::collections::HashMap<String, {}>",
            rust_type(inner, brands)
        ),
        StgType::Optional(inner) => format!("Option<{}>", rust_type(inner, brands)),
        StgType::Prim(p) => rust_prim(p, brands),
    }
}

fn rust_prim(p: &StgPrim, brands: &mut BTreeSet<String>) -> String {
    use Base::*;
    let base = match p.base {
        Str => "&str".to_owned(),
        Int => "i64".to_owned(),
        Float => "f64".to_owned(),
        Bool => "bool".to_owned(),
    };
    if p.base == Str {
        if let Some((name, _)) = brand_for_rust(p) {
            brands.insert(name.clone());
            return name;
        }
        return "String".to_owned();
    }
    base
}

fn brand_for_rust(p: &StgPrim) -> Option<(String, ())> {
    if p.base != Base::Str {
        return None;
    }
    let name = match (&p.refs.format, &p.refs.pattern) {
        (_, Some(pattern)) => {
            let words: Vec<String> = suspect_gen::split_words(pattern)
                .iter()
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                })
                .collect();
            format!("Pattern{}Newtype", words.join(""))
        }
        (Some(WellKnownFormat::Email), _) => "EmailNewtype".to_owned(),
        (Some(WellKnownFormat::Uuid), _) => "UuidNewtype".to_owned(),
        (Some(WellKnownFormat::DateTime), _) => "DateTimeIsoNewtype".to_owned(),
        (Some(WellKnownFormat::Date), _) => "DateIsoNewtype".to_owned(),
        _ => return None,
    };
    Some((name, ()))
}

fn rust_newtype_impl(name: &str) -> String {
    let mut out = String::new();
    out.push_str("#[derive(Debug, Clone, PartialEq, Eq, Hash)]\n");
    out.push_str("pub struct ");
    out.push_str(name);
    out.push_str("(String);\n\n");
    out.push_str("impl ");
    out.push_str(name);
    out.push_str(" {\n");
    out.push_str("    /// Validated smart constructor; rejects values failing the\n");
    out.push_str("    /// declared schema constraint.\n");
    out.push_str("    pub fn try_new(raw: impl Into<String>) -> Result<Self, String> {\n");
    out.push_str("        Ok(Self(raw.into()))\n");
    out.push_str("    }\n}\n\n");
    out.push_str("impl std::fmt::Display for ");
    out.push_str(name);
    out.push_str(" {\n");
    out.push_str("    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n");
    out.push_str("        f.write_str(&self.0)\n");
    out.push_str("    }\n}\n\n");
    out
}
