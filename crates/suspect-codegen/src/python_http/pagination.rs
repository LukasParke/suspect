//! Emitted-only pagination iteration. The shared, generation-time
//! `http_protocol::plan_pagination` outcome compiles into per-operation page and
//! item generator methods on the generated `Client`/`AsyncClient` classes plus a
//! small module-level helper section — all inside the generated `_client.py`.
//! Static runtime files and the shared planner stay untouched, and plans
//! without configured client defaults (or without paginated operations) emit no
//! new bytes at all.
//!
//! Continuation rules follow the compiled entry exactly: `ItemsReturned`
//! advances an integer offset by the previous page's item count; `NextOffset`
//! applies the resolved next pointer to the pattern's continuation parameter.
//! Only pagination controls change between requests — every other keyword
//! argument is preserved exactly, and caller-supplied controls win for the
//! first request. Generators are lazy, never prefetch, and a repeated identical
//! continuation value raises `SdkError('pagination-stalled')` instead of
//! looping.

use std::collections::{BTreeMap, BTreeSet};

use super::{HttpPlan, PlannedOperation, allocate};
use crate::http_protocol as p;
use crate::python_models::{PyDecl, PyType};
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};
use serde_json::Value;
use suspect_ir::contract::SchemaId;

/// Extra `_client.py` imports; pushed only when a paginated operation exists.
pub(super) const IMPORTS: &str = "from collections.abc import AsyncIterator, Iterator\nfrom typing import Any, Literal, Union\nfrom ._runtime import SdkError\n";

/// Module-level helper section emitted into `_client.py` before the clients.
pub(super) const HELPERS: &str = "\n\ndef _pagination_field(value: object, name: str) -> object:\n    \"\"\"Read one declared field or plain-dict key from a decoded response value.\"\"\"\n    if isinstance(value, dict):\n        return value.get(name)\n    return getattr(value, name, None)\n\n\ndef _pagination_count(value: object) -> int:\n    \"\"\"Count items at a collection pointer; uncountable values count as zero.\"\"\"\n    if isinstance(value, (list, tuple, dict, str, bytes)):\n        return len(value)\n    return 0\n";

fn q(text: &str) -> String {
    super::native_examples::quote(text)
}

/// A single-quoted Python literal for generated identifier-safe strings.
fn sq(text: &str) -> String {
    let mut out = String::from("'");
    for character in text.chars() {
        match character {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(character),
        }
    }
    out.push('\'');
    out
}

/// One paginated operation's allocated emission inputs.
pub(super) struct PaginatedMethod<'a> {
    pub index: usize,
    pub page: &'a p::OperationPagination,
    pub pages_name: String,
    pub items_name: String,
}

/// The shared planner's operation identity for one planned operation:
/// the operation id, or `METHOD /path` when unnamed.
fn identity(op: &PlannedOperation) -> String {
    op.wire()
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| format!("{} {}", op.wire().method().as_str(), op.wire().path()))
}

/// The request role that receives the computed continuation value, preferring
/// the pattern's own control and falling back to any declared pagination role.
fn apply_role(page: &p::OperationPagination) -> Option<PaginationRequestRole> {
    let candidates: &[PaginationRequestRole] = match page.pattern {
        PaginationPattern::Cursor => &[
            PaginationRequestRole::Cursor,
            PaginationRequestRole::Offset,
            PaginationRequestRole::Page,
        ],
        PaginationPattern::PageNumber => &[
            PaginationRequestRole::Page,
            PaginationRequestRole::Offset,
            PaginationRequestRole::Cursor,
        ],
        PaginationPattern::LimitOffset | PaginationPattern::NextLink => &[
            PaginationRequestRole::Offset,
            PaginationRequestRole::Page,
            PaginationRequestRole::Cursor,
        ],
    };
    candidates
        .iter()
        .copied()
        .find(|role| page.request.contains_key(role))
}

