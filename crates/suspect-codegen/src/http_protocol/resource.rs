use suspect_ir::contract::{ResourceKind as IrResourceKind, SchemaDialect, SourceId};

use super::planner::{Planner, parent};
use super::*;

impl Planner<'_> {
    pub(super) fn resource_context(&self, source: &SourceId) -> Option<ResourceContext> {
        let scope = self.contract.resource_scope(source)?;
        let resource = self.contract.resource(scope.resource())?;
        Some(ResourceContext {
            source: self.location(source),
            resource: self.location(resource.source()),
            kind: match resource.kind() {
                IrResourceKind::Document => ResourceKind::Document,
                IrResourceKind::OpenApiDocument => ResourceKind::OpenApiDocument,
                IrResourceKind::Schema => ResourceKind::Schema,
            },
            canonical_uri: resource.canonical_uri().to_owned(),
            base_uri: scope.base_uri().to_owned(),
            base_source: scope.base_source().map(|source| self.location(source)),
            scope_address: scope.address().to_owned(),
            schema_root: scope.schema_root().map(|source| self.location(source)),
            aliases: resource.aliases().to_vec(),
        })
    }

    pub(super) fn provenance(
        &self,
        use_site: &SourceId,
        terminal: &SourceId,
        references: Vec<SourceLocation>,
    ) -> Provenance {
        let reference_resources = references
            .iter()
            .map(|source| self.resource_context(&source.source))
            .collect();
        Provenance {
            use_site: self.location(use_site),
            terminal: self.location(terminal),
            references,
            use_site_resource: self.resource_context(use_site),
            terminal_resource: self.resource_context(terminal),
            reference_resources,
        }
    }

    /// A malformed/ambiguous logical scope is a source error, never permission
    /// to fall back to the physical document as a reference base.
    pub(super) fn ensure_resource_scope(&mut self, source: &SourceId) -> bool {
        if self
            .contract
            .resource_scope(source)
            .and_then(|scope| self.contract.resource(scope.resource()))
            .is_some()
        {
            return true;
        }
        let finding = self
            .contract
            .diagnostics()
            .iter()
            .filter(|d| {
                if d.source.document() != source.document() || !resource_finding(d.code) {
                    return false;
                }
                let boundary = if d.source.pointer().ends_with("/$self")
                    || d.source.pointer().ends_with("/$id")
                {
                    parent(&d.source).unwrap_or_else(|| d.source.clone())
                } else {
                    d.source.clone()
                };
                boundary.pointer().is_empty()
                    || source.pointer() == boundary.pointer()
                    || source
                        .pointer()
                        .strip_prefix(boundary.pointer())
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .max_by_key(|d| d.source.pointer().len());
        if let Some(finding) = finding {
            self.diagnostics.push(Diagnostic {
                source: SourceLocation {
                    source: finding.source.clone(),
                    span: finding.at.clone(),
                },
                code: finding.code,
                severity: Severity::Error,
                kind: if finding.code.starts_with("unsupported-") {
                    DiagnosticKind::Unsupported
                } else {
                    DiagnosticKind::InvalidSource
                },
                message: finding.message.clone(),
                capability: None,
                related: vec![self.location(source)],
                resource_context: self.resource_context(source),
            });
        } else {
            self.error(
                source,
                "http-resource-scope",
                "source has no unambiguous canonical resource scope in Contract",
            );
        }
        false
    }

    /// These are resource/containment metadata, not instance assertions. Only a
    /// known modern dialect and registered scope make that interpretation safe.
    pub(super) fn resource_annotation(&self, source: &SourceId, keyword: &str) -> bool {
        matches!(keyword, "$id" | "$anchor" | "$dynamicAnchor" | "$defs")
            && self.contract.schema(source).is_some_and(|s| matches!(s.dialect(), SchemaDialect::Uri(uri) if matches!(uri.trim_end_matches('#'), "https://spec.openapis.org/oas/3.1/dialect/base" | "https://json-schema.org/draft/2020-12/schema")))
            && self.contract.resource_scope(source).is_some()
    }

    /// Executable semantics are gated only for actual codec inputs, including
    /// relevant dynamic candidates. Ignored 3.0 siblings and forbidden response
    /// body declarations must not become executable codec requirements.
    pub(super) fn codec_resource_capabilities(&mut self, closure: &[SourceId]) {
        for id in closure {
            let Some(schema) = self.contract.schema(id) else {
                continue;
            };
            if schema.ignores_ref_siblings() {
                continue;
            }
            self.ensure_resource_scope(id);
            if let Some(base_source) = self
                .contract
                .resource_scope(id)
                .and_then(|scope| scope.base_source())
            {
                self.require(Capability::SchemaResources, base_source);
            }
            for key in ["$id", "$anchor", "$dynamicAnchor"] {
                if schema.raw().get(key).is_some() {
                    self.require(Capability::SchemaResources, &id.child(key));
                }
            }
            if schema.raw().get("$ref").is_some()
                && self
                    .contract
                    .resource_scope(id)
                    .is_some_and(|scope| scope.base_source().is_some())
            {
                self.require(Capability::SchemaResources, &id.child("$ref"));
            }
            for key in ["$dynamicRef", "$dynamicAnchor"] {
                if schema.raw().get(key).is_some() {
                    self.require(Capability::SchemaResources, &id.child(key));
                    self.require(Capability::DynamicSchemaReferences, &id.child(key));
                }
            }
            if schema.raw().get("$dynamicRef").is_some()
                && self
                    .contract
                    .dynamic_reference(id)
                    .is_none_or(|reference| reference.initial_target().is_none())
            {
                self.error(&id.child("$dynamicRef"), "http-dynamic-reference-unresolved", "dynamic schema reference has no indexed initial target; it cannot be replaced by a guessed static binding");
            }
        }
    }
}

pub(super) fn resource_finding(code: &str) -> bool {
    matches!(
        code,
        "invalid-resource-identity"
            | "invalid-resource-uri"
            | "invalid-resource-scope"
            | "invalid-openapi-self"
            | "invalid-schema-resource"
            | "unsupported-schema-resource"
    )
}
