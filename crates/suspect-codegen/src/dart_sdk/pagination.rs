//! Emitted-only pagination streams for the native Dart HTTP adapter.
//!
//! The shared, generation-time `http_protocol::plan_pagination` outcome lowers
//! into a generated `lib/src/pagination.dart` library part holding the typed
//! traversal error and per-operation pointer readers and continuation rules,
//! plus per-operation page/item/next-page methods on the generated `Client`.
//! Static runtime files gain nothing: with no configured policy, or with no
//! emittable paginated operation, the package stays byte-identical.
//!
//! Streams reuse the direct operation method for transport, encoding, decoding
//! and attribution, re-invoke the direct call with only the pagination control
//! changed, and resolve the compiled RFC 6901 pointers with generated accessor
//! chains — never schema search or JSONPath evaluation.

use super::models::Shape;
use super::{Plan, PlannedOperation, PlannedPayload as P, PlannedStatus};
use crate::http_protocol as p;
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};
use std::collections::BTreeSet;

/// How the walk computes and applies its continuation, mirroring the shared
/// TypeScript walker's branch order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// The offset control advances by the previous page's item count.
    ItemsReturned,
    /// The resolved next-offset pointer feeds the offset, page or cursor control.
    OffsetPointer,
    /// The resolved next-cursor pointer feeds the cursor, offset or page control.
    CursorPointer,
    /// The one-based page control advances by one.
    PageNext,
}

/// The native carrier of a request-side pagination control value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The exact `JsonInteger` model.
    Integer,
    /// A plain `String`.
    Text,
}

/// One request control the walker rewrites between pages.
#[derive(Debug, Clone)]
struct Control {
    /// The direct method's named parameter carrying the wire parameter.
    name: String,
    kind: Kind,
    required: bool,
}

impl Control {
    /// The declared type of one control value.
    fn value_type(&self) -> &'static str {
        match self.kind {
            Kind::Integer => "JsonInteger",
            Kind::Text => "String",
        }
    }

    /// The generated expression reading this control's current value from the
    /// walk's `control` local.
    fn read(&self) -> String {
        if self.required {
            "control".into()
        } else {
            format!(
                "control is Present<{}> ? control.value : null",
                self.value_type()
            )
        }
    }

    /// The generated expression reading the caller's named control argument.
    fn read_argument(&self) -> String {
        if self.required {
            self.name.clone()
        } else {
            format!(
                "{} is Present<{}> ? {}.value : null",
                self.name,
                self.value_type(),
                self.name
            )
        }
    }

    /// The generated expression storing one continuation value back into the
    /// named control argument for later pages.
    fn write(&self) -> String {
        if self.required {
            "next".into()
        } else {
            "Present(next)".into()
        }
    }
}

/// One operation's rendered pagination emission; the metadata fields document
/// the compiled walk for tests and introspection.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(super) struct Walker {
    /// Source operation identity.
    pub operation: String,
    /// Source location for documentation.
    pub location: String,
    /// The direct client method this walk extends.
    pub method: String,
    /// The control the walk rewrites, with its documented member name.
    pub control_name: String,
    pub mode: Mode,
    pub initial_offset: Option<u32>,
    /// Allocated generated client method names.
    pub pages: String,
    pub items: Option<String>,
    pub next_page: String,
    /// Allocated next-input record type for manual driving.
    pub next_input: String,
    /// Allocated continuation helper name in the pagination part.
    pub continuation: String,
    /// Yielded item type for the items stream.
    pub item_type: Option<String>,
    /// Rendered pagination-part pieces.
    pub typedef: String,
    pub helpers: String,
    /// Rendered Client methods.
    pub pages_method: String,
    pub items_method: Option<String>,
    pub next_page_method: String,
}

