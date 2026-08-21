//! Spectral-style ruleset compilation, dogfooded through suspect-low: the
//! ruleset document is parsed as an ordinary YAML/JSON [`LowDoc`] — no serde
//! layer involved.

use suspect_jsonpath::Path;
use suspect_low::{LowDoc, NodeRef, ValueKind};

use crate::engine::{Linter, RulesetError};
use crate::functions::{Casing, EnumValue, Function};
use crate::rule::{FamilySet, Rule, Severity};

/// Compiles a ruleset document into a [`Linter`].
///
/// # Errors
/// See [`RulesetError`].
pub fn compile(doc: &LowDoc) -> Result<Linter, RulesetError> {
    let root = doc.root();
    if root.kind() != ValueKind::Object {
        return Err(RulesetError::InvalidRuleset {
            field: "(root)".into(),
            message: "ruleset must be a mapping".into(),
        });
    }

    let mut rules: Vec<Rule> = Vec::new();
    if let Some(extends) = root.get("extends") {
        for target in extend_targets(&extends) {
            let base = match target {
                "spectral:oas" => crate::packs::oas::rules(),
                "spectral:overlay" => crate::packs::overlay_arazzo::overlay_rules(),
                "spectral:arazzo" => crate::packs::overlay_arazzo::arazzo_rules(),
                other => {
                    return Err(RulesetError::InvalidRuleset {
                        field: "extends".into(),
                        message: format!("unknown ruleset target `{other}`"),
                    });
                }
            };
            for rule in base {
                if !rules.iter().any(|r| r.code == rule.code) {
                    rules.push(rule);
                }
            }
        }
    }

    if let Some(rules_node) = root.get("rules") {
        if rules_node.resolved().kind() != ValueKind::Object {
            return Err(RulesetError::InvalidRuleset {
                field: "rules".into(),
                message: "`rules` must be a mapping of code to rule".into(),
            });
        }
        for entry in rules_node.entries() {
            let rule = parse_rule(entry.key, entry.value)?;
            // A user rule with the same code overrides any extended builtin.
            rules.retain(|r| r.code != rule.code);
            rules.push(rule);
        }
    }

    Ok(Linter::from_rules(rules))
}

