//! Generation-time pagination detection and manual mapping. Detection runs
//! against resolved, effective query parameters and success response schemas
//! using a deterministic normalized-key comparison; it never guesses with edit
//! distance, never emits runtime schema search, and records a source-linked
//! explanation for every ordinary single-page operation.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;
use suspect_ir::contract::{Contract, SchemaId, SourceId};

use super::Provenance;
use super::model::{
    ParameterLocation, ParameterPlan, ProtocolPlan, Representation, ResponseStatus, ScalarType,
    WireShape,
};
pub use crate::http_contract::HttpDiagnostic;
pub use crate::sdk_defaults::{
    PaginationAdvance, PaginationAliasRole, PaginationMapping, PaginationMode, PaginationOperation,
    PaginationPattern, PaginationRequestRole, PaginationResponseRole, SdkDefaults,
};

/// One operation's compiled pagination behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperationPagination {
    /// Operation identifier, or `METHOD /path` when unnamed.
    pub operation: String,
    pub pattern: PaginationPattern,
    /// Role -> actual request parameter name (query parameters in v1).
    pub request: BTreeMap<PaginationRequestRole, String>,
    /// Role -> JSON Pointer (RFC 6901) into the decoded response body.
    pub response: BTreeMap<PaginationResponseRole, String>,
    pub initial_offset: Option<u32>,
    /// The documented SDK fallback page size (`sdk_defaults.pagination.page_size`)
    /// applied to the first request only, and only when the caller omits the
    /// limit-role parameter. It is an SDK fallback convention, never a claim
    /// about a server default: when the source declares a default for the
    /// limit parameter this stays `None` so the server default is honored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_limit: Option<u32>,
    pub advance: PaginationAdvance,
    /// The source operation declaration that motivated this selection.
    pub source: Provenance,
}

/// Why an operation stays an ordinary single-page call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SinglePageExplanation {
    pub operation: String,
    pub reason: String,
}

/// Complete generation-time pagination selection for one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct PaginationOutcome {
    pub paginated: Vec<OperationPagination>,
    pub single_page: Vec<SinglePageExplanation>,
}

fn diagnostic(
    contract: &Contract,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    let source = SourceId::new(contract.entry().clone(), Default::default());
    let at = contract.source_span(&source).unwrap_or(0..0);
    HttpDiagnostic {
        source,
        at,
        code,
        message: message.into(),
    }
}

/// A diagnostic anchored to the operation declaration it is about.
fn operation_diagnostic(
    contract: &Contract,
    operation: &super::model::OperationPlan,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    let source = operation.source().use_site().source().clone();
    let at = contract.source_span(&source).unwrap_or(0..0);
    HttpDiagnostic {
        source,
        at,
        code,
        message: message.into(),
    }
}

/// Deterministic normalized key: lowercase without `_` and `-` separators.
fn normalize(name: &str) -> String {
    name.chars()
        .map(|character| character.to_ascii_lowercase())
        .filter(|character| *character != '_' && *character != '-')
        .collect()
}

const LIMIT_KEYS: &[&str] = &["limit", "pagesize", "perpage", "take", "maxresults"];
const OFFSET_KEYS: &[&str] = &["offset", "skip", "startindex"];
const CURSOR_KEYS: &[&str] = &[
    "cursor",
    "pagetoken",
    "continuationtoken",
    "after",
    "startingafter",
];
const PAGE_KEYS: &[&str] = &["page", "pagenumber"];
const NEXT_CURSOR_KEYS: &[&str] = &["nextcursor", "nextpagetoken", "nexttoken", "continuation"];
const TOTAL_KEYS: &[&str] = &["total", "totalcount", "totalitems"];

/// The observed evidence for one success response shape.
#[derive(Debug, Default)]
struct ResponseShape {
    /// Pointer to the collection: `""` for a top-level array, `/name` for a
    /// recognized array-valued envelope field.
    items: Option<String>,
    next_cursor: Option<String>,
    next_offset: Option<String>,
    has_more: Option<String>,
    total: Option<String>,
    total_pages: Option<String>,
}

fn parameter_scalar(parameter: &ParameterPlan) -> Option<ScalarType> {
    match parameter.serialization() {
        super::model::ParameterSerialization::Style { shape, .. } => match shape {
            WireShape::Scalar { scalar } => Some(*scalar),
            _ => None,
        },
        super::model::ParameterSerialization::Content { .. } => None,
    }
}