/// The pointer precedence and control-role order of the shared walker.
fn pointer_selection(
    pattern: PaginationPattern,
    advance: PaginationAdvance,
    page: &p::OperationPagination,
) -> Option<(Mode, PaginationRequestRole)> {
    use PaginationRequestRole as Role;
    use PaginationResponseRole as Pointer;
    let next_offset = page.response.contains_key(&Pointer::NextOffset);
    let next_cursor = page.response.contains_key(&Pointer::NextCursor);
    if advance == PaginationAdvance::NextOffset && (next_offset || next_cursor) {
        // The shared walker prefers a mapped next-offset pointer and otherwise
        // a mapped next-cursor pointer, then applies the value to the pattern's
        // own control first, exactly as the TypeScript walker does.
        if next_offset {
            let role = [Role::Offset, Role::Page, Role::Cursor]
                .into_iter()
                .find(|role| page.request.contains_key(role))?;
            return Some((Mode::OffsetPointer, role));
        }
        let role = [Role::Cursor, Role::Offset, Role::Page]
            .into_iter()
            .find(|role| page.request.contains_key(role))?;
        return Some((Mode::CursorPointer, role));
    }
    if pattern == PaginationPattern::PageNumber && page.request.contains_key(&Role::Page) {
        return Some((Mode::PageNext, Role::Page));
    }
    None
}

/// The request control one mode advances: the wire-named query parameter's
/// native named argument. Values must be string- or exact-integer-typed and
/// never nullable.
fn control(op: &PlannedOperation, name: &str) -> Option<(Control, String)> {
    let parameter = op.parameters.iter().find(|parameter| {
        parameter.wire_name == name && parameter.wire.location() == p::ParameterLocation::Query
    })?;
    let kind = match parameter.native_type.as_str() {
        "JsonInteger" => Kind::Integer,
        "String" => Kind::Text,
        _ => return None,
    };
    Some((
        Control {
            name: parameter.name.clone(),
            kind,
            required: parameter.required,
        },
        parameter.name.clone(),
    ))
}

/// The single typed JSON success representation the walker reads.
fn success_status(op: &PlannedOperation) -> Option<&PlannedStatus> {
    if op.stream {
        return None;
    }
    let successes = op
        .statuses
        .iter()
        .filter(|status| status.success_name.is_some())
        .collect::<Vec<_>>();
    let [status] = successes.as_slice() else {
        return None;
    };
    let status = *status;
    if status.none_variant.is_some() || status.bytes_variant.is_some() || status.media.len() != 1 {
        return None;
    }
    match &status.media[0].payload {
        P::Json {
            schema: Some(_),
            codec: Some(_),
        } => Some(status),
        _ => None,
    }
}

/// The collection kind one resolved pointer must read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeafKind {
    /// The array at the items pointer, whose element type feeds the items walk.
    Items,
    /// A string continuation token.
    Text,
    /// An exact-integer continuation value or page total.
    Integer,
    /// A boolean has-more flag.
    Flag,
}

/// One resolved pointer: guard statements, the final expression, and the
/// element type when the pointer reads a collection.
struct Resolved {
    statements: String,
    expression: String,
    element: Option<String>,
}

/// The element type when one pointer's final shape matches its expected kind.
fn leaf_element(plan: &Plan, node: usize, kind: LeafKind) -> Option<Option<String>> {
    let shape = &plan.models().symbols()[node].shape;
    match (kind, shape) {
        (LeafKind::Items, Shape::Array(target)) => Some(Some(plan.models().optional_ty(*target))),
        (LeafKind::Text, Shape::String) => Some(None),
        (LeafKind::Integer, Shape::Integer) => Some(None),
        (LeafKind::Flag, Shape::Boolean) => Some(None),
        _ => None,
    }
}

