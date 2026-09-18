//! Generated pagination walks for the Java HTTP SDK.
//!
//! The shared, generation-time `http_protocol::plan_pagination` outcome
//! compiles into one generated `Pagination.java` holding a closeable page
//! iterator, a flattening item iterator and a next-input builder per paginated
//! operation. Static runtime files and the shared planner stay untouched, and
//! plans without configured client defaults (or without an emittable paginated
//! operation) emit no new file at all.
//!
//! Traversal is lazy and pull-based: a request starts only inside `hasNext()`
//! when a not-yet-fetched page is required, the first page is exactly the
//! direct call's result included once, later requests change only the
//! pagination control, and the caller's pagination values win for the first
//! request. Stop rules, the repeated-continuation typed
//! `SdkException("pagination-stalled")` guard and RFC 6901 pointer resolution
//! through generated typed chains mirror the other backends; there is no
//! runtime schema search and no JSON re-parsing.

use std::collections::BTreeSet;

use super::{
    SdkPlan,
    http::JavaOperation,
    models::{JavaDeclaration, JavaModelPlan, JavaType, allocate, javadoc, q},
    protocol::JavaValue,
};
use crate::http_protocol::{OperationPagination, ResponseStatus};
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};
use suspect_ir::contract::SchemaId;

/// How one walk computes and applies its continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Advance {
    /// The offset control advances by the previous page's item count. The
    /// documented stop rules are a zero-item page or a false has-more flag.
    OffsetItemCount,
    /// The one-based page control advances by one per page.
    PageNext,
    /// The next-cursor pointer feeds the cursor control. The walk stops when
    /// the pointer is absent, null or empty; a repeated identical cursor is a
    /// typed pagination error.
    CursorPointer,
    /// The next-offset pointer feeds the offset control. The walk stops when
    /// the pointer is absent or null; a repeated identical offset is a typed
    /// pagination error.
    OffsetPointer,
}

/// The request control the walk rewrites between pages.
struct Control {
    /// Input accessor and builder member name.
    member: String,
    /// `JsonNumber` when the control carries an exact integer, `String` otherwise.
    number: bool,
    required: bool,
}

impl Control {
    /// The native control type used by rebuild signatures.
    fn native(&self) -> &'static str {
        if self.number {
            "JsonNumber"
        } else {
            "String"
        }
    }
}

/// The leaf flavor a role reads from a resolved chain.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Leaf {
    /// `java.util.List<Element>` item collection.
    Items(String),
    /// String continuation token.
    Token,
    /// `JsonNumber` continuation.
    Number,
    /// `Boolean` flag.
    Flag,
}

/// One resolved response-pointer reader: Java statements over a `value` local
/// that leave the final value in `binding`.
#[derive(Clone)]
struct Walk {
    steps: Vec<String>,
    binding: String,
    /// The declared, alias-resolved type `binding` holds.
    ty: JavaType,
    leaf: Leaf,
}

/// One compiled pagination walk, ready to render into `Pagination.java`.
struct PaginatedOperation<'a> {
    op: &'a JavaOperation,
    page: &'a OperationPagination,
    /// The generated client class name.
    api: String,
    /// The single success variant the walk yields.
    success: String,
    /// Whether the success body is a body-forbidden-aware wrapper.
    wrapped_body: bool,
    /// Allocated nested iterator type names.
    pages: String,
    items: Option<String>,
    /// Allocated factory method names.
    pages_method: String,
    items_method: Option<String>,
    next_page_method: String,
    advance: Advance,
    control: Control,
    /// The optional limit control and the documented SDK fallback page size it
    /// carries on the first request, when the operation has one.
    limit: Option<(Control, u32)>,
    initial: u32,
    /// True when page 1 rebuilds an absent control with the configured initial.
    inject_initial: bool,
    items_walk: Option<Walk>,
    continuation_walk: Option<(Walk, PaginationResponseRole)>,
    has_more_walk: Option<Walk>,
}

/// Follow alias declarations to the underlying value type.
fn deref(plan: &JavaModelPlan, ty: &JavaType) -> JavaType {
    match ty {
        JavaType::Named(id) => match plan.symbol(id).map(|symbol| symbol.declaration()) {
            Some(JavaDeclaration::Alias(inner)) => deref(plan, inner),
            _ => ty.clone(),
        },
        other => other.clone(),
    }
}

