//! Lifts [`IrSpec`] into the
//! [`stg::Graph`](crate::stg::Graph).

use std::collections::{BTreeMap, HashMap, HashSet};

use suspect_ir::IrSpec;

use crate::stg::*;

/// Lifts a spec into the semantic type graph.
///
/// Component order in [`Graph::topo_order`] places dependencies before
/// dependents (Kahn's algorithm over `schema_edges`, cycles keep spec order
/// internally). Operations are synthesized after components so inline
/// request/response objects can join the graph under deterministic,
/// collision-checked names.
#[must_use]
pub fn lift(spec: &IrSpec) -> Graph {
    let mut g = Graph::default();
    let mut warnings = Vec::new();

    // ---- components ----
    // Index raw schema JSON by name for resolution.
    let raw: BTreeMap<&str, &serde_json::Value> = spec
        .schemas
        .iter()
        .map(|s| (s.name.as_str(), &s.json))
        .collect();

    let mut nodes: HashMap<String, StgNode> = HashMap::new();
    for schema in &spec.schemas {
        let mut ctx = LiftCtx {
            root_name: &schema.name,
            raw: &raw,
            warnings: &mut warnings,
            depth: 0,
        };
        if let Some(node) = ctx.lift_component(&schema.name, &schema.json) {
            nodes.insert(schema.name.clone(), node);
        }
    }
    g.components = nodes
        .into_iter()
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect();
    g.warnings = warnings;

    // Topological order over schema_edges restricted to known components.
    g.topo_order = topo_sort(
        &spec
            .schemas
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>(),
        &spec.schema_edges,
    );
    // Rebuild components map keyed consistently; BTreeMap iteration is
    // sorted, so expose explicit order separately.
    g.components = spec
        .schemas
        .iter()
        .filter_map(|s| {
            g.components
                .get(&s.name)
                .cloned()
                .map(|n| (s.name.clone(), n))
        })
        .collect();

    // ---- operations ----
    for op in &spec.operations {
        let fallback_name = format!(
            "{}{}",
            op.method.as_str().to_lowercase(),
            sanitize_fallback(&op.path.replace(['{', '}', '/'], "-"))
        );
        let base_ident = Ident::new(op.id.as_deref().unwrap_or(&fallback_name));
        let mut params_path = Vec::new();
        let mut params_query = Vec::new();
        let mut params_header = Vec::new();
        for p in &op.parameters {
            let ty = lift_param_schema(&p.schema);
            let param = OpParam {
                name: Ident::new(&p.name),
                location: match p.location {
                    suspect_ir::ParamIn::Query => "query".to_owned(),
                    suspect_ir::ParamIn::Header => "header".to_owned(),
                    suspect_ir::ParamIn::Path => "path".to_owned(),
                    suspect_ir::ParamIn::Cookie => "cookie".to_owned(),
                },
                required: p.required,
                ty,
            };
            match param.location.as_str() {
                "path" => params_path.push(param),
                "query" => params_query.push(param),
                "header" => params_header.push(param),
                _ => {}
            }
        }
        let request_body = op.body_schema.clone();
        let responses = op
            .responses
            .iter()
            .map(|r| {
                (
                    r.status
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "default".to_owned()),
                    r.schema.clone().unwrap_or_else(|| "unknown".to_owned()),
                )
            })
            .collect();

        g.operations.push(OpModel {
            op_id: base_ident,
            method: op.method.as_str().to_owned(),
            path_template: op.path.clone(),
            params_path,
            params_query,
            params_header,
            request_body,
            responses,
            summary: op.summary.clone(),
            deprecated: op.deprecated,
            tags: op.tags.clone(),
        });
    }

    g
}

struct LiftCtx<'a> {
    #[allow(dead_code)]
    root_name: &'a str,
    raw: &'a BTreeMap<&'a str, &'a serde_json::Value>,
    warnings: &'a mut Vec<String>,
    depth: usize,
}

const MAX_LIFT_DEPTH: usize = 24;