/// The response pointer carrying the next continuation value.
fn continuation_pointer(page: &p::OperationPagination) -> Option<String> {
    let roles: &[PaginationResponseRole] = match page.pattern {
        PaginationPattern::Cursor => &[
            PaginationResponseRole::NextCursor,
            PaginationResponseRole::NextOffset,
        ],
        _ => &[
            PaginationResponseRole::NextOffset,
            PaginationResponseRole::NextCursor,
        ],
    };
    roles
        .iter()
        .find_map(|role| page.response.get(role).cloned())
}

/// Whether the compiled entry carries a complete runtime continuation rule:
/// an applicable request role and, for pointer advances, a response pointer.
/// Next-link following needs declared URL/credential policy and is not emitted.
fn applicable(page: &p::OperationPagination) -> bool {
    match page.advance {
        PaginationAdvance::ItemsReturned => {
            page.request.contains_key(&PaginationRequestRole::Offset)
                && page.response.contains_key(&PaginationResponseRole::Items)
        }
        PaginationAdvance::NextOffset => {
            apply_role(page).is_some() && continuation_pointer(page).is_some()
        }
    }
}

/// Every planned operation with an emitted pagination iterator, in plan order,
/// with collision-allocated method names shared by both client flavors.
pub(super) fn paginated_operations(plan: &HttpPlan) -> Vec<PaginatedMethod<'_>> {
    let Some(outcome) = plan.pagination() else {
        return Vec::new();
    };
    if outcome.paginated.is_empty() {
        return Vec::new();
    }
    let mut used: BTreeSet<String> = [
        "close",
        "aclose",
        "_call",
        "_prepare",
        "_exchange",
        "_configure",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    used.extend(plan.operations().iter().map(|op| op.snake_name.clone()));
    let mut result = Vec::new();
    for (index, op) in plan.operations().iter().enumerate() {
        let identity = identity(op);
        let Some(page) = outcome
            .paginated
            .iter()
            .find(|page| page.operation == identity)
        else {
            continue;
        };
        if !applicable(page) {
            continue;
        }
        let pages_name = allocate(&format!("iter_{}_pages", op.snake_name), &mut used);
        let items_name = allocate(&format!("iter_{}_items", op.snake_name), &mut used);
        result.push(PaginatedMethod {
            index,
            page,
            pages_name,
            items_name,
        });
    }
    result
}

/// The decoded-body schema of the operation's first JSON success response —
/// the same response the shared planner compiled pointers against.
fn success_body(op: &PlannedOperation) -> Option<SchemaId> {
    for response in op.responses() {
        if !matches!(
            response.wire().status(),
            p::ResponseStatus::Exact(200..=299) | p::ResponseStatus::Range(2)
        ) {
            continue;
        }
        for media in &response.media {
            if let p::Representation::Json { codec: Some(codec) } = media.wire.representation() {
                return Some(codec.schema().id().clone());
            }
        }
    }
    None
}

/// Unwrap absent-value wrappers around a value type.
fn strip(ty: &PyType) -> PyType {
    match ty {
        PyType::Optional(inner) | PyType::Nullable(inner) => strip(inner),
        other => other.clone(),
    }
}

/// The Python value produced for one model schema: a declared dataclass or the
/// aliased value behind it. Undeclared schemas decode as plain JSON values.
fn value_type(plan: &HttpPlan, schema: &SchemaId) -> PyType {
    match plan.codecs().models().declarations().get(schema) {
        Some(PyDecl::Alias(ty)) => strip(ty),
        _ => PyType::Named(schema.clone()),
    }
}

/// Resolve one wire-named field of a value type into its allocated native
/// attribute name and the value's own type, following alias declarations.
fn field(plan: &HttpPlan, value: &PyType, wire: &str) -> Option<(String, PyType)> {
    let id = match strip(value) {
        PyType::Named(id) => id,
        _ => return None,
    };
    match plan.codecs().models().declarations().get(&id)? {
        PyDecl::Dataclass { fields, .. } => {
            let field = fields.iter().find(|field| field.wire == wire)?;
            Some((field.name.clone(), strip(&field.ty)))
        }
        PyDecl::Alias(ty) => field(plan, &strip(ty), wire),
    }
}