/// The leaf flavor a declared chain type can serve for one role.
fn leaf_for(plan: &JavaModelPlan, ty: &JavaType, role: PaginationResponseRole) -> Option<Leaf> {
    Some(match (ty, role) {
        (JavaType::List(inner), PaginationResponseRole::Items) => {
            Leaf::Items(plan.render_type(inner))
        }
        (JavaType::String, PaginationResponseRole::NextCursor) => Leaf::Token,
        (JavaType::Number, PaginationResponseRole::NextCursor) => Leaf::Token,
        (JavaType::Number, PaginationResponseRole::NextOffset) => Leaf::Number,
        (JavaType::Boolean, PaginationResponseRole::HasMore) => Leaf::Flag,
        _ => return None,
    })
}

/// Resolve one RFC 6901 pointer into generated typed chains over the decoded
/// page model. Every intermediate segment must name a declared field of a
/// native object model; anything else cannot be walked and the operation stays
/// without walkers.
fn resolve(plan: &SdkPlan, root: &SchemaId, pointer: &str) -> Option<Walk> {
    let mut counter = 0usize;
    let mut steps = Vec::new();
    let mut current = JavaType::Named(root.clone());
    let segments: Vec<String> = pointer[1..]
        .split('/')
        .map(|raw| raw.replace("~1", "/").replace("~0", "~"))
        .collect();
    for (index, segment) in segments.iter().enumerate() {
        let is_last = index + 1 == segments.len();
        let JavaType::Named(key) = deref(plan.models(), &current) else {
            return None;
        };
        let Some(JavaDeclaration::Object { fields, .. }) =
            plan.models().symbol(&key).map(|symbol| symbol.declaration())
        else {
            return None;
        };
        let field = fields.iter().find(|field| field.wire == *segment)?;
        counter += 1;
        let binding = format!("value_{counter}");
        let rendered = plan.models().render_type(&field.ty);
        let access = format!("value.{}()", field.name);
        steps.push(if field.required {
            format!("            {rendered} {binding} = {access};")
        } else {
            format!("            {rendered} {binding} = {access}.isPresent() ? {access}.value() : null;")
        });
        if !field.required || field.nullable {
            steps.push(format!("            if ({binding} == null) return null;"));
        }
        // The final binding is the reader's result; re-assignments only feed
        // deeper segments.
        if !is_last {
            steps.push(format!("            value = {binding};"));
        }
        current = JavaType::Named(field.source.clone());
    }
    let binding = if steps.is_empty() {
        "value".to_owned()
    } else {
        format!("value_{counter}")
    };
    Some(Walk {
        steps,
        binding,
        ty: deref(plan.models(), &current),
        leaf: Leaf::Token,
    })
}

/// Attach the leaf flavor a role reads to a resolved chain.
fn with_leaf(plan: &JavaModelPlan, mut walk: Walk, role: PaginationResponseRole) -> Option<Walk> {
    walk.leaf = leaf_for(plan, &walk.ty, role)?;
    Some(walk)
}

/// The request control of one role, when the native parameter can serve.
fn control(
    plan: &SdkPlan,
    op: &JavaOperation,
    page: &OperationPagination,
    role: PaginationRequestRole,
) -> Option<Control> {
    let wire = page.request.get(&role)?;
    let parameter = op
        .parameters
        .iter()
        .find(|parameter| {
            parameter.wire.location() == crate::http_protocol::ParameterLocation::Query
                && parameter.wire.name() == wire.as_str()
        })?;
    if plan
        .models()
        .symbol(&parameter.schema)
        .is_some_and(|symbol| symbol.nullable())
    {
        return None;
    }
    let resolved = deref(plan.models(), &JavaType::Named(parameter.schema.clone()));
    let number = matches!(resolved, JavaType::Number);
    // A string control must be a plain string; literal/enum classes cannot
    // carry a resolved token without an invented conversion.
    if !number && !matches!(resolved, JavaType::String) {
        return None;
    }
    Some(Control {
        member: parameter.native_name.clone(),
        number,
        required: parameter.required,
    })
}

/// The decoded-body schema of the operation's single JSON success response —
/// the same response the shared planner compiled pointers against — plus
/// whether that body is a body-forbidden-aware wrapper.
fn success_payload(op: &JavaOperation) -> Option<(SchemaId, bool)> {
    let mut successes = op.responses.iter().filter(|response| response.can_succeed());
    let response = successes.next()?;
    if successes.next().is_some() {
        return None;
    }
    if !matches!(
        response.wire.status(),
        ResponseStatus::Exact(200..=299) | ResponseStatus::Range(2)
    ) {
        return None;
    }
    match &response.value {
        JavaValue::ResponseBody(inner) => match inner.as_ref() {
            JavaValue::Model(id) => Some((id.clone(), true)),
            _ => None,
        },
        JavaValue::Model(id) => Some((id.clone(), false)),
        _ => None,
    }
}

