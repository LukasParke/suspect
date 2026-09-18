//! Generated-only pagination emission for the TypeScript HTTP adapter.
//!
//! The compiled `PaginationOutcome` becomes one generated module holding a
//! frozen per-operation descriptor map plus a small generic walker. Request
//! controls are recorded as operation input members (already collision
//! qualified by the native plan) and response values stay RFC 6901 pointers
//! that the planner validated against the declared schemas; the walker resolves
//! them with plain property access, never schema search or JSONPath evaluation.
//! No static runtime file participates: with no paginated operations nothing is
//! emitted and the remaining package stays byte-identical.

use std::collections::BTreeMap;

use super::PlannedOperation;
use super::parameter_member;
use crate::http_protocol::{OperationPagination, PaginationOutcome};
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};
use suspect_ir::contract::ParameterLocation;

/// One operation's compiled pagination emission, ready to render.
pub(super) struct PaginatedOperation {
    /// Generated operation function name.
    pub function_name: String,
    /// Source operation identity for documentation.
    pub operation_id: String,
    /// Formatted source location for documentation comments.
    pub source: String,
    pub pattern: &'static str,
    pub advance: &'static str,
    /// JavaScript key -> operation input member carrying each request control.
    pub request: BTreeMap<&'static str, String>,
    /// JavaScript key -> RFC 6901 pointer into the decoded page body.
    pub response: BTreeMap<&'static str, String>,
    pub initial_offset: Option<u32>,
    /// The documented SDK fallback page size applied to page 1 only when the
    /// caller omitted the limit control; `None` emits `initialLimit: null`.
    pub initial_limit: Option<u32>,
    pub input_type: String,
    pub success_type: String,
    /// Yielded item type expression, derived from the emitted success union.
    pub item_type: String,
}

/// Compiles the paginated subset of the outcome against the native operations.
/// The planner already validated every request name and response pointer, so
/// this only re-keys the descriptor onto native member names.
pub(super) fn prepare(
    operations: &[PlannedOperation],
    outcome: &PaginationOutcome,
) -> Vec<PaginatedOperation> {
    outcome
        .paginated
        .iter()
        .filter_map(|page| {
            let operation = operations
                .iter()
                .find(|operation| operation.operation_id == page.operation)?;
            Some(compile(operation, page))
        })
        .collect()
}

fn compile(operation: &PlannedOperation, page: &OperationPagination) -> PaginatedOperation {
    let pattern = match page.pattern {
        PaginationPattern::LimitOffset => "limit-offset",
        PaginationPattern::Cursor => "cursor",
        PaginationPattern::PageNumber => "page-number",
        PaginationPattern::NextLink => "next-link",
    };
    let advance = match page.advance {
        PaginationAdvance::ItemsReturned => "items-returned",
        PaginationAdvance::NextOffset => "next-offset",
    };
    let request = page
        .request
        .iter()
        .map(|(role, name)| (request_key(*role), input_member(operation, name)))
        .collect();
    let response = page
        .response
        .iter()
        .map(|(role, pointer)| (response_key(*role), pointer.clone()))
        .collect();
    let items_pointer = page
        .response
        .get(&PaginationResponseRole::Items)
        .map(String::as_str)
        .unwrap_or_default();
    PaginatedOperation {
        function_name: operation.function_name.clone(),
        operation_id: operation.operation_id.clone(),
        source: super::src(&operation.source),
        pattern,
        advance,
        request,
        response,
        initial_offset: page.initial_offset,
        initial_limit: page.initial_limit,
        input_type: operation.input_type.clone(),
        success_type: operation.success_type.clone(),
        item_type: item_type(&operation.success_type, items_pointer),
    }
}

/// JavaScript descriptor keys for request roles.
fn request_key(role: PaginationRequestRole) -> &'static str {
    match role {
        PaginationRequestRole::Limit => "limit",
        PaginationRequestRole::Offset => "offset",
        PaginationRequestRole::Cursor => "cursor",
        PaginationRequestRole::Page => "page",
    }
}

/// JavaScript descriptor keys for response roles.
fn response_key(role: PaginationResponseRole) -> &'static str {
    match role {
        PaginationResponseRole::Items => "items",
        PaginationResponseRole::NextCursor => "nextCursor",
        PaginationResponseRole::NextOffset => "nextOffset",
        PaginationResponseRole::HasMore => "hasMore",
        PaginationResponseRole::Total => "total",
        PaginationResponseRole::TotalPages => "totalPages",
    }
}