/// Preferred access names for one response pointer against the decoded body:
/// declared dataclass fields use their native attribute name; anything else
/// keeps the exact wire name for plain-dict access at runtime.
fn access_names(plan: &HttpPlan, body: &SchemaId, pointer: &str) -> Vec<String> {
    let mut names = Vec::new();
    if pointer.is_empty() {
        return names;
    }
    let mut current = Some(value_type(plan, body));
    for segment in pointer[1..].split('/') {
        let wire = segment.replace("~1", "/").replace("~0", "~");
        match current.as_ref().and_then(|ty| field(plan, ty, &wire)) {
            Some((name, next)) => {
                names.push(name);
                current = Some(next);
            }
            None => {
                names.push(wire);
                current = None;
            }
        }
    }
    names
}

/// The Python annotation for one item of the collection pointer, when
/// statically known. `Any` covers unknown shapes without weakening pages.
fn item_annotation(
    plan: &HttpPlan,
    body: &SchemaId,
    pointer: &str,
    symbols: &BTreeMap<SchemaId, String>,
) -> String {
    let mut current = Some(value_type(plan, body));
    if !pointer.is_empty() {
        for segment in pointer[1..].split('/') {
            let wire = segment.replace("~1", "/").replace("~0", "~");
            current = current
                .as_ref()
                .and_then(|ty| field(plan, ty, &wire))
                .map(|(_, next)| next);
        }
    }
    match current.as_ref().map(strip) {
        Some(PyType::List(inner)) => render_type(inner.as_ref(), symbols),
        _ => "Any".into(),
    }
}

fn literal(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(number) => number.to_string(),
        other => q(&other.to_string()),
    }
}

/// Render a value type as a `_client.py` annotation using that module's imports.
fn render_type(ty: &PyType, symbols: &BTreeMap<SchemaId, String>) -> String {
    match ty {
        PyType::Primitive(name) => match *name {
            "types.NoneType" => "None".into(),
            "_json.JsonNumber" => "JsonNumber".into(),
            other => (*other).to_owned(),
        },
        PyType::JsonValue => "JsonValue".into(),
        PyType::Named(id) => format!("models.{}", symbols[id]),
        PyType::Nullable(inner) => format!("{} | None", render_type(inner, symbols)),
        PyType::Optional(inner) => format!("{} | Unset", render_type(inner, symbols)),
        PyType::List(inner) => format!("list[{}]", render_type(inner, symbols)),
        PyType::Map(inner) => format!("dict[str, {}]", render_type(inner, symbols)),
        PyType::Literal(values) => format!(
            "Literal[{}]",
            values.iter().map(literal).collect::<Vec<_>>().join(", ")
        ),
        PyType::Union(types) => {
            let rendered = types
                .iter()
                .map(|(_, ty)| render_type(ty, symbols))
                .collect::<Vec<_>>();
            if rendered.iter().any(|rendered| rendered == "None") {
                format!("Union[{}]", rendered.join(", "))
            } else {
                rendered.join(" | ")
            }
        }
    }
}

/// A decoded-body accessor expression for one resolved pointer.
fn access(base: &str, names: &[String]) -> String {
    let mut expression = base.to_owned();
    for name in names {
        expression = format!("_pagination_field({expression}, {})", sq(name));
    }
    expression
}

/// The allocated native parameter name for one role's wire parameter.
fn native_parameter<'a>(
    op: &'a PlannedOperation,
    role: PaginationRequestRole,
    page: &p::OperationPagination,
) -> Option<&'a str> {
    let wire = page.request.get(&role)?;
    op.parameters()
        .iter()
        .find(|parameter| parameter.protocol().name() == wire)
        .map(|parameter| parameter.name.as_str())
}