/// Compile every emittable paginated operation of the plan, in plan order.
/// The planner already validated every request name and response pointer, so
/// this only lowers the compiled entry onto native member names and drops
/// operations whose native representations cannot express the walk.
fn operations<'a>(plan: &'a SdkPlan) -> Vec<PaginatedOperation<'a>> {
    let Some(outcome) = plan.pagination() else {
        return Vec::new();
    };
    if outcome.paginated.is_empty() {
        return Vec::new();
    }
    let mut types: BTreeSet<String> = BTreeSet::new();
    let mut methods: BTreeSet<String> = BTreeSet::new();
    let mut compiled = Vec::new();
    for op in plan.operations() {
        let identity = op
            .wire
            .operation_id()
            .map(|located| located.value().clone())
            .unwrap_or_else(|| format!("{} {}", op.wire.method().as_str(), op.wire.path()));
        let Some(page) = outcome
            .paginated
            .iter()
            .find(|page| page.operation == identity)
        else {
            continue;
        };
        if let Some(built) = build(plan, op, page, &mut types, &mut methods) {
            compiled.push(built);
        }
    }
    compiled
}

#[allow(clippy::too_many_lines)]
fn build<'a>(
    plan: &'a SdkPlan,
    op: &'a JavaOperation,
    page: &'a OperationPagination,
    types: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> Option<PaginatedOperation<'a>> {
    use PaginationRequestRole as Role;
    if page.pattern == PaginationPattern::NextLink {
        // A continuation URL cannot replace the request target through the
        // typed input; next-link following needs declared URL policy.
        return None;
    }
    let (root, wrapped_body) = success_payload(op)?;
    let control_role = match page.pattern {
        PaginationPattern::Cursor => Role::Cursor,
        PaginationPattern::PageNumber => match page.advance {
            PaginationAdvance::NextOffset => Role::Offset,
            PaginationAdvance::ItemsReturned => Role::Page,
        },
        PaginationPattern::LimitOffset | PaginationPattern::NextLink => Role::Offset,
    };
    let limit_control = control(plan, op, page, Role::Limit);
    let control = control(plan, op, page, control_role)?;
    let items_walk = page
        .response
        .get(&PaginationResponseRole::Items)
        .and_then(|pointer| resolve(plan, &root, pointer))
        .and_then(|walk| with_leaf(plan.models(), walk, PaginationResponseRole::Items));
    let cursor_walk = page
        .response
        .get(&PaginationResponseRole::NextCursor)
        .and_then(|pointer| resolve(plan, &root, pointer))
        .and_then(|walk| with_leaf(plan.models(), walk, PaginationResponseRole::NextCursor));
    let offset_walk = page
        .response
        .get(&PaginationResponseRole::NextOffset)
        .and_then(|pointer| resolve(plan, &root, pointer))
        .and_then(|walk| with_leaf(plan.models(), walk, PaginationResponseRole::NextOffset));
    let has_more_walk = page
        .response
        .get(&PaginationResponseRole::HasMore)
        .and_then(|pointer| resolve(plan, &root, pointer))
        .and_then(|walk| with_leaf(plan.models(), walk, PaginationResponseRole::HasMore));

    let advance = match page.pattern {
        PaginationPattern::Cursor => {
            cursor_walk.as_ref()?;
            Advance::CursorPointer
        }
        PaginationPattern::LimitOffset | PaginationPattern::PageNumber => {
            let pointer_advance = page.advance == PaginationAdvance::NextOffset
                && offset_walk.is_some()
                && page.request.contains_key(&Role::Offset);
            if pointer_advance {
                offset_walk.as_ref()?;
                Advance::OffsetPointer
            } else if page.pattern == PaginationPattern::PageNumber {
                items_walk.as_ref()?;
                Advance::PageNext
            } else {
                items_walk.as_ref()?;
                Advance::OffsetItemCount
            }
        }
        PaginationPattern::NextLink => unreachable!("rejected above"),
    };
    let continuation_walk = match advance {
        Advance::CursorPointer => cursor_walk
            .clone()
            .map(|walk| (walk, PaginationResponseRole::NextCursor)),
        Advance::OffsetPointer => offset_walk
            .clone()
            .map(|walk| (walk, PaginationResponseRole::NextOffset)),
        Advance::OffsetItemCount | Advance::PageNext => None,
    };

    let stem = crate::rust_models::pascal(&op.method_name);
    let pages = allocate(&format!("{stem}Pages"), types);
    let pages_method = allocate(&method_name(&format!("{stem}Pages")), methods);
    let next_page_method = allocate(&method_name(&format!("{stem}NextPage")), methods);
    let (items, items_method) = match items_walk
        .as_ref()
        .map(|walk| walk.leaf.clone())
    {
        Some(Leaf::Items(_)) => (
            Some(allocate(&format!("{stem}Items"), types)),
            Some(allocate(&method_name(&format!("{stem}Items")), methods)),
        ),
        _ => (None, None),
    };
    Some(PaginatedOperation {
        op,
        page,
        api: plan.package().api_name.clone(),
        success: op.success_type.clone(),
        wrapped_body,
        pages,
        items,
        pages_method,
        items_method,
        next_page_method,
        advance,
        control,
        limit: limit_control
            .filter(|limit_control| !limit_control.required && limit_control.number)
            .zip(page.initial_limit),
        initial: page.initial_offset.unwrap_or(0),
        inject_initial: page.initial_offset.is_some()
            && matches!(control_role, Role::Offset | Role::Page),
        items_walk,
        continuation_walk,
        has_more_walk,
    })
}