fn extend_targets<'d>(node: &NodeRef<'d>) -> Vec<&'d str> {
    let resolved = node.resolved();
    match resolved.kind() {
        ValueKind::Str => resolved.as_str().into_iter().collect(),
        ValueKind::Array => resolved
            .items()
            .into_iter()
            .filter_map(|item| item.resolved().as_str())
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_rule(code: &str, node: Option<NodeRef<'_>>) -> Result<Rule, RulesetError> {
    let Some(node) = node else {
        return Err(bad_rule(code, "rule must be a mapping"));
    };
    let resolved = node.resolved();
    if resolved.kind() != ValueKind::Object {
        return Err(bad_rule(code, "rule must be a mapping"));
    }

    let description = resolved
        .get("description")
        .and_then(|d| d.resolved().as_str())
        .map(Box::<str>::from);

    let severity = match resolved.get("severity") {
        None => Severity::Warn,
        Some(sev) => {
            let sev = sev.resolved();
            match sev.kind() {
                ValueKind::Str => {
                    let text = sev.as_str().unwrap_or_default();
                    Severity::from_text(text)
                        .ok_or_else(|| bad_rule(code, format!("unknown severity `{text}`")))?
                }
                ValueKind::Int | ValueKind::Float => {
                    let n = sev.as_i64().ok_or_else(|| bad_rule(code, "severity must be 0-3"))?;
                    Severity::from_number(n)
                        .ok_or_else(|| bad_rule(code, format!("severity {n} out of range 0-3")))?
                }
                _ => return Err(bad_rule(code, "severity must be a string or number")),
            }
        }
    };

    let formats = match resolved.get("formats") {
        None => FamilySet::ALL,
        Some(fmts) => {
            let fmts = fmts.resolved();
            if fmts.kind() != ValueKind::Array {
                return Err(bad_rule(code, "`formats` must be an array of strings"));
            }
            let mut set = FamilySet::NONE;
            for item in fmts.items() {
                let Some(token) = item.resolved().as_str() else {
                    return Err(bad_rule(code, "`formats` entries must be strings"));
                };
                let Some(bit) = FamilySet::from_format_token(token) else {
                    return Err(bad_rule(code, format!("unknown format `{token}`")));
                };
                set = set.union(bit);
            }
            set
        }
    };

    let given = match resolved.get("given") {
        None => vec![compile_path(code, "$")?],
        Some(g) => {
            let g = g.resolved();
            match g.kind() {
                ValueKind::Str => vec![compile_path(code, g.as_str().unwrap_or_default())?],
                ValueKind::Array => {
                    let mut paths = Vec::new();
                    for item in g.items() {
                        let Some(s) = item.resolved().as_str() else {
                            return Err(bad_rule(code, "`given` entries must be strings"));
                        };
                        paths.push(compile_path(code, s)?);
                    }
                    paths
                }
                _ => return Err(bad_rule(code, "`given` must be a string or array of strings")),
            }
        }
    };

    let Some(then) = resolved.get("then") else {
        return Err(bad_rule(code, "rule is missing `then`"));
    };
    let then = then.resolved();
    let Some(function) = then.get("function") else {
        return Err(bad_rule(code, "`then` is missing `function`"));
    };
    let name = function
        .resolved()
        .as_str()
        .ok_or_else(|| bad_rule(code, "`function` must be a string"))?;
    let options = then.get("functionOptions");
    let then = compile_function(code, name, options)?;

    Ok(Rule {
        code: code.into(),
        description,
        given,
        then,
        severity,
        formats,
    })
}

fn compile_path(_code: &str, query: &str) -> Result<Path, suspect_jsonpath::PathError> {
    Path::parse(query)
}

fn compile_function(code: &str, name: &str, options: Option<NodeRef<'_>>) -> Result<Function, RulesetError> {
    let opts = options.map(|o| o.resolved());
    let opt_str = |key: &str| opts.as_ref().and_then(|o| o.get(key)).and_then(|v| v.resolved().as_str());

    let function = match name {
        "truthy" => Function::Truthy,
        "falsy" => Function::Falsy,
        "defined" => Function::Defined {
            property: required_property(code, &opts)?,
        },
        "undefined" => Function::Undefined {
            property: required_property(code, &opts)?,
        },
        "pattern" => {
            let Some(pattern) = opt_str("match") else {
                return Err(bad_rule(code, "`pattern` requires functionOptions.match"));
            };
            let re = regex::Regex::new(pattern).map_err(|e| bad_rule(code, format!("invalid regex `{pattern}`: {e}")))?;
            Function::Pattern(re)
        }
        "casing" => {
            let Some(casing) = opt_str("casing") else {
                return Err(bad_rule(code, "`casing` requires functionOptions.casing"));
            };
            let casing = Casing::from_text(casing).ok_or_else(|| bad_rule(code, format!("unknown casing `{casing}`")))?;
            Function::Casing(casing)
        }
        "length" => {
            let get_num = |key: &str| {
                opts.as_ref()
                    .and_then(|o| o.get("length"))
                    .and_then(|l| l.resolved().get(key))
                    .and_then(|n| n.resolved().as_f64())
            };
            Function::Length { min: get_num("min"), max: get_num("max") }
        }
        "enumeration" => {
            let Some(values) = opts.as_ref().and_then(|o| o.get("values")) else {
                return Err(bad_rule(code, "`enumeration` requires functionOptions.values"));
            };
            let resolved = values.resolved();
            if resolved.kind() != ValueKind::Array {
                return Err(bad_rule(code, "`values` must be an array"));
            }
            let mut list = Vec::new();
            for item in resolved.items() {
                let item = item.resolved();
                let value = match item.kind() {
                    ValueKind::Str => EnumValue::Str(item.as_str().unwrap_or_default().into()),
                    ValueKind::Bool => EnumValue::Bool(item.as_bool().unwrap_or_default()),
                    ValueKind::Int | ValueKind::Float => EnumValue::Num(item.as_f64().unwrap_or_default()),
                    ValueKind::Null => EnumValue::Null,
                    ValueKind::Object | ValueKind::Array => {
                        return Err(bad_rule(code, "`values` entries must be scalars"));
                    }
                };
                list.push(value);
            }
            Function::Enumeration(list)
        }
        "alphabetical" => Function::Alphabetical,
        "xor" => {
            let Some(properties) = opts.as_ref().and_then(|o| o.get("properties")) else {
                return Err(bad_rule(code, "`xor` requires functionOptions.properties"));
            };
            let resolved = properties.resolved();
            if resolved.kind() != ValueKind::Array {
                return Err(bad_rule(code, "`properties` must be an array of strings"));
            }
            let mut list = Vec::new();
            for item in resolved.items() {
                let Some(p) = item.resolved().as_str() else {
                    return Err(bad_rule(code, "`properties` entries must be strings"));
                };
                list.push(p.into());
            }
            Function::Xor { properties: list }
        }
        "path-params" => Function::PathParams,
        "ref-siblings" => Function::RefSiblings,
        "typed-enum" => Function::TypedEnum,
        "duplicate-keys" => Function::DuplicateKeys,
        "default-response" => Function::DefaultResponse,
        "success-response" => Function::SuccessResponse,
        "path-keys-no-trailing-slash" => Function::NoTrailingSlash,
        other => return Err(bad_rule(code, format!("unknown function `{other}`"))),
    };
    Ok(function)
}

fn required_property(code: &str, opts: &Option<NodeRef<'_>>) -> Result<Box<str>, RulesetError> {
    let property = opts
        .as_ref()
        .and_then(|o| o.get("property"))
        .and_then(|p| p.resolved().as_str());
    match property {
        Some(p) if !p.is_empty() => Ok(p.into()),
        _ => Err(bad_rule(code, "functionOptions.property is required")),
    }
}

fn bad_rule(code: &str, message: impl std::fmt::Display) -> RulesetError {
    RulesetError::BadRule {
        code: code.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests;