/// The native input member carrying one wire-named query control.
fn input_member(operation: &PlannedOperation, wire_name: &str) -> String {
    operation
        .parameters
        .iter()
        .find(|parameter| {
            parameter.wire_name == wire_name && parameter.location == ParameterLocation::Query
        })
        .map(|parameter| parameter.native_name.clone())
        .unwrap_or_else(|| parameter_member(wire_name, ParameterLocation::Query))
}

/// The yielded item type as pure type-level extraction from the emitted
/// success union: object and array bodies of every success variant
/// participate, and members without the pointer contribute nothing. The
/// planner validated that the pointer resolves through declared properties,
/// so this never invents a schema lookup at runtime.
fn item_type(success: &str, pointer: &str) -> String {
    let mut expression = format!("PageBody<{success}>");
    if !pointer.is_empty() {
        for segment in pointer[1..].split('/') {
            let unescaped = segment.replace("~1", "/").replace("~0", "~");
            expression = format!("PageProperty<{expression},{}>", q(&unescaped));
        }
    }
    format!("PageItem<{expression}>")
}

fn q(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

/// The generated `typescript/pagination.ts` module: walker library plus the
/// frozen descriptor map for exactly this contract's paginated operations.
pub(super) fn emit_library(paginated: &[PaginatedOperation]) -> String {
    let mut code = String::from(LIBRARY);
    code.push_str("/** Frozen per-operation pagination descriptors compiled from the source policy.\n * Request values are the operation input members carrying each control (qualified when member names collide); response values are RFC 6901 pointers into the decoded page body; initialLimit is the documented SDK fallback page size for page 1 only (null when unconfigured).\n * Every initializer is annotated pure: it only freezes plain generated data, so a bundler may shed the whole map (and every operation's descriptors with it) when the consumer references no paginated walk.\n */\n");
    code.push_str("export const paginationDescriptors = /* @__PURE__ */ Object.freeze({");
    for operation in paginated {
        code.push('\n');
        code.push_str(&descriptor_entry(operation));
    }
    code.push_str("\n} as const);\n");
    code
}

fn descriptor_entry(operation: &PaginatedOperation) -> String {
    let render = |map: &BTreeMap<&'static str, String>| {
        map.iter()
            .map(|(key, value)| format!("{key}:{},", q(value)))
            .collect::<String>()
    };
    format!(
        "  {}: /* @__PURE__ */ Object.freeze({{ pattern: {}, request: /* @__PURE__ */ Object.freeze({{{}}} as const), response: /* @__PURE__ */ Object.freeze({{{}}} as const), initialOffset: {}, initialLimit: {}, advance: {} }} as const),",
        q(&operation.function_name),
        q(operation.pattern),
        render(&operation.request),
        render(&operation.response),
        operation
            .initial_offset
            .map_or_else(|| "null".to_owned(), |offset| offset.to_string()),
        operation
            .initial_limit
            .map_or_else(|| "null".to_owned(), |size| size.to_string()),
        q(operation.advance),
    )
}