/// Resolve one RFC 6901 pointer over the decoded page body with generated
/// accessor chains and absence guards. `None` means this plan cannot express
/// the pointer with plain property access, so the operation stays without
/// streams.
fn resolve_pointer(plan: &Plan, root: usize, pointer: &str, kind: LeafKind) -> Option<Resolved> {
    let mut statements = String::new();
    let mut counter = 0usize;
    let mut expression = String::from("page.data");
    if plan.models().uses_native_null(root) {
        counter += 1;
        statements.push_str(&format!(
            "  final value{counter} = page.data;\n  if (value{counter} == null) {{ return null; }}\n"
        ));
        expression = format!("value{counter}");
    }
    if pointer.is_empty() {
        return leaf_element(plan, plan.models().concrete(root), kind).map(|element| Resolved {
            statements,
            expression,
            element,
        });
    }
    let mut node = plan.models().concrete(root);
    for raw in pointer[1..].split('/') {
        let segment = raw.replace("~1", "/").replace("~0", "~");
        let fields = plan.models().symbols()[node].members();
        let field = fields.iter().find(|field| field.wire_name == segment)?;
        if field.fixed.is_some() {
            return None;
        }
        let target = field.target?;
        let value_type = plan.models().ty(target);
        let access = format!("{expression}.{}", field.name);
        counter += 1;
        let local = format!("value{counter}");
        if !field.required {
            statements.push_str(&format!(
                "  final {local} = {access};\n  if ({local} is! Present<{value_type}>) {{ return null; }}\n"
            ));
            counter += 1;
            let unwrapped = format!("value{counter}");
            statements.push_str(&format!("  final {unwrapped} = {local}.value;\n"));
            expression = unwrapped;
        } else {
            statements.push_str(&format!("  final {local} = {access};\n"));
            expression = local;
        }
        if plan.models().uses_native_null(target) {
            statements.push_str(&format!("  if ({expression} == null) {{ return null; }}\n"));
        }
        node = plan.models().concrete(target);
    }
    let element = leaf_element(plan, node, kind)?;
    Some(Resolved {
        statements,
        expression,
        element,
    })
}

/// Render one pointer reader as an unexported function with a nullable return,
/// so absence reads as null at every stop rule.
fn render_accessor(
    result_type: &str,
    method: &str,
    returns: &str,
    comment: &str,
    resolved: &Resolved,
) -> String {
    format!(
        "/// {comment}\n{returns} {method}({result_type} page) {{\n{}  return {};\n}}\n",
        resolved.statements, resolved.expression
    )
}

/// The documented stop rule of one compiled walk.
fn stop_rule(mode: Mode, has_more: bool) -> &'static str {
    match mode {
        Mode::ItemsReturned => {
            if has_more {
                "after a page with no items or when the has-more indicator is false"
            } else {
                "after a page with no items"
            }
        }
        Mode::CursorPointer => "when the next cursor token is absent, null or an empty string",
        Mode::OffsetPointer => "when the next offset pointer is absent or null",
        Mode::PageNext => {
            if has_more {
                "after a page with no items, when the has-more indicator is false, or after the last declared page"
            } else {
                "after a page with no items"
            }
        }
    }
}

/// Escape one span of Dart doc comment text.
fn prose(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
}

/// The compiled pagination emission: the generated `lib/src/pagination.dart`
/// part and the per-operation Client methods, or nothing when no configured
/// operation lowers into a walk.
pub(super) struct Emission {
    /// The generated part content, without the `part of` header.
    pub(super) part: String,
    /// The per-operation methods appended inside the generated `Client`.
    pub(super) client: String,
}

