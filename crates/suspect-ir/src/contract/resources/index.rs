use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use serde_json::Value;
use suspect_low::Pointer;

use super::*;
use crate::contract::{ContractDiagnostic, ContractSeverity, SchemaDialect};

type Spans = BTreeMap<String, Range<usize>>;

impl ResourceIndex {
    pub(in crate::contract) fn parent_scope(&self, source: &SourceId) -> Option<ResourceScope> {
        if source.pointer.is_empty() {
            return self.documents.get(source.document()).cloned().flatten();
        }
        let (parent, _) = source.pointer.rsplit_once('/')?;
        self.scope(&SourceId {
            document: source.document.clone(),
            pointer: parent.to_owned(),
        })
        .cloned()
    }
    pub(in crate::contract) fn scope(&self, source: &SourceId) -> Option<&ResourceScope> {
        let mut source = source.clone();
        loop {
            if let Some(scopes) = self.scopes.get(&source) {
                let mut agreed = scopes.first()?.as_ref()?;
                for scope in scopes.iter().skip(1) {
                    let scope = scope.as_ref()?;
                    if agreed.resource != scope.resource
                        || agreed.base_uri != scope.base_uri
                        || agreed.base_source != scope.base_source
                        || agreed.address != scope.address
                    {
                        return None;
                    }
                    // A non-schema HTTP role has no schema-root hint. Its
                    // absence does not contradict the same source's schema
                    // role or change URI resolution. Prefer the known hint,
                    // independently of registration order; conflicting known
                    // roots and invalid scope markers remain blocking.
                    match (&agreed.schema_root, &scope.schema_root) {
                        (Some(left), Some(right)) if left != right => return None,
                        (None, Some(_)) => agreed = scope,
                        _ => {}
                    }
                }
                return Some(agreed);
            }
            let (parent, _) = source.pointer.rsplit_once('/')?;
            source.pointer = parent.to_owned();
        }
    }

    pub(in crate::contract) fn register_scope(
        &mut self,
        source: SourceId,
        scope: Option<ResourceScope>,
    ) {
        let scopes = self.scopes.entry(source).or_default();
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }

    pub(in crate::contract) fn document(
        &mut self,
        source: &SourceId,
        raw: &Value,
        aliases: Vec<String>,
        spans: &Spans,
        diagnostics: &mut Vec<ContractDiagnostic>,
    ) -> Option<ResourceScope> {
        let retrieval = match resource_uri::resolve_document(source.document.as_str(), "") {
            Ok(uri) => uri,
            Err(error) => {
                report(
                    diagnostics,
                    spans,
                    source,
                    source,
                    "invalid-resource-uri",
                    error.to_string(),
                );
                self.register_scope(source.clone(), None);
                self.documents.insert(source.document.clone(), None);
                return None;
            }
        };
        let version = raw.get("openapi").and_then(Value::as_str);
        let kind = if version.is_some_and(|v| {
            v.starts_with("3.0.") || v.starts_with("3.1.") || v.starts_with("3.2.")
        }) {
            ResourceKind::OpenApiDocument
        } else {
            ResourceKind::Document
        };
        let mut resource = Resource {
            source: source.clone(),
            kind,
            canonical_uri: retrieval.clone(),
            base_uri: retrieval,
            declaration_source: None,
            aliases,
            anchors: Vec::new(),
        };
        if kind == ResourceKind::OpenApiDocument
            && let Some(value) = raw.get("$self")
        {
            let declaration = source.child("$self");
            if !version.is_some_and(|v| v.starts_with("3.2.")) {
                report(
                    diagnostics,
                    spans,
                    &declaration,
                    &declaration,
                    "UNSUPPORTED_HTTP_FIELD",
                    "OpenAPI document `$self` requires OpenAPI 3.2",
                );
            } else {
                let resolved = value
                    .as_str()
                    .ok_or_else(|| "OpenAPI `$self` must be a URI-reference string".to_owned())
                    .and_then(|value| {
                        resource_uri::resolve_reference(&resource.base_uri, value)
                            .map_err(|e| e.to_string())
                    });
                match resolved {
                    Ok(uri) => {
                        resource.base_uri = resource_uri::split_reference(&uri)
                            .expect("resolved URI")
                            .0
                            .to_owned();
                        resource.canonical_uri = uri;
                        resource.declaration_source = Some(declaration);
                    }
                    Err(error) => {
                        report(
                            diagnostics,
                            spans,
                            &declaration,
                            &declaration,
                            "invalid-openapi-self",
                            error,
                        );
                        self.register_scope(source.clone(), None);
                        self.documents.insert(source.document.clone(), None);
                        return None;
                    }
                }
            }
        }
        resource.aliases.push(resource.canonical_uri.clone());
        resource.aliases.push(resource.base_uri.clone());
        let scope = ResourceScope {
            resource: source.clone(),
            schema_root: None,
            base_uri: resource.base_uri.clone(),
            base_source: resource.declaration_source.clone(),
            address: resource.canonical_uri.clone(),
        };
        self.insert(resource);
        self.documents
            .insert(source.document.clone(), Some(scope.clone()));
        self.register_scope(source.clone(), Some(scope.clone()));
        Some(scope)
    }

