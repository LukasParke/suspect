//! Generated-only pagination emission for the Ruby gem.
//!
//! The shared, generation-time `http_protocol::plan_pagination` outcome
//! compiles into per-operation page/item `Enumerator` methods plus an explicit
//! next-input builder on the generated `Client` — all emitted into the
//! generated `client.rb` and its RBS signatures. The static runtime files and
//! the shared planner stay untouched, and plans without configured SDK
//! defaults (or without emittable paginated operations) emit no new bytes.
//!
//! Traversal semantics shared with every backend: pages are fetched lazily one
//! at a time and the first page is exactly the direct call's result included
//! once; later requests rebuild only the pagination controls while preserving
//! every other keyword argument; caller-supplied pagination values win for
//! page 1; the walk stops when the continuation pointer reads absent, nil,
//! UNSET or an empty string, on a zero-item page for contents-advanced walks,
//! and on a mapped has-more indicator reading false; a repeated identical
//! continuation value raises `SdkError` kind `:'pagination-stalled'` instead
//! of looping. RFC 6901 pointers resolve at generation time into native
//! accessor chains over the decoded response models — plain property access,
//! never JSONPath or schema search.

use std::{collections::BTreeSet, fmt::Write};

use super::{models, ModelPlan, NativeType, PlannedOperation, SdkPlan};
use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as wire;
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};
use suspect_ir::contract::Contract;

/// How one walk computes and applies its continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// The offset control advances by the previous page's item count.
    ItemsReturned,
    /// The one-based page control advances by one.
    PageNext,
    /// The page's mapped continuation pointer feeds the control.
    Pointer,
}

/// One compiled paginated operation, ready to render.
#[derive(Debug, Clone)]
pub struct PaginatedOperation {
    /// Index into `SdkPlan::operations`.
    pub operation: usize,
    /// The shared compiled pagination selection this emission follows.
    pub selection: wire::OperationPagination,
    /// Client method returning the page enumerator.
    pub pages_name: String,
    /// Client method returning the flattened item enumerator.
    pub items_name: String,
    /// Client method returning the next page's keyword arguments.
    pub next_page_name: String,
    control: String,
    /// The limit keyword carrying the documented SDK fallback page size, when
    /// the operation has one.
    limit: Option<String>,
    mode: Mode,
    /// Native access names from the decoded body to the item collection.
    items: Option<Vec<String>>,
    /// The RBS element type of the item collection, when statically known.
    item_type: Option<String>,
    /// Native access names to the continuation pointer, with its text flavor.
    continuation: Option<(Vec<String>, bool)>,
    has_more: Option<Vec<String>>,
}

/// The compiled pagination emission carried by one Ruby plan.
#[derive(Debug, Clone)]
pub struct PaginationPlan {
    /// The shared detection and override selection this emission follows.
    pub outcome: wire::PaginationOutcome,
    /// Allocated reader helper names, emitted beside the walkers.
    field_reader: String,
    count_reader: String,
    pub operations: Vec<PaginatedOperation>,
}

impl PaginationPlan {
    /// The allocated native member reader: declared members read their native
    /// attribute, plain decoded hashes read the exact wire key.
    pub(super) fn field_reader(&self) -> &str {
        &self.field_reader
    }
    /// The allocated collection counter: uncountable values count as zero.
    pub(super) fn count_reader(&self) -> &str {
        &self.count_reader
    }
}

impl PaginatedOperation {
    /// The index into `SdkPlan::operations`.
    pub(super) fn index(&self) -> usize {
        self.operation
    }
    /// The direct call keyword the walk rewrites between pages.
    pub(super) fn control(&self) -> &str {
        &self.control
    }
    /// The limit keyword the documented SDK fallback page size fills on page 1.
    pub(super) fn limit(&self) -> Option<&str> {
        self.limit.as_deref()
    }
    pub(super) fn mode(&self) -> Mode {
        self.mode
    }
    /// The configured initial offset, when one is mapped.
    pub(super) fn initial_offset(&self) -> Option<u32> {
        self.selection.initial_offset
    }
    /// The shared planner's operation identity.
    pub(super) fn identity(&self) -> &str {
        &self.selection.operation
    }
    /// The decoded-body source location for typed failures.
    pub(super) fn source(&self) -> String {
        let source = self.selection.source.use_site().source();
        format!("{}#{}", source.document(), source.pointer())
    }
    pub(super) fn items(&self) -> Option<&[String]> {
        self.items.as_deref()
    }
    pub(super) fn item_type(&self) -> Option<&str> {
        self.item_type.as_deref()
    }
    pub(super) fn continuation(&self) -> Option<(&[String], bool)> {
        self.continuation
            .as_ref()
            .map(|(chain, text)| (chain.as_slice(), *text))
    }
    pub(super) fn has_more(&self) -> Option<&[String]> {
        self.has_more.as_deref()
    }
}