/// The static walker library half of the generated module. Descriptors are
/// appended after this constant by `emit_library`.
const LIBRARY: &str = r#"// Generated pagination for this package's selected operations. Descriptors
// compile the generation-time pagination policy; the walker resolves RFC 6901
// pointers with plain property access only. It never searches schemas or
// evaluates paths, and a page that satisfies the pattern's stop rule ends the
// walk. The first page of every walk is exactly the direct call's result,
// except that a descriptor carrying a first-page limit fallback supplies it
// when the caller omitted the limit control; each later page rebuilds the
// input with only the pagination controls changed and preserves every other
// input member exactly, keeping whatever limit the walk last used.
/** Compiled pagination pattern kinds, matching the source policy vocabulary. */
export type PaginationPatternName = 'limit-offset' | 'cursor' | 'page-number' | 'next-link';
/** How one page determines the position of the next page. */
export type PaginationAdvanceName = 'items-returned' | 'next-offset';
/** The operation input members that carry each pagination control. */
export interface PaginationRequestControls {
    readonly limit?: string;
    readonly offset?: string;
    readonly cursor?: string;
    readonly page?: string;
}
/** RFC 6901 pointers into each decoded page body; `''` selects the body itself. */
export interface PaginationResponsePointers {
    readonly items?: string;
    readonly nextCursor?: string;
    readonly nextOffset?: string;
    readonly hasMore?: string;
    readonly total?: string;
    readonly totalPages?: string;
}
/** One operation's compiled pagination behavior. */
export interface PaginationDescriptor {
    readonly pattern: PaginationPatternName;
    readonly request: PaginationRequestControls;
    readonly response: PaginationResponsePointers;
    readonly initialOffset: number | null;
    /** The documented SDK fallback page size for page 1, or null when none is configured. It is an SDK convention, never a server default. */
    readonly initialLimit: number | null;
    readonly advance: PaginationAdvanceName;
}
/** The decoded bodies of one page result, distributing over the success union; primitive, null and absent bodies contribute nothing. */
export type PageBody<Page> = Page extends { readonly data: infer Body } ? (Body extends object ? Body : never) : never;
/** Reads one property of a page body union; members without the property contribute nothing. */
export type PageProperty<Body, Key extends string> = Body extends object ? (Key extends keyof Body ? Body[Key] : never) : never;
/** The element type of a page collection. */
export type PageItem<Collection> = Collection extends readonly (infer Item)[] ? Item : never;
/** Thrown instead of looping forever when a source API repeats an identical continuation value. */
export class PaginationError extends Error {
    readonly suspectPaginationError = true as const;
    constructor(message: string) {
        super(message);
        this.name = 'PaginationError';
    }
}
/** Tests whether a caught value is the generated pagination loop guard. */
export function isPaginationError(error: unknown): error is PaginationError {
    return error instanceof PaginationError
        || (typeof error === 'object' && error !== null && (error as { readonly suspectPaginationError?: unknown }).suspectPaginationError === true);
}
/** Resolves one RFC 6901 pointer with plain property access; absent paths read as undefined. */
function readPointer(value: unknown, pointer: string): unknown {
    if (pointer === '') return value;
    let current: unknown = value;
    for (const raw of pointer.slice(1).split('/')) {
        if (current === null || typeof current !== 'object') return undefined;
        const key = raw.replaceAll('~1', '/').replaceAll('~0', '~');
        if (!Object.hasOwn(current, key)) return undefined;
        current = (current as Record<string, unknown>)[key];
    }
    return current;
}
/** The decoded body of one page result. */
function pageBody(page: unknown): unknown {
    return (page as { readonly data?: unknown } | null | undefined)?.data;
}
/** The page's item collection: the array at the items pointer, or nothing. */
function pageItems(page: unknown, descriptor: PaginationDescriptor): readonly unknown[] {
    const value = readPointer(pageBody(page), descriptor.response.items ?? '');
    return Array.isArray(value) ? value : [];
}
/** The current value of one control member in an input record. */
function controlValue(input: object, member: string): string | number | undefined {
    const value = (input as Record<string, unknown>)[member];
    return typeof value === 'string' || typeof value === 'number' ? value : undefined;
}
/** Rebuilds one input record with a single control member changed. */
function withControl<Input extends object>(input: Input, member: string, value: unknown): Input {
    return Object.assign({}, input, { [member]: value }) as Input;
}
/**
 * The first page's input: when the descriptor carries the documented SDK
 * fallback page size and the caller omitted the limit control, the first
 * request supplies it. Later pages never re-apply it; they keep whatever
 * limit the walk last used because continuations preserve the control.
 */
function firstInput<Input extends object>(descriptor: PaginationDescriptor, input: Input): Input {
    if (descriptor.initialLimit === null) return input;
    const member = descriptor.request.limit;
    if (member === undefined) return input;
    if (controlValue(input, member) !== undefined) return input;
    return withControl(input, member, descriptor.initialLimit);
}
/** One computed continuation: the rebuilt input record for the next page. */
interface Continuation<Input> {
    readonly input: Input;
}
/**
 * Computes the next input for one fetched page, or null when the walk stops.
 *
 * Stop rules: limit-offset walks stop when the compiled `hasMore` pointer reads
 * false or when a page yields zero items (the documented fallback when a source
 * has no explicit end marker); cursor walks stop when the continuation pointer
 * reads absent, null or the empty string; page-number walks without
 * continuation pointers stop on zero items, false `hasMore`, or the last
 * declared page. A continuation value identical to the one that produced the
 * current page throws PaginationError instead of looping forever.
 */