/// Lower an allocated Pascal type name to its factory method spelling.
fn method_name(name: &str) -> String {
    let mut value = name.to_owned();
    value[..1].make_ascii_lowercase();
    value
}

/// The exact-integer view and construction helpers shared by every walk.
fn shared_helpers() -> &'static str {
    "    /** A repeated identical continuation value would never terminate. */\n    private static SdkException stalled(String source) {\n        return new SdkException(\"pagination-stalled\", source, 0, new byte[0], false);\n    }\n    /** A pagination control value the walk cannot represent exactly. */\n    private static SdkException refused(String source) {\n        return new SdkException(\"invalid-request\", source, 0, new byte[0], false);\n    }\n    /** Exact integer view of one control value. */\n    private static java.math.BigInteger exact(String source, JsonNumber value) {\n        try {\n            return value.exactIntegerValue();\n        } catch (RuntimeException unrepresentable) {\n            throw refused(source);\n        }\n    }\n    /** One exact control value from an advanced integer. */\n    private static JsonNumber pageInteger(String source, java.math.BigInteger value) {\n        try {\n            return JsonNumber.of(value.longValueExact());\n        } catch (ArithmeticException overflow) {\n            throw refused(source);\n        }\n    }\n"
}

/// Javadoc-escaped prose with continuation lines re-prefixed for a block
/// comment that already carries the leading `*` of the first line.
fn prose(text: &str) -> String {
    javadoc(text).replace('\n', "\n     * ")
}