/// The shared planner's operation identity for one planned operation:
/// the operation id, or `METHOD /path` when unnamed.
fn identity(operation: &PlannedOperation) -> String {
    operation
        .wire
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| {
            format!(
                "{} {}",
                operation.wire.method().as_str(),
                operation.wire.path()
            )
        })
}

/// Resolve one RFC 6901 pointer into native access names over the decoded
/// response models. Declared object members use their native attribute names;
/// anything else keeps the exact wire name for plain hash access, and
/// subsequent segments are no longer statically resolved.
fn chain(models: &ModelPlan, start: usize, pointer: &str) -> Vec<String> {
    let mut names = Vec::new();
    if pointer.is_empty() {
        return names;
    }
    let mut current = Some(start);
    for raw in pointer[1..].split('/') {
        let segment = raw.replace("~1", "/").replace("~0", "~");
        let Some(symbol) = current
            .map(|index| models.carrier(index))
            .filter(|symbol| matches!(symbol.shape, models::ModelShape::Object { .. }))
        else {
            names.push(segment);
            current = None;
            continue;
        };
        let models::ModelShape::Object { fields, .. } = &symbol.shape else {
            unreachable!("filtered above")
        };
        match fields.iter().find(|field| field.wire_name == segment) {
            Some(field) => {
                names.push(field.name.clone());
                current = Some(field.schema_index);
            }
            None => {
                names.push(segment);
                current = None;
            }
        }
    }
    names
}

/// The statically known element type at the items pointer, for RBS.
fn item_type(models: &ModelPlan, start: usize, pointer: &str) -> Option<String> {
    let mut current = Some(start);
    if !pointer.is_empty() {
        for raw in pointer[1..].split('/') {
            let segment = raw.replace("~1", "/").replace("~0", "~");
            let symbol = models.carrier(current?);
            let models::ModelShape::Object { fields, .. } = &symbol.shape else {
                return None;
            };
            current = fields
                .iter()
                .find(|field| field.wire_name == segment)
                .map(|field| field.schema_index);
        }
    }
    let symbol = models.carrier(current?);
    match &symbol.shape {
        models::ModelShape::Array {
            items: Some(item), ..
        } => Some(format!("Types::{}", models.symbol(*item)?.type_name)),
        _ => None,
    }
}