/// Compile the emittable subset of the configured outcome. A configured policy
/// with no emittable operation emits nothing.
pub(super) fn emission(plan: &Plan) -> Option<Emission> {
    let outcome = plan.pagination()?;
    if outcome.paginated.is_empty() {
        return None;
    }
    let mut methods: BTreeSet<String> = plan
        .operations()
        .iter()
        .map(|op| op.method_name.clone())
        .collect();
    methods.extend(
        [
            "close",
            "runtimeType",
            "hashCode",
            "toString",
            "noSuchMethod",
        ]
        .map(str::to_owned),
    );
    let mut names = plan.models().used_names();
    let mut walkers = Vec::new();
    let mut repeats = false;
    for op in plan.operations() {
        let Some(page) = outcome
            .paginated
            .iter()
            .find(|page| page.operation == op.operation_id)
        else {
            continue;
        };
        if let Some(walker) = compile(plan, op, page, &mut methods, &mut names, &mut repeats) {
            walkers.push(walker);
        }
    }
    if walkers.is_empty() {
        return None;
    }
    let mut part = String::from(
        "/// A pagination walk refused to continue because the source repeated an\n/// identical continuation value, which would loop forever, or because it\n/// could not compute an exact continuation.\nfinal class PaginationException extends SdkException {\n  const PaginationException(this.message);\n\n  final String message;\n\n  @override\n  String toString() => 'PaginationException: $message';\n}\n",
    );
    if repeats {
        part.push_str("/// Whether two exact-integer continuations repeat the same mathematical\n/// value, under any spelling.\nbool _paginationRepeats(JsonInteger previous, JsonInteger next) {\n  if (previous.token == next.token) { return true; }\n  try {\n    return previous.toBigInt() == next.toBigInt();\n  } on JsonException {\n    return false;\n  }\n}\n");
    }
    let mut client = String::new();
    for walker in &walkers {
        part.push_str(&walker.typedef);
        part.push_str(&walker.helpers);
        client.push_str(&walker.pages_method);
        if let Some(items) = &walker.items_method {
            client.push_str(items);
        }
        client.push_str(&walker.next_page_method);
    }
    Some(Emission { part, client })
}

/// The direct method's named-parameter list and its argument lists.
fn call_shape(
    op: &PlannedOperation,
    control: &Control,
    control_name: &str,
    limit_name: Option<&str>,
) -> (String, String, String) {
    let mut parameters = Vec::new();
    let mut direct = Vec::new();
    let mut walked = Vec::new();
    for parameter in &op.parameters {
        parameters.push(if parameter.required {
            format!("required {} {}", parameter.native_type, parameter.name)
        } else {
            format!(
                "Presence<{}> {} = const Absent()",
                parameter.native_type, parameter.name
            )
        });
        direct.push(format!("{}: {}", parameter.name, parameter.name));
        if parameter.name == control_name {
            walked.push(format!("{}: control", control.name));
        } else if Some(parameter.name.as_str()) == limit_name {
            // The first page's limit comes from the filled local; later pages
            // keep whatever the walk last used (the continuation never writes
            // the limit).
            walked.push(format!("{}: limit", parameter.name));
        } else {
            walked.push(format!("{}: {}", parameter.name, parameter.name));
        }
    }
    if let Some(body) = &op.body {
        parameters.push(if body.required {
            format!("required {} body", body.native_type)
        } else {
            format!("Presence<{}> body = const Absent()", body.native_type)
        });
        direct.push("body: body".into());
        walked.push("body: body".into());
    }
    parameters.push(
        "CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative"
            .into(),
    );
    for argument in [
        "cancellation: cancellation",
        "timeout: timeout",
        "server: server",
        "securityAlternative: securityAlternative",
    ] {
        direct.push(argument.into());
        walked.push(argument.into());
    }
    (parameters.join(", "), direct.join(", "), walked.join(", "))
}

/// The generated lazy walk for one operation.
fn walk_body(
    continuation: &str,
    control: &Control,
    method: &str,
    arguments: &str,
    initial: Option<u32>,
    limit: Option<(&Control, u32)>,
) -> String {
    let mut body = String::new();
    if control.required {
        body.push_str(&format!(
            "    {} control = {};\n",
            control.value_type(),
            control.name
        ));
    } else {
        body.push_str(&format!("    var control = {};\n", control.name));
    }
    if let Some(initial) = initial.filter(|_| !control.required && control.kind == Kind::Integer) {
        body.push_str(&format!(
            "    if (!control.isPresent) {{\n      control = Present(JsonInteger.fromInt({initial}));\n    }}\n"
        ));
    }
    // The documented SDK fallback page size fills the limit argument on the
    // first request only, when the caller left it absent; later pages keep
    // whatever limit the walk last used.
    if let Some((limit_control, size)) =
        limit.filter(|(limit_control, _)| !limit_control.required && limit_control.kind == Kind::Integer)
    {
        body.push_str(&format!(
            "    var limit = {};\n    if (!limit.isPresent) {{\n      limit = Present(JsonInteger.fromInt({size}));\n    }}\n",
            limit_control.name
        ));
    }
    body.push_str(&format!(
        "    while (true) {{\n      final page = await {method}({arguments});\n      yield page;\n      final next = {continuation}({read}, page);\n      if (next == null) {{ return; }}\n      control = {write};\n    }}\n",
        method = method,
        arguments = arguments,
        continuation = continuation,
        read = control.read(),
        write = control.write()
    ));
    body
}

