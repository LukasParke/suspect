//! Build the source graph from HTTP/schema positions, including split documents.
//!
//! URI resources and anchor candidates come only from registered positions;
//! generic document-wide keyword scans cannot turn instances into schemas.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde_json::Value;
use suspect_low::Pointer;
use suspect_ref::{Workspace, resource_uri};
use suspect_source::Uri;

use super::SchemaContext as Context;
use super::shape::{self, Kind};
use super::{
    Contract, ContractDiagnostic, ContractError, ContractReader, ContractSeverity, SchemaDialect,
    SchemaNode, SchemaReference, SourceId,
};

#[derive(Clone)]
struct Task {
    source: SourceId,
    kind: Kind,
    context: Context,
}

#[derive(Clone)]
struct ReferenceTask {
    task: Task,
    keyword: &'static str,
}

#[derive(Clone)]
struct ReferenceUse {
    reference: ReferenceTask,
    absolute: String,
}

struct Catalog {
    kind: Kind,
    version: String,
    dialect: SchemaDialect,
    available: Vec<Uri>,
    documents: Vec<Contract>,
}

fn walker<'a>(
    contract: &'a mut Contract,
    workspace: &'a Workspace,
    reader: ContractReader,
) -> Walker<'a> {
    Walker {
        contract,
        workspace,
        reader,
        registered_documents: HashSet::new(),
        registry: BTreeMap::new(),
        registered_contexts: BTreeMap::new(),
        seen: BTreeMap::new(),
        pending: Vec::new(),
        references: Vec::new(),
        deferred: Vec::new(),
        resolved: Vec::new(),
        attempted_documents: HashSet::new(),
        catalogs: Vec::new(),
        load_errors: BTreeMap::new(),
        registration_revision: 0,
    }
}

pub(super) fn index(
    contract: &mut Contract,
    workspace: &Workspace,
    reader: ContractReader,
    dialect: SchemaDialect,
) -> Result<(), ContractError> {
    let entry = contract.entry.clone();
    let root = SourceId::new(entry.clone(), Pointer::root());
    let default_source = contract
        .source(&root.child("jsonSchemaDialect"))
        .map(|_| root.child("jsonSchemaDialect"));
    let context = Context {
        version: contract.openapi_version.clone(),
        dialect,
        version_source: root.child("openapi"),
        dialect_source: default_source.clone(),
        default_dialect_source: default_source,
    };
    let mut walker = walker(contract, workspace, reader);
    walker.register_document(&entry, &context);
    let context = walker.document_context(&entry, &context);
    walker.pending.push(Task {
        source: SourceId::new(entry, Pointer::root()),
        kind: Kind::Root,
        context,
    });
    loop {
        while let Some(task) = walker.pending.pop() {
            walker.visit(task);
        }
        if !walker.references.is_empty() {
            for task in std::mem::take(&mut walker.references) {
                walker.reference(task)?;
            }
            continue;
        }
        let revision = walker.registration_revision;
        for reference in std::mem::take(&mut walker.deferred) {
            walker.finish_reference(reference)?;
        }
        if !walker.pending.is_empty() || walker.registration_revision != revision {
            continue;
        }
        let candidates = walker.dynamic_candidates();
        for anchor in &candidates {
            if !walker.contract.schemas.contains_key(anchor.target()) {
                for context in walker
                    .registered_contexts
                    .get(&(anchor.target().clone(), Kind::Schema))
                    .cloned()
                    .unwrap_or_default()
                {
                    walker.pending.push(Task {
                        source: anchor.target().clone(),
                        kind: Kind::Schema,
                        context,
                    });
                }
            }
        }
        if walker.pending.is_empty() {
            walker
                .contract
                .resources
                .finish_dynamic_candidates(&candidates);
            for usage in std::mem::take(&mut walker.deferred) {
                let error = walker
                    .lookup(&usage.absolute)
                    .expect_err("unresolved reference at fixed point");
                let document = resource_uri::split_reference(&usage.absolute)
                    .expect("validated URI")
                    .0;
                let message = walker
                    .load_errors
                    .get(document)
                    .cloned()
                    .unwrap_or_else(|| error.to_string());
                walker.reference_error(&usage.reference, message);
            }
            break;
        }
    }
    // Late resource declarations can expose duplicate identities or change the
    // meaning of a previously resolved alias. No discovery-order first wins.
    for reference in std::mem::take(&mut walker.resolved) {
        match walker
            .absolute(&reference.reference)
            .and_then(|absolute| walker.lookup(&absolute).map(|target| (absolute, target)))
        {
            Ok((absolute, target)) => walker.set_target(
                &ReferenceUse {
                    absolute,
                    ..reference
                },
                Some(target),
            ),
            Err(error) => walker.reference_error(&reference.reference, error.to_string()),
        }
    }
    let candidates = walker.dynamic_candidates();
    walker
        .contract
        .resources
        .finish_dynamic_candidates(&candidates);
    let diagnostics = walker.contract.resources.declarations(walker.contract);
    walker.contract.diagnostics.extend(diagnostics);
    Ok(())
}