    pub(in crate::contract) fn schema(
        &mut self,
        source: &SourceId,
        raw: &Value,
        context: &SchemaContext,
        parent: Option<ResourceScope>,
        spans: &Spans,
        diagnostics: &mut Vec<ContractDiagnostic>,
    ) -> Option<ResourceScope> {
        let legacy = matches!(context.dialect, SchemaDialect::OpenApi30);
        if !legacy && !supports_resources(context) {
            self.register_scope(source.clone(), None);
            return None;
        }
        let Some(mut scope) = parent else {
            self.register_scope(source.clone(), None);
            return None;
        };
        if !legacy && let Some(value) = raw.get("$id") {
            let declaration = source.child("$id");
            let resolved = value
                .as_str()
                .ok_or_else(|| "`$id` must be a URI-reference string".to_owned())
                .and_then(|value| {
                    let (_, fragment) =
                        resource_uri::split_reference(value).map_err(|e| e.to_string())?;
                    if !fragment.is_empty() {
                        return Err("`$id` must not contain a nonempty fragment".to_owned());
                    }
                    resource_uri::resolve_document(&scope.base_uri, value)
                        .map_err(|e| e.to_string())
                });
            let base = match resolved {
                Ok(base) => base,
                Err(error) => {
                    report(
                        diagnostics,
                        spans,
                        source,
                        &declaration,
                        "invalid-schema-resource",
                        error,
                    );
                    self.register_scope(source.clone(), None);
                    return None;
                }
            };
            let aliases = if source.pointer.is_empty() {
                self.resources
                    .get(source)
                    .into_iter()
                    .flatten()
                    .flat_map(|r| r.aliases.iter().cloned())
                    .collect()
            } else {
                Vec::new()
            };
            self.replace_document_root(source);
            self.insert(Resource {
                source: source.clone(),
                kind: ResourceKind::Schema,
                canonical_uri: base.clone(),
                base_uri: base.clone(),
                declaration_source: Some(declaration.clone()),
                aliases,
                anchors: Vec::new(),
            });
            scope.resource = source.clone();
            scope.schema_root = Some(source.clone());
            scope.base_uri = base;
            scope.base_source = Some(declaration);
        } else if source.pointer.is_empty() {
            // A standalone schema without $id still has a retrieval resource.
            if let Some(resources) = self.resources.get_mut(source) {
                for resource in resources {
                    if resource.kind == ResourceKind::Document {
                        resource.kind = ResourceKind::Schema;
                    }
                }
            }
            scope.schema_root = Some(source.clone());
        } else if scope.schema_root.is_none() {
            scope.schema_root = Some(source.clone());
        }
        scope.address = address(&scope, source);
        // The root's preliminary document scope is not a competing schema use.
        if source.pointer.is_empty()
            && let Some(scopes) = self.scopes.get_mut(source)
        {
            scopes.retain(|scope| {
                scope
                    .as_ref()
                    .is_none_or(|scope| scope.schema_root.is_some())
            });
        }
        self.register_scope(source.clone(), Some(scope.clone()));
        if !legacy {
            for (keyword, kind) in [
                ("$anchor", AnchorKind::Static),
                ("$dynamicAnchor", AnchorKind::Dynamic),
            ] {
                if let Some(value) = raw.get(keyword) {
                    let declaration = source.child(keyword);
                    let Some(name) = value.as_str().filter(|name| valid_anchor(name)) else {
                        report(
                            diagnostics,
                            spans,
                            source,
                            &declaration,
                            "invalid-schema-anchor",
                            format!("`{keyword}` must match [A-Za-z_][A-Za-z0-9_.-]*"),
                        );
                        continue;
                    };
                    let anchor = SchemaAnchor {
                        name: name.to_owned(),
                        source: declaration,
                        target: source.clone(),
                        resource: scope.resource.clone(),
                        kind,
                    };
                    if let Some(resources) = self.resources.get_mut(&scope.resource) {
                        for resource in resources
                            .iter_mut()
                            .filter(|r| r.base_uri == scope.base_uri)
                        {
                            if !resource.anchors.contains(&anchor) {
                                resource.anchors.push(anchor.clone());
                            }
                            resource.anchors.sort_by(|a, b| a.source.cmp(&b.source));
                        }
                    }
                }
            }
        }
        Some(scope)
    }