function continuation<Input extends object>(
    descriptor: PaginationDescriptor,
    page: unknown,
    input: Input,
): Continuation<Input> | null {
    const body = pageBody(page);
    const request = descriptor.request;
    const response = descriptor.response;
    if (descriptor.advance === 'items-returned') {
        if (response.hasMore !== undefined && readPointer(body, response.hasMore) === false) return null;
        const count = pageItems(page, descriptor).length;
        if (count === 0) return null;
        const member = request.offset;
        if (member === undefined) return null;
        const current = controlValue(input, member) ?? descriptor.initialOffset ?? 0;
        if (typeof current !== 'number') return null;
        return { input: withControl(input, member, current + count) };
    }
    if (response.nextOffset !== undefined) {
        const value = readPointer(body, response.nextOffset);
        if (value === undefined || value === null || value === '') return null;
        const member = request.offset ?? request.page ?? request.cursor;
        if (member === undefined) return null;
        if (controlValue(input, member) === value) {
            throw new PaginationError('the source API returned an identical continuation value; the paginated walk would never terminate');
        }
        return { input: withControl(input, member, value) };
    }
    if (response.nextCursor !== undefined) {
        const value = readPointer(body, response.nextCursor);
        if (value === undefined || value === null || value === '') return null;
        const member = request.cursor ?? request.offset ?? request.page;
        if (member === undefined) return null;
        if (controlValue(input, member) === value) {
            throw new PaginationError('the source API returned an identical continuation value; the paginated walk would never terminate');
        }
        return { input: withControl(input, member, value) };
    }
    if (descriptor.pattern === 'page-number') {
        const member = request.page;
        if (member === undefined) return null;
        if (response.hasMore !== undefined && readPointer(body, response.hasMore) === false) return null;
        if (pageItems(page, descriptor).length === 0) return null;
        const value = controlValue(input, member) ?? 1;
        if (typeof value !== 'number') return null;
        if (response.totalPages !== undefined) {
            const total = readPointer(body, response.totalPages);
            if (typeof total === 'number' && value >= total) return null;
        }
        return { input: withControl(input, member, value + 1) };
    }
    // next-link descriptors carry no followable pointer in pagination v1, and
    // pointer-less cursor walks have no documented continuation; both stop.
    return null;
}
/**
 * Lazily yields every page result of one paginated operation. The first page is
 * exactly the direct call's result; every later page rebuilds the input with
 * only the pagination controls changed. The request for a not-yet-started page
 * never fires, so early `break` and call cancellation stop the walk cleanly.
 */
export async function* walkPages<Input extends object, Page>(
    descriptor: PaginationDescriptor,
    input: Input,
    fetchPage: (input: Input) => Promise<Page>,
): AsyncIterable<Page> {
    let current = firstInput(descriptor, input);
    for (;;) {
        const page = await fetchPage(current);
        yield page;
        const next = continuation(descriptor, page, current);
        if (next === null) return;
        current = next.input;
    }
}
/**
 * Lazily yields every item across all pages at the compiled items pointer.
 * Early `break` never starts the next page request and call cancellation
 * propagates through every page.
 */
export async function* walkItems<Input extends object, Page, Item>(
    descriptor: PaginationDescriptor,
    input: Input,
    fetchPage: (input: Input) => Promise<Page>,
): AsyncIterable<Item> {
    let current = firstInput(descriptor, input);
    for (;;) {
        const page = await fetchPage(current);
        for (const item of pageItems(page, descriptor)) {
            yield item as Item;
        }
        const next = continuation(descriptor, page, current);
        if (next === null) return;
        current = next.input;
    }
}
/**
 * Fetches the page described by `input` and returns the rebuilt input record
 * for the following page, or null when the walk stops, so callers can drive
 * pages manually.
 */
export async function nextPageInput<Input extends object, Page>(
    descriptor: PaginationDescriptor,
    fetchPage: (input: Input) => Promise<Page>,
    input: Input,
): Promise<Input | null> {
    const first = firstInput(descriptor, input);
    const page = await fetchPage(first);
    return continuation(descriptor, page, first)?.input ?? null;
}
"#;