struct Walker<'a> {
    contract: &'a mut Contract,
    workspace: &'a Workspace,
    reader: ContractReader,
    registered_documents: HashSet<Uri>,
    registry: BTreeMap<(SourceId, Kind), Context>,
    registered_contexts: BTreeMap<(SourceId, Kind), Vec<Context>>,
    seen: BTreeMap<(SourceId, Kind), Vec<Context>>,
    pending: Vec<Task>,
    references: Vec<ReferenceTask>,
    deferred: Vec<ReferenceUse>,
    resolved: Vec<ReferenceUse>,
    attempted_documents: HashSet<String>,
    catalogs: Vec<Catalog>,
    load_errors: BTreeMap<String, String>,
    registration_revision: usize,
}

impl Walker<'_> {
    fn document_context(&self, document: &Uri, inherited: &Context) -> Context {
        let raw = &self.contract.documents[document].raw;
        let Some(version) = raw.get("openapi").and_then(Value::as_str) else {
            return inherited.clone();
        };
        let dialect = if version.starts_with("3.0.") {
            SchemaDialect::OpenApi30
        } else if version.starts_with("3.1.") || version.starts_with("3.2.") {
            SchemaDialect::Uri(
                match raw.get("jsonSchemaDialect") {
                    None => "https://spec.openapis.org/oas/3.1/dialect/base",
                    Some(value) => value.as_str().unwrap_or(""),
                }
                .to_owned(),
            )
        } else {
            return inherited.clone();
        };
        let root = SourceId::new(document.clone(), Pointer::root());
        let default_source = raw
            .get("jsonSchemaDialect")
            .map(|_| root.child("jsonSchemaDialect"));
        Context {
            version: version.to_owned(),
            dialect,
            version_source: root.child("openapi"),
            dialect_source: default_source.clone(),
            default_dialect_source: default_source,
        }
    }

    fn local_context(&self, source: &SourceId, kind: Kind, mut context: Context) -> Context {
        if kind == Kind::Schema
            && !matches!(context.dialect, SchemaDialect::OpenApi30)
            && let Some(value) = self
                .contract
                .source(source)
                .and_then(|raw| raw.get("$schema"))
        {
            context.dialect = SchemaDialect::Uri(value.as_str().unwrap_or("").to_owned());
            context.dialect_source = Some(source.child("$schema"));
        }
        context
    }

    fn source_context(&self, source: &SourceId, kind: Kind, inherited: &Context) -> Context {
        let mut context = self.document_context(source.document(), inherited);
        if kind != Kind::Schema {
            return context;
        }
        let pointer = Pointer::parse(source.pointer()).expect("indexed source pointer");
        for length in 0..=pointer.tokens().len() {
            let at = SourceId::new(
                source.document().clone(),
                Pointer::from_tokens(pointer.tokens()[..length].to_vec()),
            );
            if at == *source || self.registry.contains_key(&(at.clone(), Kind::Schema)) {
                context = self.local_context(&at, Kind::Schema, context);
            }
        }
        context
    }

    fn record_schema_context(&mut self, task: &Task) {
        if task.kind != Kind::Schema {
            return;
        }
        let contexts = self
            .contract
            .schema_contexts
            .entry(task.source.clone())
            .or_default();
        let different = contexts
            .iter()
            .find(|previous| {
                let same_dialect = match (&previous.dialect, &task.context.dialect) {
                    (SchemaDialect::OpenApi30, SchemaDialect::OpenApi30) => true,
                    (SchemaDialect::Uri(a), SchemaDialect::Uri(b)) => {
                        a.strip_suffix('#').unwrap_or(a) == b.strip_suffix('#').unwrap_or(b)
                    }
                    _ => false,
                };
                !same_dialect
                    || previous.version.split('.').take(2).ne(task
                        .context
                        .version
                        .split('.')
                        .take(2))
            })
            .cloned();
        if !contexts.contains(&task.context) {
            contexts.push(task.context.clone());
        }
        if let Some(previous) = different {
            let origin = |context: &Context| {
                let source = context
                    .dialect_source
                    .as_ref()
                    .unwrap_or(&context.version_source);
                format!(
                    "{}#{} ({}, {:?})",
                    source.document(),
                    source.pointer(),
                    context.version,
                    context.dialect
                )
            };
            self.error(task.source.clone(),"AMBIGUOUS_OPENAPI_CONTEXT",format!(
                "this schema is used with conflicting effective contexts from {} and {}; one source cannot silently choose a dialect",
                origin(&previous),origin(&task.context)));
        }
    }

    fn register_document(&mut self, document: &Uri, inherited: &Context) {
        if !self.registered_documents.insert(document.clone()) {
            return;
        }
        self.registration_revision += 1;
        let context = self.document_context(document, inherited);
        let source = SourceId::new(document.clone(), Pointer::root());
        let aliases = self
            .workspace
            .available_document_uris()
            .into_iter()
            .filter(|uri| {
                self.workspace
                    .document_metadata(uri)
                    .map_or(uri, |metadata| metadata.effective_uri())
                    == document
            })
            .map(|uri| uri.to_string())
            .collect();
        let document = &self.contract.documents[document];
        self.contract.resources.document(
            &source,
            &document.raw,
            aliases,
            &document.spans,
            &mut self.contract.diagnostics,
        );
        if document.raw.get("openapi").is_none() {
            return;
        }
        self.register(Task {
            source,
            kind: Kind::Root,
            context,
        });
    }

    /// Register lexical shapes without following references or expanding the
    /// Contract closure. Complete-document registration makes forward anchors
    /// visible while keeping unrelated reference targets out of the snapshot.
    fn register(&mut self, task: Task) {
        let mut pending = vec![task];
        while let Some(mut task) = pending.pop() {
            if self.contract.source(&task.source).is_none() {
                continue;
            }
            task.context = self.local_context(&task.source, task.kind, task.context);
            self.record_schema_context(&task);
            let contexts = self
                .registered_contexts
                .entry((task.source.clone(), task.kind))
                .or_default();
            if contexts.contains(&task.context) {
                continue;
            }
            contexts.push(task.context.clone());
            self.registration_revision += 1;
            let parent = self.contract.resources.parent_scope(&task.source);
            let document = &self.contract.documents[task.source.document()];
            let raw = document
                .raw
                .pointer(task.source.pointer())
                .expect("registered source");
            if task.kind == Kind::Schema {
                self.contract.resources.schema(
                    &task.source,
                    raw,
                    &task.context,
                    parent,
                    &document.spans,
                    &mut self.contract.diagnostics,
                );
            } else {
                self.contract.resources.register_scope(
                    task.source.clone(),
                    parent.map(|scope| scope.at_source(&task.source)),
                );
            }
            pending.extend(
                shape::children(&task.source, raw, task.kind, &task.context.version)
                    .into_iter()
                    .map(|(source, kind)| Task {
                        source,
                        kind,
                        context: task.context.clone(),
                    }),
            );
            self.registry
                .entry((task.source, task.kind))
                .or_insert(task.context);
        }
    }

    fn visit(&mut self, mut task: Task) {
        self.register(task.clone());
        task.context = self.local_context(&task.source, task.kind, task.context);
        self.record_schema_context(&task);
        if let Some(previous) = self.contract.source_versions.get(&task.source) {
            if previous
                .split('.')
                .take(2)
                .ne(task.context.version.split('.').take(2))
            {
                self.error(task.source.clone(), "AMBIGUOUS_OPENAPI_CONTEXT",
                    "this fragment is used with different OpenAPI feature sets; one source cannot have two effective HTTP vocabularies");
            }
        } else {
            self.contract
                .source_versions
                .insert(task.source.clone(), task.context.version.clone());
        }
        let contexts = self
            .seen
            .entry((task.source.clone(), task.kind))
            .or_default();
        if contexts.contains(&task.context) {
            return;
        }
        contexts.push(task.context.clone());
        let raw = self
            .contract
            .source(&task.source)
            .expect("registered source exists");
        let children = shape::children(&task.source, raw, task.kind, &task.context.version);
        let has_reference =
            task.kind.reference_allowed(&task.context.version) && raw.get("$ref").is_some();
        let has_dynamic = task.kind == Kind::Schema
            && super::resources::supports_resources(&task.context)
            && raw.get("$dynamicRef").is_some();
        if matches!(task.kind, Kind::Root | Kind::Operation) {
            // Keep the contract's documented source-document policy for implicit
            // security names, including external schemes reached only this way.
            if let Some(requirements) = raw.get("security").and_then(Value::as_array) {
                let root = SourceId::new(task.source.document.clone(), Pointer::root());
                for name in requirements
                    .iter()
                    .filter_map(Value::as_object)
                    .flat_map(|map| map.keys())
                {
                    let source = root
                        .child("components")
                        .child("securitySchemes")
                        .child(name);
                    if self.contract.source(&source).is_some() {
                        self.pending.push(Task {
                            source,
                            kind: Kind::SecurityScheme,
                            context: task.context.clone(),
                        });
                    }
                }
            }
        }
        if has_reference {
            self.contract
                .reference_targets
                .entry(task.source.clone())
                .or_insert(None);
            self.references.push(ReferenceTask {
                task: task.clone(),
                keyword: "$ref",
            });
        }
        if has_dynamic {
            self.contract.resources.declare_dynamic(&task.source);
            self.references.push(ReferenceTask {
                task: task.clone(),
                keyword: "$dynamicRef",
            });
        }
        if task.kind == Kind::Schema {
            self.contract
                .schemas
                .entry(task.source.clone())
                .or_insert_with(|| SchemaNode {
                    id: task.source.clone(),
                    span: self.contract.documents[task.source.document()].spans
                        [task.source.pointer()]
                    .clone(),
                    dialect: task.context.dialect.clone(),
                    dialect_source: task.context.dialect_source.clone(),
                    default_dialect_source: task.context.default_dialect_source.clone(),
                    children: children.iter().map(|(source, _)| source.clone()).collect(),
                    references: [("$ref", has_reference), ("$dynamicRef", has_dynamic)]
                        .into_iter()
                        .filter(|(_, present)| *present)
                        .map(|(keyword, _)| SchemaReference {
                            keyword: keyword.to_owned(),
                            target: None,
                        })
                        .collect(),
                });
        } else {
            self.contract.roots.extend(
                children
                    .iter()
                    .filter(|(_, kind)| *kind == Kind::Schema)
                    .map(|(source, _)| source.clone()),
            );
        }
        self.pending
            .extend(children.into_iter().map(|(source, kind)| Task {
                source,
                kind,
                context: task.context.clone(),
            }));
    }

    fn lookup(&self, absolute: &str) -> Result<SourceId, super::ResourceResolutionError> {
        self.contract.resources.lookup(self.contract, absolute)
    }

    fn absolute(
        &self,
        reference: &ReferenceTask,
    ) -> Result<String, super::ResourceResolutionError> {
        let source = &reference.task.source;
        let raw = self
            .contract
            .source(source)
            .and_then(|value| value.get(reference.keyword))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                super::ResourceResolutionError::new(
                    "invalid-reference-uri",
                    format!("{} must be a URI-reference string", reference.keyword),
                )
            })?;
        resource_uri::split_reference(raw).map_err(|error| {
            super::ResourceResolutionError::new("invalid-reference-uri", error.to_string())
        })?;
        let base = self.reference_base(&reference.task).ok_or_else(|| {
            super::ResourceResolutionError::new(
                "invalid-resource-scope",
                format!(
                    "reference at {}#{} has no valid unambiguous URI scope",
                    source.document(),
                    source.pointer()
                ),
            )
        })?;
        resource_uri::resolve_reference(&base, raw).map_err(|error| {
            super::ResourceResolutionError::new("invalid-reference-uri", error.to_string())
        })
    }

    fn reference_base(&self, task: &Task) -> Option<String> {
        if let Some(scope) = self.contract.resource_scope(&task.source) {
            return Some(scope.base_uri().to_owned());
        }
        if task.kind != Kind::Schema
            || super::resources::supports_resources(&task.context)
            || matches!(task.context.dialect, SchemaDialect::OpenApi30)
        {
            return None;
        }
        // Preserve physical reference closure and original dialect diagnostics.
        // This is source graph discovery under an already known enclosing base;
        // an unsupported/invalid $id must not be guessed or stepped across.
        let mut current = task.source.clone();
        loop {
            if let Some(scope) = self.contract.resource_scope(&current) {
                return Some(scope.base_uri().to_owned());
            }
            if self.contract.is_schema_position(&current)
                && self
                    .contract
                    .source(&current)
                    .is_some_and(|raw| raw.get("$id").is_some())
            {
                return None;
            }
            let Some((parent, _)) = current.pointer.rsplit_once('/') else {
                return self
                    .contract
                    .resources
                    .parent_scope(&current)
                    .map(|scope| scope.base_uri().to_owned());
            };
            current.pointer = parent.to_owned();
        }
    }

    fn reference(&mut self, reference: ReferenceTask) -> Result<(), ContractError> {
        let unsupported = reference.task.kind == Kind::Schema
            && !matches!(reference.task.context.dialect, SchemaDialect::OpenApi30)
            && !super::resources::supports_resources(&reference.task.context);
        if unsupported {
            self.error(
                reference.task.source.clone(),
                "unsupported-resource-dialect",
                "reference/resource semantics require a supported schema dialect",
            );
        }
        let absolute = match self.absolute(&reference) {
            Ok(absolute) => absolute,
            Err(error) if unsupported && error.code() == "invalid-resource-scope" => return Ok(()),
            Err(error) => {
                self.reference_error(&reference, error.to_string());
                return Ok(());
            }
        };
        let usage = ReferenceUse {
            reference,
            absolute,
        };
        // Canonical identifiers outside the authorized retrieval namespace are
        // checked across supplied declarations even if one active document has
        // already registered the name. This prevents discovery-order first wins.
        if self.is_retrieval_name(&usage.absolute) {
            // An explicitly supplied retrieval name has its own pinned bytes.
            // Register them before trusting a same-named $id from another source.
            self.try_open_document(&usage)?;
        } else {
            self.discover_resources(&usage)?;
        }
        if let Ok(target) = self.lookup(&usage.absolute) {
            self.link(usage, target);
        } else if self.try_open_document(&usage)? {
            match self.lookup(&usage.absolute) {
                Ok(target) => self.link(usage, target),
                Err(_) => self.deferred.push(usage),
            }
        } else {
            self.deferred.push(usage);
        }
        Ok(())
    }

    fn finish_reference(&mut self, usage: ReferenceUse) -> Result<(), ContractError> {
        if !self.is_retrieval_name(&usage.absolute) {
            self.discover_resources(&usage)?;
        }
        match self.lookup(&usage.absolute) {
            Ok(target) => self.link(usage, target),
            Err(_) => self.deferred.push(usage),
        }
        Ok(())
    }

    fn is_retrieval_name(&self, absolute: &str) -> bool {
        let Ok((document, _)) = resource_uri::split_reference(absolute) else {
            return false;
        };
        self.workspace.available_document_uris().iter().any(|uri| {
            resource_uri::resolve_document(uri.as_str(), "")
                .ok()
                .as_deref()
                == Some(document)
        })
    }

    fn discover_resources(&mut self, usage: &ReferenceUse) -> Result<(), ContractError> {
        let incoming = &usage.reference.task;
        let available = self.workspace.available_document_uris();
        let index = if let Some(index) = self.catalogs.iter().position(|catalog| {
            catalog.kind == incoming.kind
                && catalog.version == incoming.context.version
                && catalog.dialect == incoming.context.dialect
                && catalog.available == available
        }) {
            index
        } else {
            let mut documents = Vec::new();
            let mut seen = HashSet::new();
            for uri in &available {
                // available_document_uris has applied the same requested and
                // effective allowlist checks as Workspace::open. No logical URI
                // is inserted into that allowlist or sent to an acquisition API.
                let Ok(handle) = self.workspace.open(uri.as_str()) else {
                    continue;
                };
                let uri = handle.uri().clone();
                if !seen.insert(uri.clone()) {
                    continue;
                }
                // Registration-only discovery uses the lossless source sidecar.
                // Once selected, add_document uses the requested reader and
                // retains its normal explicit Fast-reader decline behavior.
                let Ok(document) = super::compile::document(handle.doc(), ContractReader::Lossless)
                else {
                    continue;
                };
                let mut catalog =
                    super::compile::snapshot(uri.clone(), incoming.context.version.clone());
                catalog.documents.insert(uri.clone(), document);
                {
                    let mut registration = walker(&mut catalog, self.workspace, self.reader);
                    registration.register_document(&uri, &incoming.context);
                    let root = SourceId::new(uri.clone(), Pointer::root());
                    if registration
                        .contract
                        .source(&root)
                        .is_some_and(|raw| raw.get("openapi").is_none())
                    {
                        registration.register(Task {
                            source: root,
                            kind: incoming.kind,
                            context: incoming.context.clone(),
                        });
                    }
                }
                documents.push(catalog);
            }
            self.catalogs.push(Catalog {
                kind: incoming.kind,
                version: incoming.context.version.clone(),
                dialect: incoming.context.dialect.clone(),
                available,
                documents,
            });
            self.catalogs.len() - 1
        };
        let matches = self.catalogs[index]
            .documents
            .iter()
            .filter(|catalog| catalog.resources.contains_uri(&usage.absolute))
            .map(|catalog| catalog.entry.clone())
            .collect::<Vec<_>>();
        for document in matches {
            self.contract
                .add_document(self.workspace, &document, self.reader)?;
            self.prepare_document(&document, incoming);
        }
        Ok(())
    }

    fn try_open_document(&mut self, usage: &ReferenceUse) -> Result<bool, ContractError> {
        let (document, _) =
            resource_uri::split_reference(&usage.absolute).expect("validated reference URI");
        let Ok(uri) = Uri::parse(document) else {
            return Ok(false);
        };
        if resource_uri::resolve_document(uri.as_str(), "")
            .ok()
            .as_deref()
            != Some(document)
        {
            return Ok(false); // A URL parser must not repair a logical identifier.
        }
        let available = self.workspace.available_document_uris().contains(&uri);
        if !(available || self.workspace.document_provider().is_none() && uri.scheme() == "file") {
            return Ok(false);
        }
        if !self.attempted_documents.insert(document.to_owned()) {
            return Ok(false);
        }
        let document = match self.workspace.open(uri.as_str()) {
            Ok(handle) => handle.uri().clone(),
            Err(error) => {
                self.load_errors
                    .insert(document.to_owned(), error.to_string());
                return Ok(false);
            }
        };
        self.contract
            .add_document(self.workspace, &document, self.reader)?;
        self.prepare_document(&document, &usage.reference.task);
        Ok(true)
    }

    fn prepare_document(&mut self, document: &Uri, incoming: &Task) {
        self.register_document(document, &incoming.context);
        let root = SourceId::new(document.clone(), Pointer::root());
        if incoming.kind == Kind::Schema
            && self
                .contract
                .source(&root)
                .is_some_and(|raw| raw.get("openapi").is_none())
            && !self
                .registry
                .keys()
                .any(|(source, kind)| *source == root && *kind != Kind::Schema)
        {
            self.register(Task {
                source: root,
                kind: Kind::Schema,
                context: self.document_context(document, &incoming.context),
            });
        }
    }

    fn link(&mut self, usage: ReferenceUse, source: SourceId) {
        let from = &usage.reference.task;
        self.prepare_document(source.document(), from);
        let target = Task {
            source: source.clone(),
            kind: from.kind,
            context: self.source_context(&source, from.kind, &from.context),
        };
        self.register(target.clone());
        // A physical URI/pointer identifies the value even if its dialect is
        // unsupported. Keep the target/context for precise admission diagnostics.
        self.set_target(&usage, Some(source));
        self.resolved.push(usage);
        self.pending.push(target);
    }

    fn set_target(&mut self, usage: &ReferenceUse, target: Option<SourceId>) {
        let source = &usage.reference.task.source;
        let keyword = usage.reference.keyword;
        if keyword == "$ref" {
            self.contract
                .reference_targets
                .insert(source.clone(), target.clone());
        }
        if let Some(schema) = self.contract.schemas.get_mut(source) {
            if let Some(reference) = schema
                .references
                .iter_mut()
                .find(|reference| reference.keyword == keyword)
            {
                reference.target = target.clone();
            } else {
                schema.references.push(SchemaReference {
                    keyword: keyword.to_owned(),
                    target: target.clone(),
                });
            }
        }
        if keyword == "$dynamicRef" {
            self.contract
                .resources
                .dynamic_target(source, &usage.absolute, target);
        }
    }

    fn dynamic_candidates(&self) -> Vec<super::SchemaAnchor> {
        let resources = self
            .contract
            .schemas
            .keys()
            .filter_map(|id| self.contract.resource_scope(id))
            .map(|scope| scope.resource().clone())
            .collect::<BTreeSet<_>>();
        self.contract.resources.dynamic_candidates(&resources)
    }

    fn reference_error(&mut self, reference: &ReferenceTask, message: impl Into<String>) {
        self.set_target(
            &ReferenceUse {
                reference: reference.clone(),
                absolute: String::new(),
            },
            None,
        );
        let source = &reference.task.source;
        let at = self
            .contract
            .source_span(&source.child(reference.keyword))
            .unwrap_or_default();
        self.contract.diagnostics.push(ContractDiagnostic {
            source: source.clone(),
            at,
            code: if reference.keyword == "$dynamicRef" {
                "invalid-dynamic-reference"
            } else {
                "invalid-reference"
            },
            severity: ContractSeverity::Error,
            message: message.into(),
        });
    }

    fn error(&mut self, source: SourceId, code: &'static str, message: impl Into<String>) {
        let at = self.contract.source_span(&source).unwrap_or_default();
        self.contract.diagnostics.push(ContractDiagnostic {
            source,
            at,
            code,
            severity: ContractSeverity::Error,
            message: message.into(),
        });
    }
}