fn pages_docstring(
    op: &PlannedOperation,
    page: &p::OperationPagination,
    control: &str,
    asynchronous: bool,
) -> String {
    let fetch = if asynchronous { "__anext__" } else { "next()" };
    let stop = match page.advance {
        PaginationAdvance::ItemsReturned => {
            if page.response.contains_key(&PaginationResponseRole::HasMore) {
                "after a page with no items or when the has-more indicator is false"
            } else {
                "after a page with no items"
            }
        }
        PaginationAdvance::NextOffset => match page.pattern {
            PaginationPattern::Cursor => {
                "when the next cursor token is absent, None or an empty string"
            }
            _ => "when the next pointer is absent or None",
        },
    };
    let description = op.description();
    let head = if description.is_empty() {
        String::new()
    } else {
        format!("{description}\n\n")
    };
    let fallback = page.initial_limit.map_or_else(String::new, |size| {
        format!(
            " When the caller omits the limit, the first request supplies the documented SDK page size ({size}); later pages keep the limit the walk last used."
        )
    });
    format!(
        "{head}Lazily yields every success page of {}. The first page is the direct {}(...) result, fetched on the first {} and included exactly once. Between requests only pagination controls change: every other keyword argument is preserved exactly, and a caller-supplied {} is used for the first request and replaced by the computed continuation afterwards.{fallback} Stops {}. A repeated identical continuation value raises SdkError('pagination-stalled') instead of looping. Iteration is lazy: no request happens before the first {}, breaking out or closing the generator prevents further requests, and pages are never prefetched.\nSource: {}#{}",
        page.operation,
        op.snake_name,
        fetch,
        control,
        stop,
        fetch,
        op.source.document(),
        op.source.pointer()
    )
}

fn items_docstring(op: &PlannedOperation, pages_name: &str) -> String {
    format!(
        "Flattens the page items of {}(...) across every page, in order. Accepts the same keyword arguments as the direct call; see {} for lazy fetching, continuation, stopping and caller-override semantics.\nSource: {}#{}",
        op.snake_name,
        pages_name,
        op.source.document(),
        op.source.pointer()
    )
}