    fn replace_document_root(&mut self, source: &SourceId) {
        if let Some(resources) = self.resources.get_mut(source) {
            resources.retain(|resource| {
                !(resource.kind == ResourceKind::Document && resource.declaration_source.is_none())
            });
        }
    }

    fn insert(&mut self, mut resource: Resource) {
        resource.aliases.push(resource.canonical_uri.clone());
        resource.aliases.push(resource.base_uri.clone());
        resource.aliases.sort();
        resource.aliases.dedup();
        for alias in &resource.aliases {
            if let Ok(key) = uri_key(alias) {
                self.aliases
                    .entry(key)
                    .or_default()
                    .insert(resource.source.clone());
            }
        }
        let resources = self.resources.entry(resource.source.clone()).or_default();
        if let Some(previous) = resources.iter_mut().find(|previous| {
            previous.kind == resource.kind
                && previous.canonical_uri == resource.canonical_uri
                && previous.base_uri == resource.base_uri
                && previous.declaration_source == resource.declaration_source
        }) {
            previous.aliases.extend(resource.aliases);
            previous.aliases.sort();
            previous.aliases.dedup();
        } else {
            resources.push(resource);
        }
    }

    pub(in crate::contract) fn lookup(
        &self,
        contract: &Contract,
        absolute: &str,
    ) -> Result<SourceId, ResourceResolutionError> {
        let (document, encoded) = resource_uri::split_reference(absolute).map_err(invalid_uri)?;
        let fragment = resource_uri::decode_fragment(encoded).map_err(invalid_uri)?;
        let key = uri_key(absolute).map_err(invalid_uri)?;
        if let Some(ids) = self.aliases.get(&key) {
            let id = unique_resource(ids, &key)?;
            return Ok(id.clone());
        }
        let document = resource_uri::resolve_document(document, "").map_err(invalid_uri)?;
        let ids = self.aliases.get(&document).ok_or_else(|| {
            ResourceResolutionError::new(
                "unavailable-resource",
                format!("resource `{document}` is not registered in the supplied source closure"),
            )
        })?;
        let id = unique_resource(ids, &document)?;
        let resource = self.resource(id)?;
        let target = if fragment.is_empty() {
            id.clone()
        } else if fragment.starts_with('/') {
            let relative = Pointer::parse(&fragment).map_err(|error| {
                ResourceResolutionError::new("invalid-reference-pointer", error.to_string())
            })?;
            SourceId::new(
                id.document.clone(),
                Pointer::parse(id.pointer())
                    .expect("source pointer")
                    .join(&relative),
            )
        } else {
            let matches = resource
                .anchors
                .iter()
                .filter(|anchor| anchor.name == fragment)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [anchor] => anchor.target.clone(),
                [] => {
                    return Err(ResourceResolutionError::new(
                        "unknown-resource-anchor",
                        format!("#{fragment} matches no anchor declared in resource `{document}`"),
                    ));
                }
                _ => {
                    return Err(ResourceResolutionError::new(
                        "ambiguous-resource-anchor",
                        format!("anchor #{fragment} is ambiguous in resource `{document}`"),
                    ));
                }
            }
        };
        if contract.source(&target).is_none() {
            return Err(ResourceResolutionError::new(
                "missing-resource-pointer",
                format!(
                    "reference target {}#{} does not exist",
                    target.document(),
                    target.pointer()
                ),
            ));
        }
        Ok(target)
    }

    pub(in crate::contract) fn contains_uri(&self, absolute: &str) -> bool {
        let Ok(key) = uri_key(absolute) else {
            return false;
        };
        if self.aliases.contains_key(&key) {
            return true;
        }
        resource_uri::split_reference(&key)
            .is_ok_and(|(document, _)| self.aliases.contains_key(document))
    }

    fn resource(&self, id: &ResourceId) -> Result<&Resource, ResourceResolutionError> {
        match self.resources.get(id).map(Vec::as_slice) {
            Some([resource]) => Ok(resource),
            _ => Err(ResourceResolutionError::new(
                "ambiguous-resource-context",
                format!(
                    "resource {}#{} has no unique context",
                    id.document(),
                    id.pointer()
                ),
            )),
        }
    }

    pub(in crate::contract) fn declarations(&self, contract: &Contract) -> Vec<ContractDiagnostic> {
        let mut out = Vec::new();
        for (uri, sources) in &self.aliases {
            if sources.len() < 2 {
                continue;
            }
            for source in sources {
                for resource in self.resources.get(source).into_iter().flatten() {
                    let declaration = resource.declaration_source.as_ref().unwrap_or(source);
                    let owner = if declaration.pointer.ends_with("/$self") {
                        declaration
                    } else {
                        source
                    };
                    let spans = &contract.documents[source.document()].spans;
                    report(
                        &mut out,
                        spans,
                        owner,
                        declaration,
                        "invalid-resource-identity",
                        format!(
                            "URI `{uri}` identifies several physical resources: {}",
                            sources
                                .iter()
                                .map(|source| format!("{}#{}", source.document(), source.pointer()))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                }
            }
        }
        for variants in self.resources.values() {
            for resource in variants {
                let mut names = BTreeMap::<&str, Vec<&SchemaAnchor>>::new();
                for anchor in &resource.anchors {
                    names.entry(&anchor.name).or_default().push(anchor);
                }
                for (name, anchors) in names.into_iter().filter(|(_, anchors)| anchors.len() > 1) {
                    for anchor in anchors {
                        let spans = &contract.documents[anchor.source.document()].spans;
                        report(
                            &mut out,
                            spans,
                            &anchor.target,
                            &anchor.source,
                            "invalid-schema-anchor",
                            format!(
                                "anchor `{name}` is declared more than once in resource `{}`",
                                resource.base_uri
                            ),
                        );
                    }
                }
            }
        }
        out
    }

    pub(in crate::contract) fn declare_dynamic(&mut self, schema: &SchemaId) {
        self.dynamics
            .entry(schema.clone())
            .or_insert_with(|| DynamicReference {
                source: schema.child("$dynamicRef"),
                initial_target: None,
                initial_resource: None,
                dynamic_anchor: None,
                candidates: Vec::new(),
            });
    }

    pub(in crate::contract) fn dynamic_target(
        &mut self,
        schema: &SchemaId,
        absolute: &str,
        target: Option<SchemaId>,
    ) {
        let initial_resource = target
            .as_ref()
            .and_then(|target| self.scope(target))
            .map(|scope| scope.resource.clone());
        let name = resource_uri::split_reference(absolute)
            .ok()
            .and_then(|(_, fragment)| resource_uri::decode_fragment(fragment).ok())
            .filter(|name| !name.is_empty() && !name.starts_with('/'));
        let dynamic_anchor = name.filter(|name| {
            initial_resource
                .as_ref()
                .and_then(|id| self.resource(id).ok())
                .is_some_and(|resource| {
                    resource.anchors.iter().any(|anchor| {
                        anchor.kind == AnchorKind::Dynamic
                            && anchor.name == *name
                            && Some(&anchor.target) == target.as_ref()
                    })
                })
        });
        self.declare_dynamic(schema);
        let reference = self
            .dynamics
            .get_mut(schema)
            .expect("declared dynamic reference");
        reference.initial_target = target;
        reference.initial_resource = initial_resource;
        reference.dynamic_anchor = dynamic_anchor;
    }

    pub(in crate::contract) fn dynamic_candidates(
        &self,
        active_resources: &BTreeSet<ResourceId>,
    ) -> Vec<SchemaAnchor> {
        let names = self
            .dynamics
            .values()
            .filter_map(|reference| reference.dynamic_anchor.as_ref())
            .collect::<BTreeSet<_>>();
        self.resources
            .values()
            .flatten()
            .filter(|resource| active_resources.contains(&resource.source))
            .flat_map(|resource| &resource.anchors)
            .filter(|anchor| anchor.kind == AnchorKind::Dynamic && names.contains(&anchor.name))
            .cloned()
            .collect()
    }

    pub(in crate::contract) fn finish_dynamic_candidates(&mut self, candidates: &[SchemaAnchor]) {
        for reference in self.dynamics.values_mut() {
            reference.candidates = candidates
                .iter()
                .filter(|anchor| Some(&anchor.name) == reference.dynamic_anchor.as_ref())
                .cloned()
                .collect();
            reference.candidates.sort_by(|a, b| a.source.cmp(&b.source));
            reference.candidates.dedup();
        }
    }
}

pub(in crate::contract) fn address(scope: &ResourceScope, source: &SourceId) -> String {
    let pointer = source
        .pointer
        .strip_prefix(scope.resource.pointer())
        .expect("lexical resource contains source");
    if pointer.is_empty() {
        scope.base_uri.clone()
    } else {
        format!(
            "{}#{}",
            scope.base_uri,
            resource_uri::encode_fragment(pointer)
        )
    }
}

fn valid_anchor(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(byte))
}