/// The stop-rule prose shared by the walk documentation.
fn stop_rule(walk: &PaginatedOperation<'_>) -> &'static str {
    match walk.advance {
        Advance::CursorPointer => "when the next cursor is absent, null or empty",
        Advance::OffsetPointer => "when the next offset is absent or null",
        Advance::OffsetItemCount | Advance::PageNext => {
            if walk.has_more_walk.is_some() {
                "on a page with no items or when the source marks has-more false"
            } else {
                "on a page with no items"
            }
        }
    }
}

/// The typed reader for one resolved pointer.
fn reader_method(
    walk: &PaginatedOperation<'_>,
    reader: &Walk,
    pointer: &str,
    name: &str,
    comment: &str,
) -> String {
    let (returns, expression) = match &reader.leaf {
        Leaf::Items(element) => (format!("java.util.List<{element}>"), reader.binding.clone()),
        Leaf::Token => (
            "String".to_owned(),
            if matches!(reader.ty, JavaType::Number) {
                format!("{}.token()", reader.binding)
            } else {
                reader.binding.clone()
            },
        ),
        Leaf::Number => ("JsonNumber".to_owned(), reader.binding.clone()),
        Leaf::Flag => ("Boolean".to_owned(), reader.binding.clone()),
    };
    let root = if walk.wrapped_body {
        "            var value = page.data().isPresent() ? page.data().value() : null;"
    } else {
        "            var value = page.data();"
    };
    let mut out = String::new();
    out.push_str(&format!(
        "        /** {} Pointer: {{@code {}}}. An absent path reads as no value. */\n",
        javadoc(comment),
        pointer
    ));
    out.push_str(&format!(
        "        static {returns} {name}({}.{} page) {{\n",
        walk.api, walk.success
    ));
    out.push_str(root);
    out.push_str("\n            if (value == null) return null;\n");
    for step in &reader.steps {
        out.push_str(step);
        out.push('\n');
    }
    out.push_str(&format!("            return {expression};\n        }}\n"));
    out
}

/// The readers one compiled walk renders, in class-body order.
fn reader_methods(walk: &PaginatedOperation<'_>) -> String {
    let mut out = String::new();
    if let Some(items_walk) = &walk.items_walk {
        out.push_str(&reader_method(
            walk,
            items_walk,
            walk.page
                .response
                .get(&PaginationResponseRole::Items)
                .map(String::as_str)
                .unwrap_or_default(),
            "items",
            &format!(
                "Page items of one decoded {} page.",
                walk.op.method_name
            ),
        ));
    }
    if let Some((reader, role)) = &walk.continuation_walk {
        out.push_str(&reader_method(
            walk,
            reader,
            walk.page.response.get(role).map(String::as_str).unwrap_or_default(),
            match walk.advance {
                Advance::CursorPointer => "cursor",
                _ => "pageOffset",
            },
            &format!(
                "Continuation value of one decoded {} page.",
                walk.op.method_name
            ),
        ));
    }
    if let Some(reader) = &walk.has_more_walk {
        out.push_str(&reader_method(
            walk,
            reader,
            walk.page
                .response
                .get(&PaginationResponseRole::HasMore)
                .map(String::as_str)
                .unwrap_or_default(),
            "hasMore",
            &format!(
                "Declared has-more evidence of one decoded {} page.",
                walk.op.method_name
            ),
        ));
    }
    out
}

/// The continuation computation for the page after one fetched page.
fn continuation_method(walk: &PaginatedOperation<'_>) -> String {
    let api = &walk.api;
    let input = &walk.op.input_type;
    let control = &walk.control.member;
    let read = |fallback: &str| {
        if walk.control.required {
            format!("exact(SOURCE, input.{control}())")
        } else {
            format!("input.{control}().isPresent() ? exact(SOURCE, input.{control}().value()) : {fallback}")
        }
    };
    let previous = |fallback: &str| {
        if walk.control.required {
            format!("{} previous = input.{control}();\n", walk.control.native())
        } else {
            format!(
                "{} previous = input.{control}().isPresent() ? input.{control}().value() : {};\n",
                walk.control.native(),
                fallback
            )
        }
    };
    let stall = |value: &str, guard: &str| {
        format!(
            "            if ({guard}) throw stalled(SOURCE);\n            return rebuild(input, {value});\n"
        )
    };
    let mut body = String::new();
    match walk.advance {
        Advance::OffsetItemCount => {
            body.push_str(&format!(
                "            java.util.List<{}> items = items(page);\n            int count = items == null ? 0 : items.size();\n            if (count == 0) return null;\n",
                match walk.items_walk.as_ref().map(|walk| &walk.leaf) {
                    Some(Leaf::Items(element)) => element.clone(),
                    _ => unreachable!("items walk"),
                }
            ));
            if walk.has_more_walk.is_some() {
                body.push_str("            if (Boolean.FALSE.equals(hasMore(page))) return null;\n");
            }
            body.push_str(&format!(
                "            java.math.BigInteger base = {};\n            return rebuild(input, pageInteger(SOURCE, base.add(java.math.BigInteger.valueOf(count))));\n",
                read(&format!("java.math.BigInteger.valueOf({})", walk.initial))
            ));
        }
        Advance::PageNext => {
            body.push_str(&format!(
                "            java.util.List<{}> items = items(page);\n            int count = items == null ? 0 : items.size();\n            if (count == 0) return null;\n",
                match walk.items_walk.as_ref().map(|walk| &walk.leaf) {
                    Some(Leaf::Items(element)) => element.clone(),
                    _ => unreachable!("items walk"),
                }
            ));
            if walk.has_more_walk.is_some() {
                body.push_str("            if (Boolean.FALSE.equals(hasMore(page))) return null;\n");
            }
            body.push_str(&format!(
                "            java.math.BigInteger base = {};\n            return rebuild(input, pageInteger(SOURCE, base.add(java.math.BigInteger.ONE)));\n",
                read("java.math.BigInteger.ONE")
            ));
        }
        Advance::CursorPointer => {
            body.push_str(
                "            String token = cursor(page);\n            if (token == null || token.isEmpty()) return null;\n",
            );
            body.push_str(&previous("null"));
            body.push_str(&stall("token", "token.equals(previous)"));
        }
        Advance::OffsetPointer => {
            body.push_str(
                "            JsonNumber value = pageOffset(page);\n            if (value == null) return null;\n",
            );
            body.push_str(&previous("null"));
            if walk.control.required {
                body.push_str(&stall("value", "value.compareTo(previous) == 0"));
            } else {
                body.push_str(&stall("value", "previous != null && value.compareTo(previous) == 0"));
            }
        }
    }
    format!(
        "        /** Rebuilds the pagination controls for the page after the fetched one,\n         * or null when the walk stops. Every other input member stays untouched. */\n        private static {api}.{input} continuation({api}.{} page, {api}.{input} input) {{\n{body}        }}\n",
        walk.success
    )
}

/// One input with only the pagination control replaced.
fn rebuild_method(walk: &PaginatedOperation<'_>) -> String {
    let api = &walk.api;
    let input = &walk.op.input_type;
    let control = &walk.control.member;
    let mut statements = String::new();
    let factory_arguments = walk
        .op
        .constructor
        .arguments
        .iter()
        .map(|argument| {
            if argument.name == *control {
                control.clone()
            } else {
                format!("input.{}()", argument.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    statements.push_str(&format!(
        "            {api}.{input}.Builder builder = {api}.{input}.builder({factory_arguments});\n"
    ));
    for parameter in &walk.op.parameters {
        if parameter.required || parameter.native_name == *control {
            continue;
        }
        let omit = format!("omit{}", crate::rust_models::pascal(&parameter.native_name));
        statements.push_str(&format!(
            "            if (input.{}().isPresent()) builder.{}(input.{}().value()); else builder.{}();\n",
            parameter.native_name, parameter.native_name, parameter.native_name, omit
        ));
    }
    statements.push_str(&format!(
        "            builder.{control}({control});\n"
    ));
    if let Some(body) = &walk.op.body {
        if body.required {
            statements.push_str("            builder.body(input.body());\n");
        } else {
            statements.push_str(
                "            if (input.body().isPresent()) builder.body(input.body().value()); else builder.omitBody();\n",
            );
        }
    }
    statements.push_str("            return builder.build();\n");
    format!(
        "        /** One input with only the {control} control replaced; every other member\n         * is preserved exactly through the checked input builder. */\n        private static {api}.{input} rebuild({api}.{input} input, {} {control}) {{\n{statements}        }}\n",
        walk.control.native()
    )
}

/// The closeable page iterator class for one compiled walk.
#[allow(clippy::too_many_lines)]
fn pages_class(walk: &PaginatedOperation<'_>) -> String {
    let api = &walk.api;
    let input = &walk.op.input_type;
    let success = &walk.success;
    let label = format!("{} {}", walk.op.http_method, walk.op.path);
    let fallback = walk
        .limit
        .as_ref()
        .map(|(_, size)| {
            format!(
                " When the caller omits the limit, the first request supplies the documented SDK page size ({size}); later pages keep the limit the walk last used."
            )
        })
        .unwrap_or_default();
    let prose_text = prose(&format!(
        "Lazily walks every page of {label}, one request at a time.\n\nThe first page is exactly the direct {}(...) call's result, fetched by the first hasNext() and included exactly once; every later request rebuilds the input with only the {} pagination control changed, and a caller-supplied control wins for the first request.{fallback} The request for a not-yet-started page never fires, so close() or abandoning iteration stops the walk without another request. Stops {}. A source that repeats an identical continuation value fails the walk with an SdkException of kind pagination-stalled.\n\nSource: {}#{}",
        walk.op.method_name,
        walk.control.member,
        stop_rule(walk),
        walk.op.source.document(),
        walk.op.source.pointer()
    ));
    let next = if walk.inject_initial && walk.control.number {
        format!(
            "            this.next = input.{}().isPresent() ? input : rebuild(input, JsonNumber.of({}));\n",
            walk.control.member, walk.initial
        )
    } else {
        "            this.next = input;\n".to_owned()
    };
    // The documented SDK fallback page size applies to the first request only,
    // when the caller left the limit absent; later pages keep whatever limit
    // the walk last used.
    let limit_fill = walk
        .limit
        .as_ref()
        .map(|(limit_control, size)| {
            format!(
                "            this.next = this.next.{}().isPresent() ? this.next : rebuildLimit(this.next, JsonNumber.of({}));\n",
                limit_control.member, size
            )
        })
        .unwrap_or_default();
    format!(
        "    /**\n     * {prose_text}\n     */\n    public static final class {pages} implements java.util.Iterator<{api}.{success}>, AutoCloseable {{\n        private static final String SOURCE = {};\n        private final {api} client;\n        private {api}.{input} next;\n        private {api}.{success} pending;\n        private {pages}({api} client, {api}.{input} input) {{\n            this.client = client;\n{next}{limit_fill}        }}\n        /** Fetches the next page when required and reports whether one is available. The request for a not-yet-started page starts here, never on construction. */\n        @Override public boolean hasNext() {{\n            if (pending != null) return true;\n            if (next == null) return false;\n            {api}.{success} page = client.{method}(next);\n            next = continuation(page, next);\n            pending = page;\n            return true;\n        }}\n        /** The fetched page; exactly the direct call's result for the first page. */\n        @Override public {api}.{success} next() {{\n            if (!hasNext()) throw new java.util.NoSuchElementException(\"the pagination walk has ended\");\n            {api}.{success} page = pending;\n            pending = null;\n            return page;\n        }}\n        /** Stop the walk; no further request is issued. A fetched but unconsumed page is closed. */\n        @Override public void close() {{\n            next = null;\n            if (pending != null) {{\n                pending.close();\n                pending = null;\n            }}\n        }}\n{take_items}{readers}{continuation}{rebuild}{limit_rebuild}    }}\n",
        q(&format!(
            "{}#{}",
            walk.op.source.document(),
            walk.op.source.pointer()
        )),
        pages = walk.pages,
        method = walk.op.method_name,
        take_items = take_items_method(walk),
        readers = reader_methods(walk),
        continuation = continuation_method(walk),
        rebuild = rebuild_method(walk),
        limit_rebuild = limit_rebuild_method(walk),
        limit_fill = limit_fill,
    )
}

/// One input with only the fallback page-size member replaced; the limit fill
/// reuses the checked input builder exactly like the control rebuild.
fn limit_rebuild_method(walk: &PaginatedOperation<'_>) -> String {
    let Some((limit_control, _size)) = &walk.limit else {
        return String::new();
    };
    let api = &walk.api;
    let input = &walk.op.input_type;
    let control = &limit_control.member;
    let mut statements = String::new();
    let factory_arguments = walk
        .op
        .constructor
        .arguments
        .iter()
        .map(|argument| {
            if argument.name == *control {
                control.clone()
            } else {
                format!("input.{}()", argument.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    statements.push_str(&format!(
        "            {api}.{input}.Builder builder = {api}.{input}.builder({factory_arguments});\n"
    ));
    for parameter in &walk.op.parameters {
        if parameter.required || parameter.native_name == *control {
            continue;
        }
        let omit = format!("omit{}", crate::rust_models::pascal(&parameter.native_name));
        statements.push_str(&format!(
            "            if (input.{}().isPresent()) builder.{}(input.{}().value()); else builder.{}();\n",
            parameter.native_name, parameter.native_name, parameter.native_name, omit
        ));
    }
    statements.push_str(&format!(
        "            builder.{control}({control});\n"
    ));
    if let Some(body) = &walk.op.body {
        if body.required {
            statements.push_str("            builder.body(input.body());\n");
        } else {
            statements.push_str(
                "            if (input.body().isPresent()) builder.body(input.body().value()); else builder.omitBody();\n",
            );
        }
    }
    statements.push_str("            return builder.build();\n");
    format!(
        "        /** One input with only the {control} limit replaced; every other member\n         * is preserved exactly through the checked input builder. */\n        private static {api}.{input} rebuildLimit({api}.{input} input, {} {control}) {{\n{statements}        }}\n",
        limit_control.native()
    )
}

/// The page-consuming item source used by the item walk: consuming the pending
/// page before fetching another one is what keeps an empty page from stalling
/// the flattened iteration.
fn take_items_method(walk: &PaginatedOperation<'_>) -> String {
    let Some(Leaf::Items(element)) = &walk.items_walk.as_ref().map(|walk| &walk.leaf) else {
        return String::new();
    };
    format!(
        "        /** The next page's items; fetches the page when required and consumes it, or null at the end of the walk. */\n        java.util.List<{element}> takeItems() {{\n            if (pending == null && !hasNext()) return null;\n            {api}.{success} page = pending;\n            pending = null;\n            return items(page);\n        }}\n",
        api = walk.api,
        success = walk.success,
    )
}

/// The flattening item iterator class for one compiled walk.
fn items_class(walk: &PaginatedOperation<'_>) -> String {
    let (Some(items), _, Some(Leaf::Items(element))) =
        (&walk.items, &walk.items_method, walk.items_walk.as_ref().map(|walk| &walk.leaf))
    else {
        return String::new();
    };
    let pages = &walk.pages;
    let api = &walk.api;
    let input = &walk.op.input_type;
    let prose_text = prose(&format!(
        "Flattens the page items of {} across every {} page, in order. Items already fetched from the current page are returned without a request, so abandoning iteration never issues another one.\n\nSource: {}#{}",
        walk.op.method_name,
        walk.op.method_name,
        walk.op.source.document(),
        walk.op.source.pointer()
    ));
    format!(
        "    /**\n     * {prose_text}\n     */\n    public static final class {items} implements java.util.Iterator<{element}>, AutoCloseable {{\n        private final {pages} pages;\n        private java.util.ListIterator<{element}> queue = java.util.Collections.<{element}>emptyListIterator();\n        private {items}({api} client, {api}.{input} input) {{\n            this.pages = new {pages}(client, input);\n        }}\n        /** Pulls from the already-fetched page first; a request starts only when the previous page's items are exhausted, and an empty page cannot stall the walk. */\n        @Override public boolean hasNext() {{\n            while (!queue.hasNext()) {{\n                java.util.List<{element}> pageItems = pages.takeItems();
                if (pageItems == null) return false;
                queue = pageItems.listIterator();\n            }}\n            return true;\n        }}\n        /** The next item across all pages. */\n        @Override public {element} next() {{\n            if (!hasNext()) throw new java.util.NoSuchElementException(\"the pagination walk has ended\");\n            return queue.next();\n        }}\n        /** Stop the walk; no further request is issued. */\n        @Override public void close() {{\n            pages.close();\n        }}\n    }}\n"
    )
}

/// The public factory methods for one compiled walk.
fn factories(walk: &PaginatedOperation<'_>) -> String {
    let mut out = format!(
        "    /** Start a lazy page walk over {{@code input}}; the caller's pagination values win for the first request. Stops {stop}. */\n    public static {pages} {pages_method}({api} client, {api}.{input} input) {{\n        return new {pages}(client, input);\n    }}\n",
        stop = stop_rule(walk),
        pages = walk.pages,
        pages_method = walk.pages_method,
        api = walk.api,
        input = walk.op.input_type,
    );
    if let (Some(items), Some(items_method)) = (&walk.items, &walk.items_method) {
        out.push_str(&format!(
            "    /** Start a lazy item walk over {{@code input}}; see {{@link {pages}#hasNext()}}. */\n    public static {items} {items_method}({api} client, {api}.{input} input) {{\n        return new {items}(client, input);\n    }}\n",
            pages = walk.pages,
            items = items,
            items_method = items_method,
            api = walk.api,
            input = walk.op.input_type,
        ));
    }
    // With a configured fallback page size the manual builder reuses the page
    // walk's constructor so page 1 receives it exactly like the page and item
    // walks do; without one the direct fetch is byte-identical to before.
    let next_page_body = if walk.limit.is_some() {
        format!(
            "        {pages} walk = new {pages}(client, input);\n        return java.util.Optional.ofNullable(walk.hasNext() ? walk.next : null);\n",
            pages = walk.pages,
        )
    } else {
        format!(
            "        return java.util.Optional.ofNullable({pages}.continuation(client.{method}(input), input));\n",
            pages = walk.pages,
            method = walk.op.method_name,
        )
    };
    out.push_str(&format!(
        "    /** Fetches the page described by {{@code input}} and returns the rebuilt input for the following page, or empty when the walk stops. Issues exactly one request per call; see {{@link {pages}#hasNext()}} for the request-start semantics. */\n    public static java.util.Optional<{api}.{input}> {next_page_method}({api} client, {api}.{input} input) {{\n{next_page_body}    }}\n",
        pages = walk.pages,
        next_page_method = walk.next_page_method,
        api = walk.api,
        input = walk.op.input_type,
        next_page_body = next_page_body,
    ));
    out
}

/// The complete `Pagination.java` source body, or `None` when nothing paginates.
pub(crate) fn source(plan: &SdkPlan) -> Option<String> {
    let compiled = operations(plan);
    if compiled.is_empty() {
        return None;
    }
    let mut out = String::from(
        "/**\n * Generated pagination walks for the source-selected paginated operations of\n * this package.\n *\n * <p>Every walk keeps one request in flight: a request starts only inside\n * {@code hasNext()} when a not-yet-fetched page is required, the first page is\n * exactly the direct call's result included once, and every later request\n * rebuilds the input with only the pagination control changed. Calling\n * {@code close()} or abandoning iteration guarantees no further request. Stop\n * rules compile the generation-time pagination policy; there is no runtime\n * schema search or JSON re-parsing.\n */\npublic final class Pagination {\n    private Pagination() {}\n",
    );
    out.push_str(shared_helpers());
    for walk in &compiled {
        out.push_str(&factories(walk));
    }
    for walk in &compiled {
        out.push_str(&pages_class(walk));
    }
    for walk in &compiled {
        out.push_str(&items_class(walk));
    }
    out.push_str("}\n");
    Some(out)
}