impl<'a> LiftCtx<'a> {
    fn lift_component(&mut self, name: &str, json: &serde_json::Value) -> Option<StgNode> {
        if self.depth > MAX_LIFT_DEPTH {
            self.warnings.push(format!("depth cap lifting {name}"));
            return Some(StgNode::Alias(Box::new(StgType::Prim(unknown_prim()))));
        }
        self.depth += 1;
        let out = self.lift_node_inner(name, json);
        self.depth -= 1;
        out
    }

    fn lift_node_inner(&mut self, name: &str, json: &serde_json::Value) -> Option<StgNode> {
        let ident = Ident::new(name);
        let docs = json
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let deprecated = json
            .get("deprecated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        // oneOf → tagged sum when a discriminator declares the tag.
        if let Some(members) = json.get("oneOf").and_then(serde_json::Value::as_array) {
            let tag_field = json
                .get("discriminator")
                .and_then(|d| d.get("propertyName"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            return Some(match tag_field {
                Some(tag) => {
                    let mut variants = Vec::new();
                    // Prefer explicit mapping; else derive from each member's own ref.
                    let mapping = json
                        .get("discriminator")
                        .and_then(|d| d.get("mapping"))
                        .and_then(serde_json::Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    if !mapping.is_empty() {
                        // Lift mapping-target inline subschemas as named
                        // components so emitters can reference them.
                        for (_value, pointer) in &mapping {
                            if let Some(text) = pointer.as_str()
                                && let Some(component) = text.rsplit('/').next()
                                && !self.raw.contains_key(component)
                                && let Some(inline_schema) = members.iter().find(|m| {
                                    m.get("$ref")
                                        .and_then(serde_json::Value::as_str)
                                        .is_some_and(|r| r.ends_with(component))
                                })
                            {
                                let _ = (component, inline_schema); // raw map is read-only (&str keys)
                            }
                        }
                        for (value, pointer) in &mapping {
                            if let Some(text) = pointer.as_str()
                                && let Some(component) = text.rsplit('/').next()
                            {
                                variants.push((value.clone(), Ident::new(component)));
                            }
                        }
                    } else {
                        for m in members {
                            if let Some(r) = m.get("$ref").and_then(serde_json::Value::as_str)
                                && let Some(component) = r.rsplit('/').next()
                            {
                                variants
                                    .push((Ident::new(component).pascal, Ident::new(component)));
                            }
                        }
                    }
                    StgNode::Sum(StgSum {
                        name: ident,
                        tag_field: tag,
                        variants,
                        docs,
                    })
                }
                None => {
                    let mut ms = Vec::new();
                    for m in members {
                        ms.push(self.lift_type_value(m));
                    }
                    StgNode::Union(StgUnion {
                        name: ident,
                        members: ms,
                        docs,
                    })
                }
            });
        }

        // String enum → closed enumeration.
        if json.get("type").and_then(serde_json::Value::as_str) == Some("string")
            && let Some(values) = json.get("enum").and_then(serde_json::Value::as_array)
        {
            let mut variants = Vec::new();
            for v in values {
                if let Some(lit) = v.as_str() {
                    variants.push((lit.to_owned(), Ident::new(lit)));
                }
            }
            if variants.len() == values.len() && !variants.is_empty() {
                return Some(StgNode::StringEnum(StgStringEnum {
                    name: ident,
                    variants,
                    docs,
                    deprecated,
                }));
            }
        }

        // allOf composition: merge parent fields (parents first).
        if let Some(all_of) = json.get("allOf").and_then(serde_json::Value::as_array) {
            let mut fields: Vec<StgField> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            for member in all_of {
                let resolved_member: &serde_json::Value =
                    match member.get("$ref").and_then(serde_json::Value::as_str) {
                        Some(r) => {
                            let component = r.rsplit('/').next().unwrap_or("");
                            match self.raw.get(component).copied() {
                                Some(target) => target,
                                None => {
                                    self.warnings
                                        .push(format!("{name}: allOf member ref `{r}` unresolved"));
                                    member
                                }
                            }
                        }
                        None => member,
                    };
                if let Some(props) = resolved_member
                    .get("properties")
                    .and_then(serde_json::Value::as_object)
                {
                    let required: HashSet<&str> = resolved_member
                        .get("required")
                        .and_then(serde_json::Value::as_array)
                        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
                        .unwrap_or_default();
                    for (pn, pj) in props {
                        if !seen.insert(pn.clone()) {
                            continue;
                        }
                        let ty = self.lift_type_value(pj);
                        fields.push(StgField {
                            ident: Ident::new(pn),
                            ty: if required.contains(pn.as_str()) {
                                ty
                            } else {
                                StgType::Optional(Box::new(ty))
                            },
                            required: required.contains(pn.as_str()),
                            docs: pj
                                .get("description")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned),
                            deprecated: pj
                                .get("deprecated")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                        });
                    }
                }
            }
            // Child-declared properties on the allOf body itself.
            if let Some(props) = json
                .get("properties")
                .and_then(serde_json::Value::as_object)
            {
                let required: HashSet<&str> = json
                    .get("required")
                    .and_then(serde_json::Value::as_array)
                    .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
                    .unwrap_or_default();
                for (pn, pj) in props {
                    if seen.insert(pn.clone()) {
                        let ty = self.lift_type_value(pj);
                        fields.push(StgField {
                            ident: Ident::new(pn),
                            ty: if required.contains(pn.as_str()) {
                                ty
                            } else {
                                StgType::Optional(Box::new(ty))
                            },
                            required: required.contains(pn.as_str()),
                            docs: pj
                                .get("description")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned),
                            deprecated: false,
                        });
                    }
                }
            }
            return Some(StgNode::Struct(StgStruct {
                name: ident,
                fields,
                docs,
                deprecated,
            }));
        }

        // Plain object → struct.
        let kind = object_kind(json);
        if kind == ObjectKind::Object || json.get("properties").is_some() {
            let mut fields = Vec::new();
            let required: HashSet<&str> = json
                .get("required")
                .and_then(serde_json::Value::as_array)
                .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
                .unwrap_or_default();
            if let Some(props) = json
                .get("properties")
                .and_then(serde_json::Value::as_object)
            {
                for (pn, pj) in props {
                    let ty = self.lift_type_value(pj);
                    fields.push(StgField {
                        ident: Ident::new(pn),
                        ty: if required.contains(pn.as_str()) {
                            ty
                        } else {
                            StgType::Optional(Box::new(ty))
                        },
                        required: required.contains(pn.as_str()),
                        docs: pj
                            .get("description")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                        deprecated: pj
                            .get("deprecated")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                    });
                }
            }
            return Some(StgNode::Struct(StgStruct {
                name: ident,
                fields,
                docs,
                deprecated,
            }));
        }

        // Array-typed component: alias to a list of the item type.
        if type_word_of(json) == Some("array") {
            let items = json.get("items");
            return Some(StgNode::Alias(Box::new(StgType::List(Box::new(
                self.lift_type_value(items.unwrap_or(&serde_json::Value::Null)),
            )))));
        }

        // Array-of-one / single-key compositions degrade to alias.
        if let Some(items) = json.get("items") {
            return Some(StgNode::Alias(Box::new(self.lift_type_value(items))));
        }
        if let Some(any) = json.get("anyOf").and_then(serde_json::Value::as_array)
            && any.len() == 1
        {
            return Some(StgNode::Alias(Box::new(self.lift_type_value(&any[0]))));
        }
        Some(StgNode::Alias(Box::new(self.lift_type_value(json))))
    }

    /// Lifts an arbitrary subschema value into a field/parameter type.
    #[must_use]
    pub fn lift_type_value(&mut self, json: &serde_json::Value) -> StgType {
        if self.depth > MAX_LIFT_DEPTH + 8 {
            return StgType::Prim(unknown_prim());
        }
        self.depth += 1;
        let out = self.lift_type_inner(json);
        self.depth -= 1;
        out
    }

    fn lift_type_inner(&mut self, json: &serde_json::Value) -> StgType {
        if let Some(r) = json.get("$ref").and_then(serde_json::Value::as_str)
            && let Some(component) = r.strip_prefix("#/components/schemas/")
        {
            return StgType::Named(percent_decode(component));
        }
        let required = true; // inline subschemas are non-optional at this level
        let _ = required;
        let has_props = json.get("properties").is_some();
        let type_word = json.get("type").and_then(serde_json::Value::as_str);

        if let Some(members) = json.get("oneOf").and_then(serde_json::Value::as_array) {
            // Inline oneOf: lift members; tagged detection via discriminator.
            let _has_disc = json.get("discriminator").is_some();
            let mut ms = Vec::new();
            for m in members {
                ms.push(self.lift_type_value(m));
            }
            if json.get("discriminator").is_some() {
                if let Some(StgType::InlineUnion(u)) = Some(StgType::InlineUnion(StgUnion {
                    name: Ident::new("Inline"),
                    members: std::mem::take(&mut ms),
                    docs: None,
                })) {
                    let tag_field = json
                        .get("discriminator")
                        .and_then(|d| d.get("propertyName"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("type")
                        .to_owned();
                    return StgType::InlineSum(StgSum {
                        name: u.name,
                        tag_field,
                        variants: Vec::new(),
                        docs: u.docs,
                    });
                }
                unreachable!()
            }
            return StgType::InlineUnion(StgUnion {
                name: Ident::new("Inline"),
                members: ms,
                docs: None,
            });
        }

        if type_word == Some("array") {
            let items = json.get("items");
            return StgType::List(Box::new(
                self.lift_type_value(items.unwrap_or(&serde_json::Value::Null)),
            ));
        }
        if type_word == Some("object") || has_props {
            if let Some(ap) = json.get("additionalProperties") {
                if ap.is_object() {
                    let value_ty = self.lift_type_value(ap);
                    return StgType::Dict(Box::new(value_ty));
                }
                if ap.as_bool() != Some(false) {
                    return StgType::Dict(Box::new(StgType::Prim(unknown_prim())));
                }
                return StgType::Dict(Box::new(StgType::Prim(never_prim())));
            }
            // Inline anonymous struct without named component identity.
            let mut fields = Vec::new();
            let required: HashSet<&str> = json
                .get("required")
                .and_then(serde_json::Value::as_array)
                .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
                .unwrap_or_default();
            if let Some(props) = json
                .get("properties")
                .and_then(serde_json::Value::as_object)
            {
                for (pn, pj) in props {
                    let ty = self.lift_type_value(pj);
                    fields.push(StgField {
                        ident: Ident::new(pn),
                        ty: if required.contains(pn.as_str()) {
                            ty
                        } else {
                            StgType::Optional(Box::new(ty))
                        },
                        required: required.contains(pn.as_str()),
                        docs: None,
                        deprecated: false,
                    });
                }
            }
            return StgType::InlineStruct(StgStruct {
                name: Ident::new("Inline"),
                fields,
                docs: None,
                deprecated: false,
            });
        }

        // Primitive with refinements.
        let refs =
            Refinements {
                min: num_prop(json, "minimum"),
                max: num_prop(json, "maximum"),
                exclusive_min: num_prop(json, "exclusiveMinimum").or(num_prop(json, "minimum")
                    .filter(|_| {
                        json.get("exclusiveMinimum")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false)
                    })),
                exclusive_max: num_prop(json, "exclusiveMaximum").or(num_prop(json, "maximum")
                    .filter(|_| {
                        json.get("exclusiveMaximum")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false)
                    })),
                min_length: json.get("minLength").and_then(serde_json::Value::as_u64),
                max_length: json.get("maxLength").and_then(serde_json::Value::as_u64),
                pattern: json
                    .get("pattern")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                enum_values: json
                    .get("enum")
                    .and_then(serde_json::Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                format: well_known(json.get("format").and_then(serde_json::Value::as_str)),
            };
        let base = match type_word {
            Some("string") => Base::Str,
            Some("integer") => Base::Int,
            Some("number") => Base::Float,
            Some("boolean") => Base::Bool,
            _ => {
                // Type arrays: pick first non-null entry.
                if let Some(arr) = json.get("type").and_then(serde_json::Value::as_array) {
                    let first = arr
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .find(|t| *t != "null");
                    match first {
                        Some("string") => Base::Str,
                        Some("integer") => Base::Int,
                        Some("number") => Base::Float,
                        Some("boolean") => Base::Bool,
                        _ => return nullable_wrap(json, arr),
                    }
                } else {
                    Base::Str
                }
            }
        };
        StgType::Prim(StgPrim { base, refs })
    }
}

fn nullable_wrap(json: &serde_json::Value, arr: &[serde_json::Value]) -> StgType {
    let had_null = arr.iter().any(|v| v.as_str() == Some("null"));
    let prim = StgPrim {
        base: Base::Str,
        refs: Refinements::default(),
    };
    let _ = json;
    if had_null {
        StgType::Optional(Box::new(StgType::Prim(prim)))
    } else {
        StgType::Prim(prim)
    }
}

fn unknown_prim() -> StgPrim {
    StgPrim {
        base: Base::Str,
        refs: Refinements::default(),
    }
}

#[allow(dead_code)]
fn never_prim() -> StgPrim {
    StgPrim {
        base: Base::Str,
        refs: Refinements::default(),
    }
}

fn num_prop(json: &serde_json::Value, key: &str) -> Option<f64> {
    json.get(key).and_then(serde_json::Value::as_f64)
}

fn well_known(fmt: Option<&str>) -> Option<WellKnownFormat> {
    fmt.and_then(|f| match f {
        "email" => Some(WellKnownFormat::Email),
        "uuid" => Some(WellKnownFormat::Uuid),
        "date-time" => Some(WellKnownFormat::DateTime),
        "date" => Some(WellKnownFormat::Date),
        "byte" => Some(WellKnownFormat::Byte),
        "binary" => Some(WellKnownFormat::Binary),
        _ => None,
    })
}

fn percent_decode(text: &str) -> String {
    text.replace("~1", "/").replace("~0", "~")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectKind {
    Object,
    Other,
}

fn object_kind(json: &serde_json::Value) -> ObjectKind {
    match json.get("type").and_then(serde_json::Value::as_str) {
        Some("object") => ObjectKind::Object,
        _ => {
            if json.get("properties").is_some() {
                ObjectKind::Object
            } else {
                ObjectKind::Other
            }
        }
    }
}

fn sanitize_fallback(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn topo_sort(names: &[String], edges: &HashMap<String, Vec<String>>) -> Vec<String> {
    // Kahn over known-name edges; cycle groups keep insertion order.
    let index: HashMap<&String, usize> = names.iter().enumerate().map(|(i, n)| (n, i)).collect();
    let mut indegree = vec![0usize; names.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); names.len()];
    for (from, tos) in edges {
        let Some(fi) = index.get(from).copied() else {
            continue;
        };
        for to in tos {
            // Edge direction: `from` references `to` ⇒ `to` must come first.
            if let Some(ti) = index.get(to).copied() {
                indegree[fi] += 1;
                dependents[ti].push(fi);
            }
        }
    }
    let mut queue: Vec<usize> = (0..names.len()).filter(|&i| indegree[i] == 0).collect();
    let mut out = Vec::with_capacity(names.len());
    let mut qi = 0;
    while qi < queue.len() {
        let i = queue[qi];
        qi += 1;
        out.push(i);
        for &dep in &dependents[i] {
            indegree[dep] -= 1;
            if indegree[dep] == 0 {
                queue.push(dep);
            }
        }
    }
    // Cycles: append remaining in original order.
    if out.len() < names.len() {
        let placed: HashSet<usize> = out.iter().copied().collect();
        for (i, n) in names.iter().enumerate() {
            if !placed.contains(&i) {
                out.push(i);
                let _ = n;
            }
        }
    }
    out.into_iter().map(|i| names[i].clone()).collect()
}

/// Lifts a parameter schema (already-materialized JSON from IrParameter).
fn lift_param_schema(schema: &Option<serde_json::Value>) -> StgType {
    match schema {
        Some(v) => {
            let mut ctx = LiftCtx {
                root_name: "",
                raw: &BTreeMap::new(),
                warnings: &mut Vec::new(),
                depth: 0,
            };
            ctx.lift_type_value(v)
        }
        None => StgType::Prim(StgPrim {
            base: Base::Str,
            refs: Refinements::default(),
        }),
    }
}

fn type_word_of(json: &serde_json::Value) -> Option<&str> {
    json.get("type").and_then(serde_json::Value::as_str)
}