fn uri_key(uri: &str) -> Result<String, resource_uri::Error> {
    let (document, fragment) = resource_uri::split_reference(uri)?;
    let document = resource_uri::resolve_document(document, "")?;
    let fragment = resource_uri::decode_fragment(fragment)?;
    Ok(if fragment.is_empty() {
        document
    } else {
        format!("{document}#{}", resource_uri::encode_fragment(&fragment))
    })
}

fn unique_resource<'a>(
    ids: &'a BTreeSet<ResourceId>,
    uri: &str,
) -> Result<&'a ResourceId, ResourceResolutionError> {
    if ids.len() != 1 {
        return Err(ResourceResolutionError::new(
            "ambiguous-resource-id",
            format!("URI `{uri}` identifies multiple physical resources"),
        ));
    }
    Ok(ids.first().expect("one resource"))
}

fn invalid_uri(error: resource_uri::Error) -> ResourceResolutionError {
    ResourceResolutionError::new("invalid-reference-uri", error.to_string())
}

fn report(
    out: &mut Vec<ContractDiagnostic>,
    spans: &Spans,
    owner: &SourceId,
    at: &SourceId,
    code: &'static str,
    message: impl Into<String>,
) {
    out.push(ContractDiagnostic {
        source: owner.clone(),
        at: spans.get(at.pointer()).cloned().unwrap_or_default(),
        code,
        severity: ContractSeverity::Error,
        message: message.into(),
    });
}