fn compile(
    models: &ModelPlan,
    operation: &PlannedOperation,
    page: &wire::OperationPagination,
) -> Option<Compiled> {
    if page.pattern == PaginationPattern::NextLink {
        return None;
    }
    // Walks read the decoded page through the single successful alternative's
    // typed JSON body.
    let success: Vec<_> = operation
        .responses
        .iter()
        .filter(|response| response.can_succeed())
        .collect();
    let [response] = success.as_slice() else {
        return None;
    };
    if response.media.len() != 1 {
        return None;
    }
    let NativeType::Codec(root) = response.media[0].value_type else {
        return None;
    };    let (mode, candidates) = match page.pattern {
        PaginationPattern::Cursor => (Mode::Pointer, vec![PaginationRequestRole::Cursor]),
        PaginationPattern::LimitOffset => {
            if page.advance == PaginationAdvance::NextOffset
                && page
                    .response
                    .contains_key(&PaginationResponseRole::NextOffset)
            {
                (
                    Mode::Pointer,
                    vec![
                        PaginationRequestRole::Offset,
                        PaginationRequestRole::Page,
                        PaginationRequestRole::Cursor,
                    ],
                )
            } else {
                (Mode::ItemsReturned, vec![PaginationRequestRole::Offset])
            }
        }
        PaginationPattern::PageNumber => {
            if page.advance == PaginationAdvance::NextOffset
                && page
                    .response
                    .contains_key(&PaginationResponseRole::NextOffset)
            {
                (
                    Mode::Pointer,
                    vec![
                        PaginationRequestRole::Page,
                        PaginationRequestRole::Offset,
                        PaginationRequestRole::Cursor,
                    ],
                )
            } else {
                (Mode::PageNext, vec![PaginationRequestRole::Page])
            }
        }
        PaginationPattern::NextLink => unreachable!("rejected above"),
    };
    let role = candidates
        .iter()
        .copied()
        .find(|role| page.request.contains_key(role))?;
    // The control keyword is the direct call's native keyword for the role's
    // wire-named query parameter.
    let control = operation
        .parameters
        .iter()
        .find(|parameter| {
            parameter.location() == wire::ParameterLocation::Query
                && parameter.wire.name() == page.request[&role]
        })?
        .keyword
        .clone();
    let text = role == PaginationRequestRole::Cursor;
    // The limit keyword is the direct call's native keyword for the limit
    // role's wire-named query parameter.
    let limit = page
        .request
        .get(&PaginationRequestRole::Limit)
        .and_then(|name| {
            operation
                .parameters
                .iter()
                .find(|parameter| {
                    parameter.location() == wire::ParameterLocation::Query
                        && parameter.wire.name() == name.as_str()
                })
                .map(|parameter| parameter.keyword.clone())
        });
    let items_pointer = page.response.get(&PaginationResponseRole::Items);
    let items = match mode {
        Mode::ItemsReturned | Mode::PageNext => Some(chain(models, root, items_pointer?)),
        Mode::Pointer => items_pointer.map(|pointer| chain(models, root, pointer)),
    };
    let item_type = items_pointer.and_then(|pointer| item_type(models, root, pointer));
    let continuation = match mode {
        Mode::Pointer => {
            let pointer = match page.pattern {
                PaginationPattern::Cursor => page
                    .response
                    .get(&PaginationResponseRole::NextCursor)
                    .or_else(|| page.response.get(&PaginationResponseRole::NextOffset)),
                _ => page.response.get(&PaginationResponseRole::NextOffset),
            }?;
            Some((chain(models, root, pointer), text))
        }
        Mode::ItemsReturned | Mode::PageNext => None,
    };
    let has_more = page
        .response
        .get(&PaginationResponseRole::HasMore)
        .map(|pointer| chain(models, root, pointer));
    Some(Compiled {
        control,
        limit,
        mode,
        items,
        item_type,
        continuation,
        has_more,
    })
}

struct Compiled {
    control: String,
    limit: Option<String>,
    mode: Mode,
    items: Option<Vec<String>>,
    item_type: Option<String>,
    continuation: Option<(Vec<String>, bool)>,
    has_more: Option<Vec<String>>,
}

/// Compile the shared pagination selection into this backend's emission plan.
/// Selections this plan cannot express with native accessor chains are
/// skipped: the operation stays an ordinary single-page call.
pub(super) fn plan(
    contract: &Contract,
    wire_plan: &wire::ProtocolPlan,
    defaults: Option<&crate::sdk_defaults::SdkDefaults>,
    operations: &[PlannedOperation],
    models: &ModelPlan,
    used: &mut BTreeSet<String>,
) -> Result<Option<PaginationPlan>, Vec<HttpDiagnostic>> {
    let Some(defaults) = defaults else {
        return Ok(None);
    };
    let outcome = wire::plan_pagination(contract, wire_plan, Some(defaults))?;
    let field_reader = models::allocate("pagination_field", used);
    let count_reader = models::allocate("pagination_count", used);
    let mut planned = Vec::new();
    for page in &outcome.paginated {
        let Some((index, operation)) = operations
            .iter()
            .enumerate()
            .find(|(_, operation)| identity(operation) == page.operation)
        else {
            continue;
        };
        let Some(compiled) = compile(models, operation, page) else {
            continue;
        };
        let pages_name = models::allocate(&format!("{}_pages", operation.method_name), used);
        let items_name = models::allocate(&format!("{}_items", operation.method_name), used);
        let next_page_name =
            models::allocate(&format!("{}_next_page", operation.method_name), used);
        planned.push(PaginatedOperation {
            operation: index,
            selection: page.clone(),
            pages_name,
            items_name,
            next_page_name,
            control: compiled.control,
            limit: compiled.limit,
            mode: compiled.mode,
            items: compiled.items,
            item_type: compiled.item_type,
            continuation: compiled.continuation,
            has_more: compiled.has_more,
        });
    }
    if planned.is_empty() {
        return Ok(None);
    }
    Ok(Some(PaginationPlan {
        outcome,
        field_reader,
        count_reader,
        operations: planned,
    }))
}

fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap().replace('#', "\\#")
}

/// A decoded-body accessor expression for one resolved pointer.
fn access(plan: &PaginationPlan, chain_names: &[String]) -> String {
    let mut expression = "page.data".to_owned();
    for name in chain_names {
        expression = format!(
            "{}({}, {})",
            plan.field_reader(),
            expression,
            q(name)
        );
    }
    expression
}

fn stall_raise(operation: &PaginatedOperation) -> String {
    format!(
        "raise RequestError.new('the source API returned an identical continuation value; the paginated walk would never terminate', kind: :'pagination-stalled', source: {}, operation_id: {})",
        q(&operation.source()),
        q(operation.identity()),
    )
}

/// The per-operation pagination methods emitted into the generated `Client`,
/// plus the two allocated reader helpers they share. Called only when the plan
/// carries emittable paginated operations.
pub(super) fn client_methods(plan: &SdkPlan, pagination: &PaginationPlan) -> String {
    let mut out = String::new();
    for operation in &pagination.operations {
        let op = &plan.operations()[operation.index()];
        let control = operation.control();
        let control_key = format!(":{control}");
        let pages_doc = pages_documentation(operation);
        let _ = write!(
            out,
            "    # {}\n    def {}(**kwargs)\n      Enumerator.new do |yielder|\n",
            pages_doc, operation.pages_name,
        );
        out.push_str("        request = kwargs.dup\n");
        if let (Some(limit), Some(size)) = (operation.limit(), operation.selection.initial_limit) {
            let _ = write!(
                out,
                "        limit = request[:{limit}]\n        request[:{limit}] = {size} if limit.nil? || limit.equal?(UNSET)\n"
            );
        }
        match operation.mode() {
            Mode::ItemsReturned => {
                let initial = operation.initial_offset().unwrap_or(0);
                let _ = write!(
                    out,
                    "        position = request[{control_key}]\n        position = {initial} if position.nil? || position.equal?(UNSET)\n"
                );
                if operation.initial_offset().is_some() {
                    let _ = writeln!(
                        out,
                        "        request[{control_key}] = position\n"
                    );
                }
                out.push_str("        loop do\n");
                let _ = writeln!(
                    out,
                    "          page = {}(**request)\n          yielder << page\n",
                    op.method_name
                );
                if let Some(items) = operation.items() {
                    let _ = writeln!(
                        out,
                        "          count = {}({})\n          break if count.zero?\n",
                        pagination.count_reader(),
                        access(pagination, items),
                    );
                } else {
                    out.push_str("          break\n");
                }
                if let Some(has_more) = operation.has_more() {
                    let _ = writeln!(
                        out,
                        "          break if {}.equal?(false)\n",
                        access(pagination, has_more)
                    );
                }
                let _ = writeln!(
                    out,
                    "          position += count\n          request[{control_key}] = position\n        end\n",
                );
            }
            Mode::PageNext => {
                let _ = write!(
                    out,
                    "        position = request[{control_key}]\n        position = 1 if position.nil? || position.equal?(UNSET)\n        loop do\n"
                );
                let _ = writeln!(
                    out,
                    "          page = {}(**request)\n          yielder << page\n",
                    op.method_name
                );
                if let Some(items) = operation.items() {
                    let _ = writeln!(
                        out,
                        "          count = {}({})\n          break if count.zero?\n",
                        pagination.count_reader(),
                        access(pagination, items),
                    );
                } else {
                    out.push_str("          break\n");
                }
                if let Some(has_more) = operation.has_more() {
                    let _ = writeln!(
                        out,
                        "          break if {}.equal?(false)\n",
                        access(pagination, has_more)
                    );
                }
                let _ = writeln!(
                    out,
                    "          position += 1\n          request[{control_key}] = position\n        end\n",
                );
            }
            Mode::Pointer => {
                let Some((continuation, text)) = operation.continuation() else {
                    continue;
                };
                let _ = write!(
                    out,
                    "        previous = request[{control_key}]\n        previous = nil if previous.equal?(UNSET)\n        loop do\n"
                );
                let _ = writeln!(
                    out,
                    "          page = {}(**request)\n          yielder << page\n",
                    op.method_name
                );
                let _ = writeln!(
                    out,
                    "          value = {}\n          break if value.nil? || value.equal?(UNSET){}\n",
                    access(pagination, continuation),
                    if text { " || value == ''" } else { "" },
                );
                let _ = writeln!(out, "          {} if previous == value\n", stall_raise(operation));
                let _ = writeln!(
                    out,
                    "          previous = value\n          request[{control_key}] = value\n        end\n",
                );
            }
        }
        out.push_str("      end\n    end\n\n");
        // Items: flattened across pages, in order, with the same laziness.
        let items_doc = format!(
            "Flattens the page items of {}(...) across every page, in order. Accepts\n    # the same keyword arguments as the direct call; see {} for lazy\n    # fetching, continuation, stopping and caller-override semantics.",
            op.method_name, operation.pages_name
        );
        if operation.items().is_some() {
            let _ = write!(
                out,
                "    # {items_doc}\n    def {}(**kwargs)\n      Enumerator.new do |yielder|\n        {}(**kwargs).each do |page|\n",
                operation.items_name, operation.pages_name,
            );
            if let Some(items) = operation.items() {
                let _ = writeln!(
                    out,
                    "          items = {}\n          items.each {{ |item| yielder << item }} if items.is_a?(Array)\n",
                    access(pagination, items),
                );
            }
            out.push_str("        end\n      end\n    end\n\n");
        }
        // Next page input, or nil when the walk stops.
        let _ = write!(
            out,
            "    # Issues one request for `kwargs` and returns the rebuilt keyword hash\n    # that fetches the following page, or nil when the walk stops. Every other\n    # input is preserved exactly; typed failures raise SdkError subclasses.\n    def {}(**kwargs)\n      request = kwargs.dup\n",
            operation.next_page_name,
        );
        if let (Some(limit), Some(size)) = (operation.limit(), operation.selection.initial_limit) {
            let _ = write!(
                out,
                "      limit = request[:{limit}]\n      request[:{limit}] = {size} if limit.nil? || limit.equal?(UNSET)\n"
            );
        }
        match operation.mode() {
            Mode::ItemsReturned => {
                let initial = operation.initial_offset().unwrap_or(0);
                let _ = write!(
                    out,
                    "      position = request[{control_key}]\n      position = {initial} if position.nil? || position.equal?(UNSET)\n"
                );
                if operation.initial_offset().is_some() {
                    let _ = writeln!(out, "      request[{control_key}] = position\n");
                }
                let _ = writeln!(out, "      page = {}(**request)\n", op.method_name);
                if let Some(items) = operation.items() {
                    let _ = writeln!(
                        out,
                        "      count = {}({})\n      return nil if count.zero?\n",
                        pagination.count_reader(),
                        access(pagination, items),
                    );
                }
                if let Some(has_more) = operation.has_more() {
                    let _ = writeln!(
                        out,
                        "      return nil if {}.equal?(false)\n",
                        access(pagination, has_more)
                    );
                }
                let _ = writeln!(
                    out,
                    "      request[{control_key}] = position + count\n      request\n",
                );
            }
            Mode::PageNext => {
                let _ = write!(
                    out,
                    "      position = request[{control_key}]\n      position = 1 if position.nil? || position.equal?(UNSET)\n"
                );
                let _ = writeln!(out, "      page = {}(**request)\n", op.method_name);
                if let Some(items) = operation.items() {
                    let _ = writeln!(
                        out,
                        "      count = {}({})\n      return nil if count.zero?\n",
                        pagination.count_reader(),
                        access(pagination, items),
                    );
                }
                if let Some(has_more) = operation.has_more() {
                    let _ = writeln!(
                        out,
                        "      return nil if {}.equal?(false)\n",
                        access(pagination, has_more)
                    );
                }
                let _ = writeln!(
                    out,
                    "      request[{control_key}] = position + 1\n      request\n",
                );
            }
            Mode::Pointer => {
                let Some((continuation, text)) = operation.continuation() else {
                    continue;
                };
                let _ = write!(
                    out,
                    "      previous = request[{control_key}]\n      previous = nil if previous.equal?(UNSET)\n"
                );
                let _ = writeln!(out, "      page = {}(**request)\n", op.method_name);
                let _ = writeln!(
                    out,
                    "      value = {}\n      return nil if value.nil? || value.equal?(UNSET){}\n      {} if previous == value\n      request[{control_key}] = value\n      request\n",
                    access(pagination, continuation),
                    if text { " || value == ''" } else { "" },
                    stall_raise(operation),
                );
            }
        }
        out.push_str("    end\n\n");
    }
    out.push_str(&format!(
        "    # @api private\n    def {}(value, name)\n      return nil if value.nil? || value.equal?(UNSET)\n      return value[name] if value.instance_of?(Hash)\n      value.respond_to?(name) ? value.public_send(name) : nil\n    end\n\n    # @api private\n    def {}(value)\n      value.respond_to?(:length) ? value.length : 0\n    end\n    private :{}, :{}\n",
        pagination.field_reader(),
        pagination.count_reader(),
        pagination.field_reader(),
        pagination.count_reader(),
    ));
    out
}