/// Follow indexed `$ref` edges to one non-reference schema value.
fn follow<'a>(contract: &'a Contract, id: &SchemaId) -> Option<&'a Value> {
    let mut current = id.clone();
    for _ in 0..64 {
        let view = contract.schema(&current)?;
        current = view
            .references()
            .iter()
            .find(|reference| reference.keyword == "$ref")
            .and_then(|reference| reference.target.clone())?;
    }
    None
}

/// The resolved raw value of one schema, following its reference chain.
fn schema_value<'a>(contract: &'a Contract, id: &SchemaId) -> Option<&'a Value> {
    let view = contract.schema(id)?;
    let raw = view.raw();
    if raw.get("$ref").is_some() {
        follow(contract, id)
    } else {
        Some(raw)
    }
}

/// Array-collection vocabulary used by `response_shape`.
const ITEMS_KEYS: &[&str] = &["data", "items", "results", "records"];

struct QueryParameters<'a> {
    by_normalized: BTreeMap<String, &'a ParameterPlan>,
}

impl<'a> QueryParameters<'a> {
    fn collect(operation: &'a super::model::OperationPlan) -> Self {
        let mut by_normalized = BTreeMap::new();
        for parameter in operation.parameters() {
            if parameter.location() == ParameterLocation::Query {
                by_normalized.insert(normalize(parameter.name()), parameter);
            }
        }
        Self { by_normalized }
    }

    /// One unique parameter whose normalized name matches any key with a
    /// compatible scalar type. Two competing candidates of one role are
    /// ambiguity, never a match.
    fn unique(
        &self,
        keys: &[&str],
        aliases: &BTreeSet<String>,
        expected: Option<ScalarType>,
    ) -> Result<Option<&'a ParameterPlan>, Ambiguity> {
        let mut found: Option<&'a ParameterPlan> = None;
        for key in keys
            .iter()
            .copied()
            .chain(aliases.iter().map(String::as_str))
        {
            let Some(parameter) = self.by_normalized.get(key) else {
                continue;
            };
            if let Some(expected) = expected
                && parameter_scalar(parameter) != Some(expected)
            {
                continue;
            }
            if let Some(previous) = found {
                if previous.name() != parameter.name() {
                    return Err(Ambiguity);
                }
                continue;
            }
            found = Some(*parameter);
        }
        Ok(found)
    }
}

struct Ambiguity;

/// Operation display identity: operation id, or `METHOD /path`.
fn operation_identity(operation: &super::model::OperationPlan) -> String {
    operation
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| format!("{} {}", operation.method().as_str(), operation.path()))
}

fn operation_override_key(operation: &super::model::OperationPlan) -> String {
    format!("{} {}", operation.method().as_str(), operation.path())
}