/// The rebuilt-input record fields of one operation, with the control
/// replaced by the computed continuation.
fn record_arguments(op: &PlannedOperation, control: &Control, control_name: &str) -> String {
    let mut fields = Vec::new();
    for parameter in &op.parameters {
        if parameter.name == control_name {
            fields.push(format!("{}: {}", control.name, control.write()));
        } else {
            fields.push(format!("{}: {}", parameter.name, parameter.name));
        }
    }
    if op.body.is_some() {
        fields.push("body: body".into());
    }
    fields.join(", ")
}

/// The next-input record type of one operation, re-typed for manual driving.
fn next_input_typedef(
    op: &PlannedOperation,
    name: &str,
    control: &Control,
    control_name: &str,
) -> String {
    let mut fields = Vec::new();
    for parameter in &op.parameters {
        let field_type = if parameter.name == control_name {
            control.value_type().to_string()
        } else {
            parameter.native_type.clone()
        };
        fields.push(if parameter.required {
            format!("{field_type} {}", parameter.name)
        } else {
            format!("Presence<{}> {}", parameter.native_type, parameter.name)
        });
    }
    if let Some(body) = &op.body {
        fields.push(if body.required {
            format!("required {} body", body.native_type)
        } else {
            format!("Presence<{}> body", body.native_type)
        });
    }
    format!(
        "/// The rebuilt pagination input for the page after the fetched one.\ntypedef {name} = ({{ {fields} }});\n",
        name = name,
        fields = fields.join(", ")
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn compile(
    plan: &Plan,
    op: &PlannedOperation,
    page: &p::OperationPagination,
    methods: &mut BTreeSet<String>,
    names: &mut BTreeSet<String>,
    repeats: &mut bool,
) -> Option<Walker> {
    let status = success_status(op)?;
    let result_type = status.success_name.clone()?;
    let root = match &status.media[0].payload {
        P::Json {
            schema: Some(schema),
            ..
        } => plan.models().model(schema)?.index,
        _ => return None,
    };
    let stem = super::models::exported(&op.method_name);
    // Resolve the limit control before the `control` binding below shadows
    // the resolver function.
    let limit_control = page
        .request
        .get(&PaginationRequestRole::Limit)
        .and_then(|name| control(op, name).map(|(control, _)| control));
    let (mode, control, control_name) = match (page.pattern, page.advance) {
        (PaginationPattern::LimitOffset, PaginationAdvance::ItemsReturned) => {
            let name = page.request.get(&PaginationRequestRole::Offset)?;
            let (control, member) = control(op, name)?;
            (Mode::ItemsReturned, control, member)
        }
        _ => {
            let (mode, role) = pointer_selection(page.pattern, page.advance, page)?;
            let name = page.request.get(&role)?;
            let (control, member) = control(op, name)?;
            (mode, control, member)
        }
    };
    let mut items_accessor = None;
    let mut item_type = None;
    let mut helpers = String::new();
    if let Some(pointer) = page.response.get(&PaginationResponseRole::Items) {
        let resolved = resolve_pointer(plan, root, pointer, LeafKind::Items)?;
        let element = resolved.element.clone()?;
        let method = format!("_pagination{stem}Items");
        item_type = Some(element.clone());
        helpers.push_str(&render_accessor(
            &result_type,
            &method,
            &format!("List<{element}>?"),
            &format!(
                "Items collection of one decoded page at '{}'; absent paths read as no items.",
                prose(pointer)
            ),
            &resolved,
        ));
        items_accessor = Some(method);
    }
    let mut cursor_accessor = None;
    let mut offset_accessor = None;
    if mode == Mode::OffsetPointer || mode == Mode::CursorPointer {
        let pointer = if mode == Mode::OffsetPointer {
            page.response
                .get(&PaginationResponseRole::NextOffset)
                .or_else(|| page.response.get(&PaginationResponseRole::NextCursor))?
        } else {
            page.response
                .get(&PaginationResponseRole::NextCursor)
                .or_else(|| page.response.get(&PaginationResponseRole::NextOffset))?
        };
        let (kind, returns) = match control.kind {
            Kind::Text => (LeafKind::Text, "String?"),
            Kind::Integer => (LeafKind::Integer, "JsonInteger?"),
        };
        let resolved = resolve_pointer(plan, root, pointer, kind)?;
        let method = match mode {
            Mode::OffsetPointer => format!("_pagination{stem}NextOffset"),
            _ => format!("_pagination{stem}Cursor"),
        };
        let rendered = render_accessor(
            &result_type,
            &method,
            returns,
            &format!(
                "Continuation value of one decoded page at '{}'; absent paths read as null.",
                prose(pointer)
            ),
            &resolved,
        );
        helpers.push_str(&rendered);
        if mode == Mode::OffsetPointer {
            *repeats = true;
            offset_accessor = Some(rendered);
        } else {
            cursor_accessor = Some(rendered);
        }
    }
    let mut has_more_accessor = None;
    if let Some(pointer) = page.response.get(&PaginationResponseRole::HasMore) {
        let resolved = resolve_pointer(plan, root, pointer, LeafKind::Flag)?;
        let method = format!("_pagination{stem}HasMore");
        let rendered = render_accessor(
            &result_type,
            &method,
            "bool?",
            &format!(
                "Declared has-more evidence of one decoded page at '{}'.",
                prose(pointer)
            ),
            &resolved,
        );
        helpers.push_str(&rendered);
        has_more_accessor = Some(rendered);
    }
    let mut total_pages_accessor = None;
    if mode == Mode::PageNext
        && let Some(pointer) = page.response.get(&PaginationResponseRole::TotalPages)
        && let Some(resolved) = resolve_pointer(plan, root, pointer, LeafKind::Integer)
    {
        let method = format!("_pagination{stem}TotalPages");
        let rendered = render_accessor(
            &result_type,
            &method,
            "JsonInteger?",
            &format!(
                "Declared total page count of one decoded page at '{}'.",
                prose(pointer)
            ),
            &resolved,
        );
        helpers.push_str(&rendered);
        total_pages_accessor = Some(rendered);
    }
    // Counting modes cannot advance without the items collection; pointer
    // modes advance without it but then emit no items stream.
    if items_accessor.is_none() && matches!(mode, Mode::ItemsReturned | Mode::PageNext) {
        return None;
    }
    let pages = super::models::allocate(&format!("{}Pages", op.method_name), methods);
    let next_page = super::models::allocate(&format!("{}NextPage", op.method_name), methods);
    let items = items_accessor
        .is_some()
        .then(|| super::models::allocate(&format!("{}Items", op.method_name), methods));
    let next_input = super::models::allocate(&format!("{stem}NextInput"), names);
    let continuation = format!("_pagination{stem}Next");
    // The documented SDK fallback page size rewrites the limit argument on
    // the first request only, when the caller left it absent.
    let limit_fill = limit_control.zip(page.initial_limit);
    let (parameters, direct_arguments, arguments) = call_shape(
        op,
        &control,
        &control_name,
        if limit_fill.is_some() {
            page.request
                .get(&PaginationRequestRole::Limit)
                .map(|name| name.as_str())
        } else {
            None
        },
    );
    let has_more = has_more_accessor.is_some();
    let location = format!("{}#{}", op.source.document(), op.source.pointer());
    let description = op
        .wire
        .description()
        .map(|d| d.value().to_string())
        .unwrap_or_else(|| op.operation_id.clone());
    let typedef = next_input_typedef(op, &next_input, &control, &control_name);
    let helpers = format!(
        "{helpers}{}",
        continuation_helpers(
            &continuation,
            mode,
            control.kind,
            &page.operation,
            &result_type,
            items_accessor
                .as_deref()
                .map(|_| format!("_pagination{stem}Items")),
            cursor_accessor
                .as_deref()
                .map(|_| format!("_pagination{stem}Cursor")),
            offset_accessor
                .as_deref()
                .map(|_| format!("_pagination{stem}NextOffset")),
            has_more_accessor
                .as_deref()
                .map(|_| format!("_pagination{stem}HasMore")),
            total_pages_accessor
                .as_deref()
                .map(|_| format!("_pagination{stem}TotalPages")),
        )
    );
    let initial = (mode == Mode::ItemsReturned)
        .then_some(page.initial_offset)
        .flatten();
    let walk = walk_body(
        &continuation,
        &control,
        &op.method_name,
        &arguments,
        initial,
        limit_fill.as_ref().map(|(limit_control, size)| (limit_control, *size)),
    );
    let fallback_doc = page.initial_limit.map_or_else(String::new, |size| {
        format!(
            " When the caller omits the limit, the first request supplies the documented SDK page size ({size}); later pages keep the limit the walk last used."
        )
    });
    let pages_method = format!(
        "  /// {description} Lazily yields every success page of {operation}. The first page is the direct {method}(...) result, fetched on the first iteration step and included exactly once. Between requests only the {control} pagination control changes: every other argument is preserved exactly, and a caller-supplied {control} is used for the first request and replaced by the computed continuation afterwards.{fallback_doc} Stops {stop}. A repeated identical continuation value throws PaginationException instead of looping. Iteration is lazy: no request happens before the stream is listened to, breaking out or cancelling prevents further requests, and pages are never prefetched.\n  /// Source: {location}.\n  Stream<{result}> {pages}({{ {parameters} }}) async* {{\n{walk}  }}\n",
        description = prose(&description),
        operation = page.operation,
        method = op.method_name,
        control = control_name,
        stop = stop_rule(mode, has_more),
        location = prose(&location),
        result = result_type,
        pages = pages,
        parameters = parameters,
        walk = walk,
        fallback_doc = fallback_doc,
    );
    let items_method = items.as_ref().map(|items| {
        format!(
            "  /// Flattens the page items of {method} across every page, in order. Accepts the same arguments as the direct call; see {pages} for lazy fetching, continuation, stopping and caller-override semantics.\n  /// Source: {location}.\n  Stream<{item_type}> {items}({{ {parameters} }}) async* {{\n    await for (final page in {pages}({direct})) {{\n      final pageItems = {accessor}(page);\n      if (pageItems == null) {{ continue; }}\n      for (final item in pageItems) {{\n        yield item;\n      }}\n    }}\n  }}\n",
            method = op.method_name,
            pages = pages,
            location = prose(&location),
            item_type = item_type.as_deref().unwrap_or("JsonValue"),
            items = items,
            parameters = parameters,
            direct = direct_arguments,
            accessor = items_accessor.as_deref().unwrap_or_default()
        )
    });
    let next_page_method = format!(
        "  /// Fetches the page described by the arguments and returns the rebuilt input for the following page, or null when the walk stops, so callers can drive pages manually. See {pages} for the stop rules and caller-override semantics.\n  /// Source: {location}.\n  Future<{next_input}?> {next_page}({{ {parameters} }}) async {{\n    final page = await {method}({direct});\n    final next = {continuation}({read}, page);\n    if (next == null) {{ return null; }}\n    return ({record});\n  }}\n",
        pages = pages,
        location = prose(&location),
        next_input = next_input,
        next_page = next_page,
        parameters = parameters,
        method = op.method_name,
        direct = direct_arguments,
        continuation = continuation,
        read = control.read_argument(),
        record = record_arguments(op, &control, &control_name)
    );
    Some(Walker {
        operation: page.operation.clone(),
        location,
        method: op.method_name.clone(),
        control_name,
        mode,
        initial_offset: page.initial_offset,
        pages,
        items,
        next_page,
        next_input,
        continuation,
        item_type,
        typedef,
        helpers,
        pages_method,
        items_method,
        next_page_method,
    })
}

#[allow(clippy::too_many_arguments)]
fn continuation_helpers(
    continuation: &str,
    mode: Mode,
    kind: Kind,
    operation: &str,
    result_type: &str,
    items: Option<String>,
    cursor: Option<String>,
    offset: Option<String>,
    has_more: Option<String>,
    total_pages: Option<String>,
) -> String {
    let has_more_stop = has_more
        .as_ref()
        .map(|accessor| format!("  if ({accessor}(page) == false) {{ return null; }}\n"))
        .unwrap_or_default();
    let stall = |message: &str| format!("PaginationException('{operation}: {message}')");
    let (returns, read_type) = match kind {
        Kind::Text => ("String?", "String?"),
        Kind::Integer => ("JsonInteger?", "JsonInteger?"),
    };
    let body = match mode {
        Mode::ItemsReturned => format!(
            "{has_more_stop}  final items = {items}(page);\n  final count = items == null ? 0 : items.length;\n  if (count == 0) {{ return null; }}\n  var base = BigInt.zero;\n  if (current != null) {{\n    try {{\n      base = current.toBigInt();\n    }} on JsonException {{\n      throw {stall};\n    }}\n  }}\n  try {{\n    return JsonInteger.fromBigInt(base + BigInt.from(count));\n  }} on JsonException {{\n    throw {advanced};\n  }}\n",
            has_more_stop = has_more_stop,
            items = items.unwrap_or_default(),
            stall = stall("the current pagination offset is not an exact integer"),
            advanced = stall("the advanced pagination offset is not representable")
        ),
        Mode::CursorPointer => format!(
            "  final token = {accessor}(page);\n  if (token == null || token.isEmpty) {{ return null; }}\n  if (current != null && current == token) {{\n    throw {stall};\n  }}\n  return token;\n",
            accessor = cursor.unwrap_or_default(),
            stall = stall(
                "the source API returned an identical continuation value; the paginated walk would never terminate"
            )
        ),
        Mode::OffsetPointer => format!(
            "  final value = {accessor}(page);\n  if (value == null) {{ return null; }}\n  if (current != null && _paginationRepeats(current, value)) {{\n    throw {stall};\n  }}\n  return value;\n",
            accessor = offset.unwrap_or_default(),
            stall = stall(
                "the source API returned an identical continuation value; the paginated walk would never terminate"
            )
        ),
        Mode::PageNext => {
            let total = total_pages
                .as_ref()
                .map(|accessor| {
                    format!(
                        "  final total = {accessor}(page);\n  if (total != null) {{\n    try {{\n      if (number >= total.toBigInt()) {{ return null; }}\n    }} on JsonException {{\n      throw {total_failure};\n    }}\n  }}\n",
                        accessor = accessor,
                        total_failure = stall("the declared total page count is not an exact integer")
                    )
                })
                .unwrap_or_default();
            format!(
                "{has_more_stop}  final items = {items}(page);\n  if (items == null || items.isEmpty) {{ return null; }}\n  var number = BigInt.one;\n  if (current != null) {{\n    try {{\n      number = current.toBigInt();\n    }} on JsonException {{\n      throw {page_failure};\n    }}\n  }}\n{total}  try {{\n    return JsonInteger.fromBigInt(number + BigInt.one);\n  }} on JsonException {{\n    throw {advanced};\n  }}\n",
                has_more_stop = has_more_stop,
                items = items.unwrap_or_default(),
                page_failure = stall("the current page number is not an exact integer"),
                total = total,
                advanced = stall("the advanced page number is not representable")
            )
        }
    };
    format!(
        "/// The continuation value for the page after one fetched page, or null at\n/// the stop rule; a repeated identical continuation value refuses the walk.\n{returns} {continuation}({read_type} current, {result_type} page) {{\n{body}}}\n",
        returns = returns,
        continuation = continuation,
        read_type = read_type,
        result_type = result_type,
        body = body
    )
}