/// The RBS signatures for one plan's emitted pagination methods.
pub(super) fn signatures(plan: &SdkPlan, pagination: &PaginationPlan) -> String {
    let mut out = String::new();
    for operation in &pagination.operations {
        let op = &plan.operations()[operation.index()];
        let success = op
            .responses
            .iter()
            .find(|response| response.can_succeed())
            .map(|response| response.class_name.clone())
            .unwrap_or_else(|| "ApiResponse".to_owned());
        let _ = writeln!(
            out,
            "    def {}: (**untyped) -> Enumerator[{success}]\n",
            operation.pages_name,
        );
        if let Some(item_type) = operation.item_type() {
            let _ = writeln!(
                out,
                "    def {}: (**untyped) -> Enumerator[{item_type}]\n",
                operation.items_name,
            );
        }
        let _ = writeln!(
            out,
            "    def {}: (**untyped) -> Hash[Symbol, untyped]?\n",
            operation.next_page_name,
        );
    }
    out
}

fn pages_documentation(operation: &PaginatedOperation) -> String {
    let stop = match operation.mode() {
        Mode::ItemsReturned => {
            if operation.has_more().is_some() {
                "after a page with no items or when the has-more indicator is false"
            } else {
                "after a page with no items"
            }
        }
        Mode::PageNext => {
            if operation.has_more().is_some() {
                "after a page with no items or when the has-more indicator is false"
            } else {
                "after a page with no items"
            }
        }
        Mode::Pointer => {
            if operation.continuation().is_some_and(|(_, text)| text) {
                "when the continuation pointer is absent, nil, UNSET or an empty string"
            } else {
                "when the continuation pointer is absent, nil or UNSET"
            }
        }
    };
    format!(
        "Lazily yields every success page of {}. The first page is the direct\n    # call's result, fetched on the first next and included exactly once.\n    # Between requests only pagination controls change: every other keyword\n    # argument is preserved exactly, and a caller-supplied {} is used for the\n    # first request and replaced by the computed continuation afterwards.{} Stops\n    # {}. A repeated identical continuation value raises SdkError kind\n    # :'pagination-stalled' instead of looping. Iteration is lazy: no request\n    # happens before the first next, breaking out prevents further requests,\n    # and pages are never prefetched.",
        operation.identity(),
        operation.control(),
        operation.selection.initial_limit.map_or_else(String::new, |size| {
            format!("\n    # When the caller omits the limit, the first request supplies the\n    # documented SDK page size ({size}); later pages keep the limit the walk\n    # last used.")
        }),
        stop,
    )
}