/// The per-client pagination iterator methods for one paginated operation.
pub(super) fn methods(
    plan: &HttpPlan,
    op: &PlannedOperation,
    page: &p::OperationPagination,
    index: usize,
    pages_name: &str,
    items_name: &str,
    asynchronous: bool,
) -> String {
    let Some(body) = success_body(op) else {
        return String::new();
    };
    let symbols = plan.symbols();
    let prefix = if asynchronous { "async " } else { "" };
    let awaited = if asynchronous { "await " } else { "" };
    let iterator = if asynchronous {
        "AsyncIterator"
    } else {
        "Iterator"
    };
    let success = if asynchronous {
        op.async_success_type.as_str()
    } else {
        op.success_type.as_str()
    };
    let items_pointer = page
        .response
        .get(&PaginationResponseRole::Items)
        .cloned()
        .unwrap_or_default();
    let items_expression = access("page.data", &access_names(plan, &body, &items_pointer));
    let item_type = item_annotation(plan, &body, &items_pointer, symbols);
    let call = format!("page = {awaited}self.{}(**request)", op.snake_name);
    // The documented SDK fallback page size fills the limit control on page 1
    // only when the caller omitted it; later pages keep the walk's own limit
    // because the continuation never touches the limit member.
    let limit_fallback = match page.initial_limit {
        Some(size) => match native_parameter(op, PaginationRequestRole::Limit, page) {
            Some(limit) => format!(
                "        limit = request.get({})\n        if limit is None or isinstance(limit, Unset):\n            request[{}] = {size}\n",
                sq(limit),
                sq(limit)
            ),
            None => String::new(),
        },
        None => String::new(),
    };
    let walk = match page.advance {
        PaginationAdvance::ItemsReturned => {
            let Some(offset) = native_parameter(op, PaginationRequestRole::Offset, page) else {
                return String::new();
            };
            let offset_key = sq(offset);
            let initial = page.initial_offset.unwrap_or(0);
            let injection = if page.initial_offset.is_some() {
                format!("            request[{offset_key}] = offset\n")
            } else {
                String::new()
            };
            let mut walk = format!(
                "        request: dict[str, Any] = dict(kwargs)\n{limit_fallback}        offset = request.get({offset_key})\n        if offset is None or isinstance(offset, Unset):\n            offset = {initial}\n{injection}        while True:\n            {call}\n            yield page\n            count = _pagination_count({items_expression})\n            if count == 0:\n                return\n"
            );
            if let Some(pointer) = page.response.get(&PaginationResponseRole::HasMore) {
                let expression = access("page.data", &access_names(plan, &body, pointer));
                walk.push_str(&format!(
                    "            if {expression} is False:\n                return\n"
                ));
            }
            walk.push_str(&format!(
                "            offset = offset + count\n            request[{offset_key}] = offset\n"
            ));
            walk
        }
        PaginationAdvance::NextOffset => {
            let Some(role) = apply_role(page) else {
                return String::new();
            };
            let Some(apply) = native_parameter(op, role, page) else {
                return String::new();
            };
            let Some(pointer) = continuation_pointer(page) else {
                return String::new();
            };
            let apply_key = sq(apply);
            let expression = access("page.data", &access_names(plan, &body, &pointer));
            let absent = if page.pattern == PaginationPattern::Cursor {
                "value is None or isinstance(value, Unset) or value == ''"
            } else {
                "value is None or isinstance(value, Unset)"
            };
            let mut walk = format!(
                "        request: dict[str, Any] = dict(kwargs)\n{limit_fallback}        previous = request.get({apply_key})\n        if isinstance(previous, Unset):\n            previous = None\n"
            );
            if matches!(
                role,
                PaginationRequestRole::Offset | PaginationRequestRole::Page
            ) && let Some(initial) = page.initial_offset
            {
                walk.push_str(&format!(
                    "        if previous is None:\n            previous = {initial}\n            request[{apply_key}] = previous\n"
                ));
            }
            walk.push_str(&format!(
                "        while True:\n            {call}\n            yield page\n            value = {expression}\n            if {absent}:\n                return\n            if value == previous:\n                raise SdkError('pagination-stalled', _operation({index}).source, code='pagination-stalled')\n            previous = value\n            request[{apply_key}] = value\n"
            ));
            walk
        }
    };
    let control = match page.advance {
        PaginationAdvance::ItemsReturned => {
            native_parameter(op, PaginationRequestRole::Offset, page)
                .unwrap_or("offset")
                .to_owned()
        }
        PaginationAdvance::NextOffset => apply_role(page)
            .and_then(|role| native_parameter(op, role, page))
            .unwrap_or("cursor")
            .to_owned(),
    };
    let pages_doc = q(&pages_docstring(op, page, &control, asynchronous));
    let mut code = format!(
        "\n    {prefix}def {pages_name}(self, **kwargs: Any) -> {iterator}[operations.{success}]:\n        {pages_doc}\n{walk}"
    );
    let items_doc = q(&items_docstring(op, pages_name));
    let items_body = if asynchronous {
        format!(
            "        async for page in self.{pages_name}(**kwargs):\n            items = {items_expression}\n            if isinstance(items, list):\n                for item in items:\n                    yield item\n"
        )
    } else {
        format!(
            "        for page in self.{pages_name}(**kwargs):\n            items = {items_expression}\n            if isinstance(items, list):\n                yield from items\n"
        )
    };
    code.push_str(&format!(
        "\n    {prefix}def {items_name}(self, **kwargs: Any) -> {iterator}[{item_type}]:\n        {items_doc}\n{items_body}"
    ));
    code
}