/// Detect and compile pagination for every selected operation.
///
/// # Errors
/// Per-operation overrides that name absent parameters, point outside the
/// response schema, or map one name to two roles. Inferred ambiguity is never
/// an error: it is a recorded single-page explanation.
pub fn plan(
    contract: &Contract,
    protocol: &ProtocolPlan,
    defaults: Option<&SdkDefaults>,
) -> Result<PaginationOutcome, Vec<HttpDiagnostic>> {
    let Some(defaults) = defaults else {
        return Ok(PaginationOutcome::default());
    };
    let pagination = &defaults.pagination;
    let mut aliases: BTreeMap<PaginationAliasRole, BTreeSet<String>> = BTreeMap::new();
    for (role, names) in &pagination.aliases {
        aliases.insert(*role, names.iter().map(|name| normalize(name)).collect());
    }
    let alias = |role: PaginationAliasRole| aliases.get(&role).cloned().unwrap_or_default();
    let limit_aliases = alias(PaginationAliasRole::Limit);
    let offset_aliases = alias(PaginationAliasRole::Offset);
    let cursor_aliases = alias(PaginationAliasRole::Cursor);
    let page_aliases = alias(PaginationAliasRole::Page);

    let mut outcome = PaginationOutcome::default();
    let mut errors = Vec::new();
    for operation in protocol.operations() {
        let identity = operation_identity(operation);
        let override_entry = operation
            .operation_id()
            .and_then(|located| pagination.operations.get(located.value().as_str()))
            .or_else(|| {
                pagination
                    .operations
                    .get(&operation_override_key(operation))
            });
        let Some(selection) = override_entry else {
            if pagination.mode == PaginationMode::Off {
                outcome.single_page.push(SinglePageExplanation {
                    operation: identity,
                    reason: "pagination detection is disabled by configuration".into(),
                });
                continue;
            }
            // Automatic detection only considers the configured pattern
            // subset; per-operation mappings below are unaffected.
            let matched = detect(
                &limit_aliases,
                &offset_aliases,
                &cursor_aliases,
                &page_aliases,
                contract,
                operation,
            );
            let admitted = |pattern: PaginationPattern| {
                pagination
                    .patterns
                    .as_ref()
                    .is_none_or(|subset| subset.contains(&pattern))
            };
            match matched.iter().find(|detected| admitted(detected.pattern)) {
                Some(detected) => {
                    let mut compiled = detected.compiled.clone();
                    match fallback_limit(contract, operation, &compiled, pagination.page_size) {
                        Ok(limit) => compiled.initial_limit = limit,
                        Err(message) => errors.push(operation_diagnostic(
                            contract,
                            operation,
                            "sdk-pagination-page-size",
                            message,
                        )),
                    }
                    outcome.paginated.push(compiled);
                }
                None if matched.is_empty() => outcome.single_page.push(SinglePageExplanation {
                    operation: identity,
                    reason: single_page_reason(contract, operation),
                }),
                None => outcome.single_page.push(SinglePageExplanation {
                    operation: identity,
                    reason: restricted_reason(&matched),
                }),
            }
            continue;
        };
        match selection {
            PaginationOperation::Disabled => {
                outcome.single_page.push(SinglePageExplanation {
                    operation: identity,
                    reason: "pagination is explicitly disabled for this operation".into(),
                });
            }
            PaginationOperation::Mapping(mapping) => {
                match compile_mapping(contract, operation, mapping) {
                    Ok(compiled) => {
                        if let Err(message) =
                            fallback_limit(contract, operation, &compiled, pagination.page_size)
                        {
                            errors.push(operation_diagnostic(
                                contract,
                                operation,
                                "sdk-pagination-page-size",
                                message,
                            ));
                        } else {
                            outcome.paginated.push(compiled);
                        }
                    }
                    Err(message) => errors.push(diagnostic(
                        contract,
                        "sdk-pagination-override",
                        format!("pagination.operations[{identity}]: {message}"),
                    )),
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(outcome)
    } else {
        Err(errors)
    }
}

fn required_scalar(role: PaginationRequestRole) -> Option<ScalarType> {
    match role {
        PaginationRequestRole::Limit
        | PaginationRequestRole::Offset
        | PaginationRequestRole::Page => Some(ScalarType::Integer),
        PaginationRequestRole::Cursor => Some(ScalarType::String),
    }
}

fn success_shape(
    contract: &Contract,
    operation: &super::model::OperationPlan,
) -> Option<ResponseShape> {
    let response = operation.responses().iter().find(|response| {
        matches!(
            response.status(),
            ResponseStatus::Exact(200..=299) | ResponseStatus::Range(2)
        )
    })?;
    let media = response.media().iter().find(|media| {
        matches!(
            media.representation(),
            Representation::Json { codec: Some(_) }
        )
    })?;
    let Representation::Json { codec } = media.representation() else {
        return None;
    };
    response_shape(contract, codec.as_ref()?.schema().id())
}

/// Inspect one success response body for pagination evidence.
fn response_shape(contract: &Contract, root: &SchemaId) -> Option<ResponseShape> {
    let raw = schema_value(contract, root)?;
    let mut shape = ResponseShape::default();
    if raw.get("type").and_then(Value::as_str) == Some("array") {
        shape.items = Some(String::new());
        return Some(shape);
    }
    let properties = raw.get("properties")?.as_object()?;
    for name in properties.keys() {
        let pointer = format!("/{name}");
        let normalized = normalize(name);
        let property_id = root.child("properties").child(name);
        let Some(property) = schema_value(contract, &property_id) else {
            continue;
        };
        match property.get("type").and_then(Value::as_str) {
            Some("array") => {
                if shape.items.is_none() && ITEMS_KEYS.contains(&normalized.as_str()) {
                    shape.items = Some(pointer);
                }
            }
            Some("string") => {
                if shape.next_cursor.is_none() && NEXT_CURSOR_KEYS.contains(&normalized.as_str()) {
                    shape.next_cursor = Some(pointer);
                }
            }
            Some("integer") => {
                if normalized == "nextoffset" {
                    if shape.next_cursor.is_none() {
                        shape.next_cursor = Some(pointer.clone());
                    }
                    if shape.next_offset.is_none() {
                        shape.next_offset = Some(pointer.clone());
                    }
                }
                if shape.total.is_none() && TOTAL_KEYS.contains(&normalized.as_str()) {
                    shape.total = Some(pointer.clone());
                }
                if shape.total_pages.is_none() && normalized == "totalpages" {
                    shape.total_pages = Some(pointer);
                }
            }
            Some("boolean") if shape.has_more.is_none() && normalized == "hasmore" => {
                shape.has_more = Some(pointer);
            }
            _ => {}
        }
    }
    Some(shape)
}

fn compile_mapping(
    contract: &Contract,
    operation: &super::model::OperationPlan,
    mapping: &PaginationMapping,
) -> Result<OperationPagination, String> {
    let query = QueryParameters::collect(operation);
    let mut request = BTreeMap::new();
    let mut response = BTreeMap::new();
    for (role, name) in &mapping.request {
        let parameter = query
            .by_normalized
            .get(&normalize(name))
            .or_else(|| query.by_normalized.get(name.as_str()))
            .ok_or_else(|| {
                format!("request.{role:?} names {name:?}, which is not a query parameter of this operation")
            })?;
        if let Some(expected) = required_scalar(*role)
            && parameter_scalar(parameter) != Some(expected)
        {
            return Err(format!(
                "request.{role:?} parameter {name:?} must be {expected:?}-typed on the wire"
            ));
        }
        request.insert(*role, parameter.name().to_owned());
    }
    if let Some(root) = success_root(contract, operation) {
        for (role, pointer) in &mapping.response {
            let Some(target) = pointer_target(contract, &root, pointer) else {
                return Err(format!(
                    "response.{role:?} pointer {pointer:?} does not resolve within the declared success schema"
                ));
            };
            let declared = target
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let acceptable = match role {
                PaginationResponseRole::Items => {
                    declared.as_deref().is_none_or(|kind| kind == "array")
                }
                PaginationResponseRole::NextCursor => declared
                    .as_deref()
                    .is_none_or(|kind| matches!(kind, "string" | "integer")),
                PaginationResponseRole::NextOffset
                | PaginationResponseRole::Total
                | PaginationResponseRole::TotalPages => {
                    declared.as_deref().is_none_or(|kind| kind == "integer")
                }
                PaginationResponseRole::HasMore => {
                    declared.as_deref().is_none_or(|kind| kind == "boolean")
                }
            };
            if !acceptable {
                return Err(format!(
                    "response.{role:?} pointer {pointer:?} resolves to a {declared:?} value, which cannot serve that role"
                ));
            }
            response.insert(*role, pointer.clone());
        }
    }
    let advance = mapping.advance.unwrap_or(match mapping.pattern {
        PaginationPattern::LimitOffset => PaginationAdvance::ItemsReturned,
        PaginationPattern::Cursor | PaginationPattern::PageNumber => PaginationAdvance::NextOffset,
        PaginationPattern::NextLink => PaginationAdvance::NextOffset,
    });
    Ok(OperationPagination {
        operation: operation_identity(operation),
        pattern: mapping.pattern,
        request,
        response,
        initial_offset: mapping.initial_offset,
        initial_limit: None,
        advance,
        source: operation.source().clone(),
    })
}

/// The schema id of the operation's first JSON success body, when one exists.
fn success_root(_contract: &Contract, operation: &super::model::OperationPlan) -> Option<SchemaId> {
    let response = operation.responses().iter().find(|response| {
        matches!(
            response.status(),
            ResponseStatus::Exact(200..=299) | ResponseStatus::Range(2)
        )
    })?;
    let media = response.media().iter().find(|media| {
        matches!(
            media.representation(),
            Representation::Json { codec: Some(_) }
        )
    })?;
    let Representation::Json { codec } = media.representation() else {
        return None;
    };
    Some(codec.as_ref()?.schema().id().clone())
}

/// Resolve one RFC 6901 pointer through declared object properties. Every
/// segment must name a declared property; a pointer to nowhere is refused.
fn pointer_target<'a>(contract: &'a Contract, root: &SchemaId, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() {
        return schema_value(contract, root);
    }
    let mut current = root.clone();
    for segment in pointer[1..].split('/') {
        let unescaped = segment.replace("~1", "/").replace("~0", "~");
        let raw = schema_value(contract, &current)?;
        raw.get("properties")?.get(&unescaped)?;
        current = current.child("properties").child(&unescaped);
    }
    schema_value(contract, &current)
}

/// One builtin pattern's structural match against one operation.
struct Detected {
    pattern: PaginationPattern,
    compiled: OperationPagination,
}

/// The kebab-case configuration spelling of one pattern.
fn pattern_name(pattern: PaginationPattern) -> &'static str {
    match pattern {
        PaginationPattern::LimitOffset => "limit-offset",
        PaginationPattern::Cursor => "cursor",
        PaginationPattern::PageNumber => "page-number",
        PaginationPattern::NextLink => "next-link",
    }
}

/// Why a matched operation stays single-page under a `patterns` restriction:
/// every builtin pattern that matched, named as configured.
fn restricted_reason(matched: &[Detected]) -> String {
    let names = matched
        .iter()
        .map(|detected| pattern_name(detected.pattern))
        .collect::<Vec<_>>()
        .join(", ");
    let verb = if matched.len() == 1 { "is" } else { "are" };
    format!(
        "the {names} pattern{} matched this operation but {verb} excluded by sdk_defaults.pagination.patterns",
        if matched.len() == 1 { "" } else { "s" }
    )
}

/// The documented SDK fallback page size for one compiled operation.
///
/// `initial_limit` applies only when the compiled request carries a limit
/// role and the source declares no default for that parameter: a declared
/// source default is honored by recording none, so the helper fallback never
/// masquerades as a server default. A configured size that violates a
/// declared upper bound on the parameter is a generation error instead of a
/// request the source would refuse.
fn fallback_limit(
    contract: &Contract,
    operation: &super::model::OperationPlan,
    page: &OperationPagination,
    page_size: Option<u32>,
) -> Result<Option<u32>, String> {
    let Some(size) = page_size else {
        return Ok(None);
    };
    let Some(name) = page.request.get(&PaginationRequestRole::Limit) else {
        return Ok(None);
    };
    let Some(parameter) = operation.parameters().iter().find(|parameter| {
        parameter.location() == ParameterLocation::Query && parameter.name() == name.as_str()
    }) else {
        return Ok(None);
    };
    let Some(raw) = schema_value(contract, parameter.codec().schema().id()) else {
        return Ok(None);
    };
    if raw.get("default").is_some() {
        // The source declares its own default for the limit parameter; the
        // documented convention defers to it and records no helper fallback.
        return Ok(None);
    }
    if let Some(maximum) = raw.get("maximum").and_then(Value::as_f64)
        && maximum < f64::from(size)
    {
        return Err(format!(
            "page_size {size} exceeds the declared maximum {maximum} of the {name} limit parameter"
        ));
    }
    if let Some(exclusive) = raw.get("exclusiveMaximum").and_then(Value::as_f64)
        && exclusive <= f64::from(size)
    {
        return Err(format!(
            "page_size {size} is not below the declared exclusive maximum {exclusive} of the {name} limit parameter"
        ));
    }
    Ok(Some(size))
}

/// Builtin pattern detection with documented evidence requirements: a cursor
/// continuation beats a coincidental limit/offset pair, and a `limit` query
/// parameter alone is never sufficient. Every structurally matching pattern is
/// reported in priority order so a configured `patterns` subset can restrict
/// which of them is admitted.
fn detect(
    limit_aliases: &BTreeSet<String>,
    offset_aliases: &BTreeSet<String>,
    cursor_aliases: &BTreeSet<String>,
    page_aliases: &BTreeSet<String>,
    contract: &Contract,
    operation: &super::model::OperationPlan,
) -> Vec<Detected> {
    let query = QueryParameters::collect(operation);
    let Some(shape) = success_shape(contract, operation) else {
        return Vec::new();
    };
    let limit = query
        .unique(LIMIT_KEYS, limit_aliases, Some(ScalarType::Integer))
        .ok()
        .flatten();
    let mut matched = Vec::new();
    // Cursor pattern: a string continuation parameter plus a recognized
    // next-cursor response field, both required.
    if let Ok(Some(cursor)) = query.unique(CURSOR_KEYS, cursor_aliases, Some(ScalarType::String))
        && let Some(next_cursor) = &shape.next_cursor
    {
        let mut request = BTreeMap::new();
        request.insert(PaginationRequestRole::Cursor, cursor.name().to_owned());
        if let Some(limit) = limit {
            request.insert(PaginationRequestRole::Limit, limit.name().to_owned());
        }
        let mut response = BTreeMap::new();
        response.insert(PaginationResponseRole::NextCursor, next_cursor.clone());
        if let Some(items) = &shape.items {
            response.insert(PaginationResponseRole::Items, items.clone());
        }
        matched.push(Detected {
            pattern: PaginationPattern::Cursor,
            compiled: OperationPagination {
                operation: operation_identity(operation),
                pattern: PaginationPattern::Cursor,
                request,
                response,
                initial_offset: None,
                initial_limit: None,
                advance: PaginationAdvance::NextOffset,
                source: operation.source().clone(),
            },
        });
    }
    // Limit/offset: size plus position parameters and a collection response.
    if let (Some(limit), Some(offset)) = (
        limit,
        query
            .unique(OFFSET_KEYS, offset_aliases, Some(ScalarType::Integer))
            .ok()
            .flatten(),
    ) && shape.items.is_some()
    {
        let mut request = BTreeMap::new();
        request.insert(PaginationRequestRole::Limit, limit.name().to_owned());
        request.insert(PaginationRequestRole::Offset, offset.name().to_owned());
        let mut response = BTreeMap::new();
        response.insert(
            PaginationResponseRole::Items,
            shape.items.clone().expect("items checked"),
        );
        if let Some(total) = &shape.total {
            response.insert(PaginationResponseRole::Total, total.clone());
        }
        matched.push(Detected {
            pattern: PaginationPattern::LimitOffset,
            compiled: OperationPagination {
                operation: operation_identity(operation),
                pattern: PaginationPattern::LimitOffset,
                request,
                response,
                initial_offset: Some(0),
                initial_limit: None,
                advance: PaginationAdvance::ItemsReturned,
                source: operation.source().clone(),
            },
        });
    }
    // Page number: page plus size, never one parameter doing both jobs.
    if let (Some(page), Some(limit)) = (
        query
            .unique(PAGE_KEYS, page_aliases, Some(ScalarType::Integer))
            .ok()
            .flatten(),
        limit,
    ) && page.name() != limit.name()
        && shape.items.is_some()
    {
        let mut request = BTreeMap::new();
        request.insert(PaginationRequestRole::Page, page.name().to_owned());
        request.insert(PaginationRequestRole::Limit, limit.name().to_owned());
        let mut response = BTreeMap::new();
        response.insert(
            PaginationResponseRole::Items,
            shape.items.clone().expect("items checked"),
        );
        if let Some(total) = &shape.total {
            response.insert(PaginationResponseRole::Total, total.clone());
        }
        if let Some(total_pages) = &shape.total_pages {
            response.insert(PaginationResponseRole::TotalPages, total_pages.clone());
        }
        matched.push(Detected {
            pattern: PaginationPattern::PageNumber,
            compiled: OperationPagination {
                operation: operation_identity(operation),
                pattern: PaginationPattern::PageNumber,
                request,
                response,
                initial_offset: None,
                initial_limit: None,
                advance: PaginationAdvance::NextOffset,
                source: operation.source().clone(),
            },
        });
    }
    matched
}

/// A source-linked explanation for why inference did not select a pattern.
fn single_page_reason(contract: &Contract, operation: &super::model::OperationPlan) -> String {
    let query = QueryParameters::collect(operation);
    if query.by_normalized.is_empty() {
        return "no query parameters to paginate with".into();
    }
    match success_shape(contract, operation) {
        None => "no JSON success response schema to inspect".into(),
        Some(shape) if shape.items.is_none() => {
            "the success response declares no recognized collection field".into()
        }
        Some(_) => "no complete request/response continuation rule matched".into(),
    }
}
