//! Emitted pagination helpers over the shared pagination selection.
//!
//! The shared planner decides WHICH operations paginate and which request
//! controls and response pointers participate. This module lowers that
//! selection into generated Go: one lazy page walk and item walk per paginated
//! operation, an explicit next-input builder and a typed traversal error.
//!
//! Emission is strictly conditional: without a configured policy, or without
//! any paginated operation, the backend emits nothing new, so no-policy output
//! stays byte-identical. Pointers resolve at generation time into generated
//! accessor expressions over the decoded response models; there is no runtime
//! schema search and no JSON re-parsing.

use std::collections::{BTreeMap, BTreeSet};

use super::emit::q;
use super::*;
use crate::go_models::{GoDecl, GoType};
use crate::rust_models::RepresentationRole;
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};

/// How one walk rebuilds its input between pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmittedAdvance {
    /// The offset control advances by the previous page's item count. The
    /// documented stop rules are a zero-item page or `has-more` false.
    OffsetItemCount,
    /// The page-number control advances by one (the documented 1-based
    /// fallback when no server continuation pointer is mapped).
    PageNext,
    /// The next-cursor pointer feeds the cursor control. The walk stops when
    /// the pointer is absent, null or empty; a repeated identical cursor is a
    /// typed pagination error.
    CursorPointer,
    /// The next-offset pointer feeds the offset or page control. The walk
    /// stops when the pointer is absent or null; a repeated identical offset
    /// is a typed pagination error.
    OffsetPointer,
}

/// One request control's generated shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// Input struct field name.
    pub field: String,
    /// Whether the input field is an `Optional[T]` wrapper.
    pub optional: bool,
    /// Model name when the control lowers to a string literal type and the
    /// generated code must convert the plain token.
    pub literal: Option<String>,
}

impl Control {
    /// The Go expression storing `value` into this input field.
    fn assignment(&self, value: &str) -> String {
        let converted = self
            .literal
            .as_ref()
            .map_or_else(|| value.to_owned(), |model| format!("{model}({value})"));
        if self.optional {
            format!("OptionalSome({converted})")
        } else {
            converted
        }
    }
    /// The Go expression reading this control's `Integer` value.
    fn integer(&self) -> String {
        if self.optional {
            format!("it.input.{}.Value", self.field)
        } else {
            format!("it.input.{}", self.field)
        }
    }
}

/// One response pointer's generated reader.
#[derive(Debug, Clone)]
pub struct Accessor {
    /// The complete unexported Go method on the concrete success response type.
    pub code: String,
    /// Element type for items accessors; the item walk flattens these.
    pub item: Option<String>,
}

/// One operation's compiled pagination emission.
#[derive(Debug, Clone)]
pub struct PaginationOperation {
    /// The shared selection this emission follows.
    pub pagination: protocol::OperationPagination,
    /// The client method this walk extends, e.g. `ListWidgets`.
    pub method: String,
    pub input_type: String,
    pub success_type: String,
    /// Concrete success response wrapper the walk reads.
    pub response: String,
    /// Allocated walk entry points and iterator types.
    pub pages: String,
    pub items: Option<String>,
    pub next_page: String,
    pub page_iterator: String,
    pub item_iterator: Option<String>,
    /// Request controls by role.
    pub controls: BTreeMap<PaginationRequestRole, Control>,
    /// The control the continuation writes between pages.
    pub continuation_control: Control,
    pub advance: EmittedAdvance,
    pub initial_offset: Option<u32>,
    pub items_accessor: Option<Accessor>,
    pub cursor_accessor: Option<Accessor>,
    pub offset_accessor: Option<Accessor>,
    pub has_more_accessor: Option<Accessor>,
}

/// Compiled pagination helpers carried by one plan.
#[derive(Debug, Clone)]
pub struct PaginationPlan {
    /// The shared detection and override selection this emission follows.
    pub outcome: protocol::PaginationOutcome,
    /// Allocated typed pagination error, present only when helpers are emitted.
    pub error_type: Option<String>,
    pub operations: Vec<PaginationOperation>,
}

impl PaginationPlan {
    /// Whether this plan emits the pagination helpers at all.
    #[must_use]
    pub fn emits(&self) -> bool {
        !self.operations.is_empty()
    }
}

#[derive(Debug, Clone)]
struct Level {
    /// Go field name used to reach this level; `None` for the decoded body.
    field: Option<String>,
    /// Raw Go type of the value at this level, wrappers included.
    ty: GoType,
}

#[derive(Debug, Clone)]
struct Resolved {
    levels: Vec<Level>,
}

/// Unwrap wrapper and alias layers into the (wrappers, plain value) pair.
fn deref(descriptors: &crate::go_models::GoDescriptors, ty: GoType) -> (Vec<Wrapper>, GoType) {
    let mut wrappers = Vec::new();
    let mut current = ty;
    loop {
        let next = match &current {
            GoType::Nullable(inner) => {
                wrappers.push(Wrapper::Nullable);
                (**inner).clone()
            }
            GoType::Optional(inner) => {
                wrappers.push(Wrapper::Optional);
                (**inner).clone()
            }
            GoType::Presence(inner) => {
                wrappers.push(Wrapper::Presence);
                (**inner).clone()
            }
            GoType::Pointer(inner) => {
                wrappers.push(Wrapper::Pointer);
                (**inner).clone()
            }
            GoType::Named(key) => match descriptors.declarations.get(key) {
                Some(GoDecl::Alias(alias)) => alias.clone(),
                _ => return (wrappers, current),
            },
            plain => return (wrappers, plain.clone()),
        };
        current = next;
    }
}

/// One wrapper shape the generated reader knows how to guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wrapper {
    Optional,
    Nullable,
    Presence,
    Pointer,
}

/// Resolve one RFC 6901 pointer into the decoded response model, starting at
/// the concrete success representation.
fn resolve_pointer(
    models: &crate::go_models::ModelPlan,
    root: &SchemaId,
    pointer: &str,
) -> Result<Resolved, String> {
    let descriptors = models.descriptors();
    let mut levels = vec![Level {
        field: None,
        ty: GoType::Named((root.clone(), RepresentationRole::Model)),
    }];
    if pointer.is_empty() {
        return Ok(Resolved { levels });
    }
    for raw in pointer[1..].split('/') {
        let segment = raw.replace("~1", "/").replace("~0", "~");
        let last = levels.last().expect("started").clone();
        let (_, inner) = deref(descriptors, last.ty);
        let key = match &inner {
            GoType::Named(key) => key,
            _ => {
                return Err(format!(
                    "segment {segment:?} reads through a non-object representation"
                ));
            }
        };
        let Some(GoDecl::Struct { fields, .. }) = descriptors.declarations.get(key) else {
            return Err(format!(
                "segment {segment:?} reads through a non-object representation"
            ));
        };
        let field = fields
            .iter()
            .find(|field| field.wire == segment)
            .ok_or_else(|| {
                format!("segment {segment:?} is not a declared property of the response model")
            })?;
        levels.push(Level {
            field: Some(field.name.clone()),
            ty: field.ty.clone(),
        });
    }
    Ok(Resolved { levels })
}

fn final_type(models: &crate::go_models::ModelPlan, resolved: &Resolved) -> (Vec<Wrapper>, GoType) {
    let last = resolved.levels.last().expect("started");
    deref(models.descriptors(), last.ty.clone())
}

/// Emit one unexported reader method on the concrete success response type.
/// Every wrapper along the pointer guards with the absent return; the value is
/// fully unwrapped and converted before return. `present` is the suffix after
/// the converted value (empty for slice readers, ", true" for flag readers).
#[allow(clippy::too_many_arguments)]
fn accessor_code(
    response: &str,
    method: &str,
    returns: &str,
    absent: &str,
    comment: &str,
    resolved: &Resolved,
    present: &str,
    convert: impl Fn(&str) -> String,
    models: &crate::go_models::ModelPlan,
) -> String {
    let mut body = String::new();
    let mut expression = String::from("page.Data");
    for (index, level) in resolved.levels.iter().enumerate() {
        let (wrappers, _) = deref(models.descriptors(), level.ty.clone());
        for wrapper in wrappers {
            let guard = match wrapper {
                Wrapper::Optional => format!("\tif !{expression}.IsSet {{\n\t\t{absent}\n\t}}\n"),
                Wrapper::Nullable => format!("\tif !{expression}.IsValue {{\n\t\t{absent}\n\t}}\n"),
                Wrapper::Presence => {
                    format!(
                        "\tif !{expression}.IsSet || {expression}.Null {{\n\t\t{absent}\n\t}}\n"
                    )
                }
                Wrapper::Pointer => format!("\tif {expression} == nil {{\n\t\t{absent}\n\t}}\n"),
            };
            body.push_str(&guard);
            if wrapper != Wrapper::Pointer {
                expression.push_str(".Value");
            }
        }
        if let Some(field) = resolved
            .levels
            .get(index + 1)
            .and_then(|l| l.field.as_ref())
        {
            expression.push('.');
            expression.push_str(field);
        }
    }
    body.push_str(&format!("\treturn {}{}\n", convert(&expression), present));
    format!("// {comment}\nfunc (page {response}) {method}() {returns} {{\n{body}}}\n")
}

/// The success representation the shared planner reads: the first exact/range
/// 2xx response with a typed JSON body.
fn success_response(operation: &PlannedOperation) -> Option<&PlannedResponse> {
    operation.responses.iter().find(|response| {
        matches!(
            response.status(),
            protocol::ResponseStatus::Exact(200..=299) | protocol::ResponseStatus::Range(2)
        ) && !response.forbidden_body
            && response.media.as_ref().is_some_and(|media| {
                matches!(
                    media.wire.representation(),
                    protocol::Representation::Json { codec: Some(_) }
                )
            })
    })
}

type StringReader = Box<dyn Fn(&str) -> String>;

fn string_reader(model: &crate::go_models::ModelPlan, inner: &GoType) -> Option<StringReader> {
    let descriptors = model.descriptors();
    match inner {
        GoType::Primitive("string") => Some(Box::new(|expression: &str| expression.to_owned())),
        GoType::Primitive("Integer") => Some(Box::new(|expression: &str| {
            format!("{expression}.String()")
        })),
        GoType::Named(key) => match descriptors.declarations.get(key) {
            Some(GoDecl::Literals {
                underlying: "string",
                ..
            }) => Some(Box::new(move |expression: &str| {
                format!("string({expression})")
            })),
            _ => None,
        },
        _ => None,
    }
}

fn control(
    operation: &PlannedOperation,
    symbols: &BTreeMap<SchemaId, String>,
    models: &crate::go_models::ModelPlan,
    role: PaginationRequestRole,
    name: &str,
) -> Result<Control, String> {
    let parameter = operation
        .parameters
        .iter()
        .find(|p| {
            p.wire.location() == protocol::ParameterLocation::Query && p.wire.name() == name
        })
        .ok_or_else(|| {
            format!("request role {role:?} names {name:?}, which is not a query parameter of the planned operation")
        })?;
    let model = symbols
        .get(parameter.schema())
        .cloned()
        .ok_or_else(|| format!("request role {role:?} parameter {name:?} has no native model"))?;
    let (wrappers, inner) = deref(
        models.descriptors(),
        GoType::Named((parameter.schema().clone(), RepresentationRole::Model)),
    );
    if wrappers.iter().any(|wrapper| {
        matches!(
            wrapper,
            Wrapper::Nullable | Wrapper::Presence | Wrapper::Pointer
        )
    }) {
        return Err(format!(
            "request role {role:?} parameter {name:?} is nullable, which this emission cannot use as a pagination control"
        ));
    }
    let literal = match &inner {
        GoType::Primitive("string") | GoType::Primitive("Integer") => None,
        GoType::Named(key) => match models.descriptors().declarations.get(key) {
            Some(GoDecl::Literals {
                underlying: "string",
                ..
            }) => Some(model),
            _ => {
                return Err(format!(
                    "request role {role:?} parameter {name:?} must be string- or integer-typed"
                ));
            }
        },
        _ => {
            return Err(format!(
                "request role {role:?} parameter {name:?} must be string- or integer-typed"
            ));
        }
    };
    Ok(Control {
        field: parameter.field_name.clone(),
        optional: !parameter.wire.required(),
        literal,
    })
}

/// Emit one accessor from a resolved pointer.
#[allow(clippy::too_many_arguments)]
fn build_accessor(
    response: &str,
    method: &str,
    returns: &str,
    absent: &str,
    comment: &str,
    resolved: &Resolved,
    present: &str,
    convert: impl Fn(&str) -> String,
    models: &crate::go_models::ModelPlan,
) -> Accessor {
    Accessor {
        code: accessor_code(
            response, method, returns, absent, comment, resolved, present, convert, models,
        ),
        item: None,
    }
}

/// Lower one shared pagination selection into this backend's emission plan.
#[allow(clippy::too_many_arguments)]
fn lower(
    pagination: &protocol::OperationPagination,
    operations: &[PlannedOperation],
    symbols: &BTreeMap<SchemaId, String>,
    models: &crate::go_models::ModelPlan,
    names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> Result<PaginationOperation, String> {
    use PaginationRequestRole as Role;
    use PaginationResponseRole as Pointer;
    let operation = operations
        .iter()
        .find(|op| op.operation_id == pagination.operation)
        .ok_or_else(|| {
            format!(
                "pagination selects {:?}, which is not a planned operation",
                pagination.operation
            )
        })?;
    if pagination.pattern == PaginationPattern::NextLink {
        return Err(
            "the Go HTTP backend cannot emit next-link helpers: a continuation URL cannot replace the request target through the typed input"
                .into(),
        );
    }
    let mut controls = BTreeMap::new();
    for (role, name) in &pagination.request {
        controls.insert(*role, control(operation, symbols, models, *role, name)?);
    }
    let response = success_response(operation).ok_or_else(|| {
        "the pagination selection has no typed JSON success representation to read".to_owned()
    })?;
    let media = response.media().expect("media checked");
    let root = media.schema().expect("typed codec checked").clone();

    let read = |role: Pointer, name: &str| {
        pagination
            .response
            .get(&role)
            .map(|pointer| {
                resolve_pointer(models, &root, pointer)
                    .map_err(|error| format!("{name} pointer {pointer:?}: {error}"))
            })
            .transpose()
    };
    let items_pointer = read(Pointer::Items, "items")?;
    let cursor_pointer = read(Pointer::NextCursor, "next-cursor")?;
    let offset_pointer = read(Pointer::NextOffset, "next-offset")?;
    let has_more_pointer = read(Pointer::HasMore, "has-more")?;

    let items = items_pointer.map(|resolved| {
        let (_, inner) = final_type(models, &resolved);
        let GoType::Slice(element) = &inner else {
            return Err(format!(
                "the items pointer reads a {}, which is not an array",
                inner.render(&models.descriptors().names)
            ));
        };
        let item = element.render(&models.descriptors().names);
        let mut built = build_accessor(
            &response.type_name,
            "httpPaginationItems",
            &format!("[]{item}"),
            "return nil",
            "httpPaginationItems returns the page's items at the configured pointer;\n// an absent path reads as no items.",
            &resolved,
            "",
            |expression| expression.to_owned(),
            models,
        );
        built.item = Some(item);
        Ok(built)
    });
    let cursor = cursor_pointer.map(|resolved| {
        let (_, inner) = final_type(models, &resolved);
        let Some(convert) = string_reader(models, &inner) else {
            return Err(format!(
                "the next-cursor pointer reads a {}, which cannot serve as a cursor",
                inner.render(&models.descriptors().names)
            ));
        };
        Ok(build_accessor(
            &response.type_name,
            "httpPaginationCursor",
            "(string, bool)",
            r#"return "", false"#,
            "httpPaginationCursor returns the continuation cursor and whether the\n// page carries one.",
            &resolved,
            ", true",
            convert,
            models,
        ))
    });
    let offset = offset_pointer.map(|resolved| {
        let (_, inner) = final_type(models, &resolved);
        if !matches!(inner, GoType::Primitive("Integer")) {
            return Err(format!(
                "the next-offset pointer reads a {}, which cannot serve as an exact offset",
                inner.render(&models.descriptors().names)
            ));
        }
        Ok(build_accessor(
            &response.type_name,
            "httpPaginationOffset",
            "(Integer, bool)",
            "return Integer{}, false",
            "httpPaginationOffset returns the continuation offset and whether the\n// page carries one.",
            &resolved,
            ", true",
            |expression| expression.to_owned(),
            models,
        ))
    });
    let has_more = has_more_pointer.map(|resolved| {
        let (_, inner) = final_type(models, &resolved);
        if !matches!(inner, GoType::Primitive("bool")) {
            return Err(format!(
                "the has-more pointer reads a {}, which cannot serve as has-more evidence",
                inner.render(&models.descriptors().names)
            ));
        }
        Ok(build_accessor(
            &response.type_name,
            "httpPaginationHasMore",
            "(bool, bool)",
            "return false, false",
            "httpPaginationHasMore reports the page's has-more evidence.",
            &resolved,
            ", true",
            |expression| expression.to_owned(),
            models,
        ))
    });
    let (items, cursor, offset, has_more) = (
        items.transpose()?,
        cursor.transpose()?,
        offset.transpose()?,
        has_more.transpose()?,
    );

    let advance = match pagination.pattern {
        PaginationPattern::Cursor => EmittedAdvance::CursorPointer,
        PaginationPattern::LimitOffset => match (pagination.advance, offset.is_some()) {
            (PaginationAdvance::NextOffset, true) => EmittedAdvance::OffsetPointer,
            _ => EmittedAdvance::OffsetItemCount,
        },
        PaginationPattern::PageNumber => match (pagination.advance, offset.is_some()) {
            (PaginationAdvance::NextOffset, true) => EmittedAdvance::OffsetPointer,
            _ => EmittedAdvance::PageNext,
        },
        PaginationPattern::NextLink => unreachable!("rejected above"),
    };
    let (continuation_role, control_role) = match advance {
        EmittedAdvance::CursorPointer => (Pointer::NextCursor, Role::Cursor),
        EmittedAdvance::PageNext => (Pointer::NextOffset, Role::Page),
        _ => (Pointer::NextOffset, Role::Offset),
    };
    if matches!(
        advance,
        EmittedAdvance::OffsetItemCount | EmittedAdvance::PageNext
    ) && items.is_none()
    {
        return Err(
            "the walk advances by page contents, which requires a resolved items pointer".into(),
        );
    }
    if advance == EmittedAdvance::CursorPointer && cursor.is_none() {
        return Err("a cursor walk requires a resolved next-cursor pointer".into());
    }
    let continuation_control = controls.get(&control_role).cloned().ok_or_else(|| {
        format!("the {advance:?} walk requires a request {control_role:?} control")
    })?;
    if advance == EmittedAdvance::OffsetPointer
        && !pagination.response.contains_key(&continuation_role)
    {
        return Err("the next-offset advance requires a resolved next-offset pointer".into());
    }

    let pages = allocate(&format!("{}Pages", operation.method_name), methods);
    let next_page = allocate(&format!("{}NextPage", operation.method_name), methods);
    let page_iterator = allocate(&format!("{}PageIterator", operation.method_name), names);
    let (items_method, item_iterator) = items
        .is_some()
        .then(|| {
            (
                allocate(&format!("{}Items", operation.method_name), methods),
                allocate(&format!("{}ItemIterator", operation.method_name), names),
            )
        })
        .unzip();
    Ok(PaginationOperation {
        pagination: pagination.clone(),
        method: operation.method_name.clone(),
        input_type: operation.input_type.clone(),
        success_type: operation.success_type.clone(),
        response: response.type_name.clone(),
        pages,
        items: items_method,
        next_page,
        page_iterator,
        item_iterator,
        controls,
        continuation_control,
        advance,
        initial_offset: pagination.initial_offset,
        items_accessor: items,
        cursor_accessor: cursor,
        offset_accessor: offset,
        has_more_accessor: has_more,
    })
}

/// Compile the shared pagination selection into this backend's emission plan.
/// A configured policy with no paginated operations keeps the outcome on the
/// plan but emits nothing.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan(
    contract: &Contract,
    wire: &protocol::ProtocolPlan,
    defaults: Option<&crate::sdk_defaults::SdkDefaults>,
    operations: &[PlannedOperation],
    symbols: &BTreeMap<SchemaId, String>,
    codecs: &crate::go_codecs::CodecPlan,
    names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> Result<Option<PaginationPlan>, Vec<HttpDiagnostic>> {
    let Some(defaults) = defaults else {
        return Ok(None);
    };
    let outcome = protocol::plan_pagination(contract, wire, Some(defaults))?;
    let mut lowered = Vec::new();
    let mut errors = Vec::new();
    for pagination in &outcome.paginated {
        match lower(
            pagination,
            operations,
            symbols,
            codecs.models(),
            names,
            methods,
        ) {
            Ok(operation) => lowered.push(operation),
            Err(message) => errors.push(super::diagnostic(
                contract,
                pagination.source.use_site().source().clone(),
                "http-go-pagination-unsupported",
                message,
            )),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let error_type = (!lowered.is_empty()).then(|| allocate("PaginationError", names));
    Ok(Some(PaginationPlan {
        outcome,
        error_type,
        operations: lowered,
    }))
}

/// Render `go/pagination.go`: the shared walk rules plus one walk per
/// paginated operation. Called only when at least one operation is paginated.
pub(super) fn emit(plan: &HttpPlan, pagination: &PaginationPlan) -> String {
    let error_type = pagination
        .error_type
        .as_deref()
        .unwrap_or("PaginationError");
    let mut code = String::from(
        "// Code generated by suspect. DO NOT EDIT.\n//\n// Pagination helpers follow the application-configured pagination policy.\n// Each walk keeps one request in flight: the first Next issues page 1\n// exactly once, honoring the caller's pagination values, and every later\n// page changes only the pagination controls while preserving every other\n// input member. Early loop exit, cancellation and errors never issue another\n// request. Stop rules and continuation mechanics are generation-time policy;\n// there is no runtime schema search or JSON re-parsing.\npackage sdk\n\nimport (\n\t\"context\"\n\t\"math/big\"\n\t\"strconv\"\n)\n\n",
    );
    code.push_str(&format!(
        "// {error_type} reports a pagination-specific traversal failure: a repeated or\n// otherwise non-advancing continuation value, an unadvanceable control, or a\n// success representation the walk cannot read. Transport, request-validation\n// and decoding failures surface through Err unchanged.\ntype {error_type} struct {{\n\t// Operation identifies the paginated operation by its source identity.\n\tOperation string\n\t// Reason describes the failure without echoing response data.\n\tReason string\n\t// Cause optionally carries the underlying error.\n\tCause error\n}}\n\nfunc (e *{error_type}) Error() string {{\n\tif e.Operation == \"\" {{\n\t\treturn \"pagination failure: \" + e.Reason\n\t}}\n\treturn \"pagination failure for \" + e.Operation + \": \" + e.Reason\n}}\n\nfunc (e *{error_type}) Unwrap() error {{ return e.Cause }}\n\nfunc paginationFailure(operation, reason string) *{error_type} {{\n\treturn &{error_type}{{Operation: operation, Reason: reason}}\n}}\n\n// httpPageInteger builds an exact integer control value from a native count.\nfunc httpPageInteger(value int64) Integer {{\n\ttoken, err := ParseInteger(strconv.FormatInt(value, 10))\n\tif err != nil {{\n\t\tpanic(err) // unreachable: FormatInt output is always an exact integer token\n\t}}\n\treturn token\n}}\n\n// httpPageAdvance returns the exact integer token advanced by one native\n// count. Continuations must be canonical base-10 integers; anything else is a\n// typed pagination failure instead of a silent guess.\nfunc httpPageAdvance(current Integer, count int64) (Integer, error) {{\n\tvalue, ok := new(big.Int).SetString(current.String(), 10)\n\tif !ok {{\n\t\treturn Integer{{}}, paginationFailure(\"\", \"the current continuation is not an exact base-10 integer, so the walk cannot advance it\")\n\t}}\n\treturn ParseInteger(value.Add(value, big.NewInt(count)).String())\n}}\n\n"
    ));
    for operation in &pagination.operations {
        code.push_str(&operation_code(plan, pagination, operation));
    }
    code
}

fn operation_code(
    _plan: &HttpPlan,
    pagination: &PaginationPlan,
    operation: &PaginationOperation,
) -> String {
    let error_type = pagination
        .error_type
        .as_deref()
        .unwrap_or("PaginationError");
    let identity = q(&operation.pagination.operation);
    let mut code = String::new();
    for accessor in [
        operation.items_accessor.as_ref(),
        operation.cursor_accessor.as_ref(),
        operation.offset_accessor.as_ref(),
        operation.has_more_accessor.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        code.push_str(&accessor.code);
    }

    let state = match operation.advance {
        EmittedAdvance::OffsetItemCount => "\toffset  Integer\n",
        EmittedAdvance::PageNext => "\tnumber  Integer\n",
        EmittedAdvance::CursorPointer | EmittedAdvance::OffsetPointer => "\tprevious string\n",
    };
    let stop = match operation.advance {
        EmittedAdvance::OffsetItemCount | EmittedAdvance::PageNext => {
            if operation.has_more_accessor.is_some() {
                "The walk stops on a page with no items, when the source marks has-more\n// false, or on failure."
            } else {
                "The walk stops on a page with no items, or on failure."
            }
        }
        EmittedAdvance::CursorPointer => {
            "The walk stops when the next cursor is absent, null or empty, or on\n// failure; a repeated identical cursor is a typed pagination error."
        }
        EmittedAdvance::OffsetPointer => {
            "The walk stops when the next offset is absent or null, or on failure;\n// a repeated identical offset is a typed pagination error."
        }
    };
    let initial = match operation.advance {
        EmittedAdvance::OffsetItemCount | EmittedAdvance::OffsetPointer => {
            match (&operation.continuation_control, operation.initial_offset) {
                (control, Some(value)) if control.optional => format!(
                    "\t\tif !it.input.{}.IsSet {{\n\t\t\tit.input.{} = OptionalSome(httpPageInteger({value}))\n\t\t}}\n",
                    control.field, control.field
                ),
                _ => String::new(),
            }
        }
        EmittedAdvance::PageNext => {
            match (&operation.continuation_control, operation.initial_offset) {
                (control, Some(value)) if control.optional => format!(
                    "\t\tif !it.input.{}.IsSet {{\n\t\t\tit.input.{} = OptionalSome(httpPageInteger({value}))\n\t\t}}\n",
                    control.field, control.field
                ),
                (control, None) if control.optional => format!(
                    "\t\tif it.input.{}.IsSet {{\n\t\t\tit.number = it.input.{}.Value\n\t\t}} else {{\n\t\t\tit.number = httpPageInteger(1)\n\t\t}}\n",
                    control.field, control.field
                ),
                _ => String::new(),
            }
        }
        EmittedAdvance::CursorPointer => String::new(),
    };
    let track = match operation.advance {
        EmittedAdvance::OffsetItemCount => {
            let read = operation.continuation_control.integer();
            if operation.continuation_control.optional && operation.initial_offset.is_none() {
                format!(
                    "\t\tif it.input.{}.IsSet {{\n\t\t\tit.offset = {read}\n\t\t}}\n",
                    operation.continuation_control.field
                )
            } else {
                format!("\t\tit.offset = {read}\n")
            }
        }
        EmittedAdvance::PageNext => {
            let read = operation.continuation_control.integer();
            if operation.continuation_control.optional && operation.initial_offset.is_some() {
                format!("\t\tit.number = {read}\n")
            } else if operation.continuation_control.optional {
                String::new() // handled by the initial branch above
            } else {
                format!("\t\tit.number = {read}\n")
            }
        }
        EmittedAdvance::CursorPointer => {
            let control = &operation.continuation_control;
            if control.optional {
                format!(
                    "\t\tif it.input.{}.IsSet {{\n\t\t\tit.previous = it.input.{}.Value\n\t\t}}\n",
                    control.field, control.field
                )
            } else {
                format!("\t\tit.previous = it.input.{}\n", control.field)
            }
        }
        EmittedAdvance::OffsetPointer => {
            let control = &operation.continuation_control;
            if control.optional {
                format!(
                    "\t\tif it.input.{}.IsSet {{\n\t\t\tit.previous = it.input.{}.Value.String()\n\t\t}}\n",
                    control.field, control.field
                )
            } else {
                format!("\t\tit.previous = it.input.{}.String()\n", control.field)
            }
        }
    };
    let limit_fill = match (
        operation.pagination.initial_limit,
        operation.controls.get(&PaginationRequestRole::Limit),
    ) {
        // A required limit control has no "omitted" state to fall back from,
        // and an unset optional control takes the documented SDK page size on
        // page 1 only; later pages keep whatever limit the walk last used.
        (Some(size), Some(control)) if control.optional => format!(
            "\t\tif !it.input.{}.IsSet {{\n\t\t\tit.input.{} = {}\n\t\t}}\n",
            control.field,
            control.field,
            control.assignment(&format!("httpPageInteger({size})")),
        ),
        _ => String::new(),
    };
    let mut first_page = limit_fill;
    first_page.push_str(&initial);
    first_page.push_str(&track);
    let first_page = first_page.trim_end_matches('\n');
    let limit_doc = operation
        .pagination
        .initial_limit
        .map_or_else(String::new, |size| {
            format!(
                "// When the caller leaves the limit unset, page 1 supplies the documented\n// SDK page size ({size}); later pages keep the limit the walk last used.\n"
            )
        });
    code.push_str(&format!(
        "// {page_iterator} walks {method} one page at a time with one request in flight.\n// Next fetches the next page; Page returns the fetched response. {stop}\n{limit_doc}type {page_iterator} struct {{\n\tclient  *Client\n\tctx     context.Context\n\tinput   {input}\n\treply   {result}\n\tpage    {response}\n{state}\tfetched bool\n\tdone    bool\n\terr     error\n}}\n\n// {pages} returns a lazy page walk for {method}. The first Next issues page 1\n// exactly once, honoring the caller's pagination values; every later page\n// changes only the pagination controls and preserves every other input\n// member. Errors surface through Err: the operation's own error unchanged, or\n// a *{error_type} for pagination-specific failures.\nfunc (c *Client) {pages}(ctx context.Context, input {input}) *{page_iterator} {{\n\treturn &{page_iterator}{{client: c, ctx: ctx, input: input}}\n}}\n\n// Next fetches the next page and reports whether one was fetched. The final\n// page that ends the walk is still fetched and returned exactly once. After a\n// false result, Err carries the cause; exhaustion without failure leaves Err\n// nil.\nfunc (it *{page_iterator}) Next() bool {{\n\tif it.err != nil || it.done {{\n\t\treturn false\n\t}}\n\tif it.fetched {{\n\t\tmore, err := it.nextInput()\n\t\tif err != nil {{\n\t\t\tit.err = err\n\t\t\treturn false\n\t\t}}\n\t\tif !more {{\n\t\t\tit.done = true\n\t\t\treturn false\n\t\t}}\n\t}} else {{\n\t\tit.fetched = true\n{first_page}\n\t}}\n\treply, err := it.client.{method}(it.ctx, it.input)\n\tif err != nil {{\n\t\tit.err = err\n\t\treturn false\n\t}}\n\tpage, ok := reply.({response})\n\tif !ok {{\n\t\tit.err = paginationFailure({identity}, \"the walk met a success representation the pagination plan cannot read\")\n\t\treturn false\n\t}}\n\tit.reply, it.page = reply, page\n\treturn true\n}}\n\n// Page returns the response fetched by the last successful Next, or the zero\n// value before the first call or after a failure.\nfunc (it *{page_iterator}) Page() {result} {{\n\treturn it.reply\n}}\n\n// Err returns the terminal cause after Next reports false, or nil while the\n// walk is healthy. Cancellation surfaces as the context's error.\nfunc (it *{page_iterator}) Err() error {{\n\treturn it.err\n}}\n\n// Close stops the walk. Fetched pages are buffered and need no other cleanup.\nfunc (it *{page_iterator}) Close() {{\n\tit.done = true\n}}\n\n",
        page_iterator = operation.page_iterator,
        method = operation.method,
        stop = stop,
        input = operation.input_type,
        result = operation.success_type,
        response = operation.response,
        state = state,
        pages = operation.pages,
        error_type = error_type,
        identity = identity,
        first_page = first_page,
        limit_doc = limit_doc,
    ));

    let control = &operation.continuation_control;
    let write_control = |value: &str| control.assignment(value);
    let next_input = match operation.advance {
        EmittedAdvance::OffsetItemCount => {
            let has_more = operation
                .has_more_accessor
                .as_ref()
                .map(|_| format!(
                    "\tif value, present := it.page.{}(); present && !value {{\n\t\treturn false, nil\n\t}}\n",
                    "httpPaginationHasMore"
                ))
                .unwrap_or_default();
            let guard = if control.optional && operation.initial_offset.is_none() {
                format!(
                    "\tif it.offset.String() == \"\" {{\n\t\treturn false, paginationFailure({identity}, \"the walk cannot compute the next offset because page 1 sent no offset\")\n\t}}\n",
                    identity = identity
                )
            } else {
                String::new()
            };
            format!(
                "\tcount := int64(len(it.page.httpPaginationItems()))\n\tif count == 0 {{\n\t\treturn false, nil\n\t}}\n{has_more}{guard}\tnext, err := httpPageAdvance(it.offset, count)\n\tif err != nil {{\n\t\treturn false, err\n\t}}\n\tit.offset = next\n\tit.input.{field} = {assignment}\n\treturn true, nil\n",
                has_more = has_more,
                guard = guard,
                field = control.field,
                assignment = write_control("next"),
            )
        }
        EmittedAdvance::PageNext => {
            let has_more = operation
                .has_more_accessor
                .as_ref()
                .map(|_| "\tif value, present := it.page.httpPaginationHasMore(); present && !value {\n\t\treturn false, nil\n\t}\n".to_owned())
                .unwrap_or_default();
            format!(
                "\tcount := int64(len(it.page.httpPaginationItems()))\n\tif count == 0 {{\n\t\treturn false, nil\n\t}}\n{has_more}\tnext, err := httpPageAdvance(it.number, 1)\n\tif err != nil {{\n\t\treturn false, err\n\t}}\n\tit.number = next\n\tit.input.{field} = {assignment}\n\treturn true, nil\n",
                has_more = has_more,
                field = control.field,
                assignment = write_control("next"),
            )
        }
        EmittedAdvance::CursorPointer => format!(
            "\ttoken, present := it.page.httpPaginationCursor()\n\tif !present || token == \"\" {{\n\t\treturn false, nil\n\t}}\n\tif token == it.previous {{\n\t\treturn false, paginationFailure({identity}, \"the server repeated the same continuation cursor\")\n\t}}\n\tit.previous = token\n\tit.input.{field} = {assignment}\n\treturn true, nil\n",
            identity = identity,
            field = control.field,
            assignment = write_control("token"),
        ),
        EmittedAdvance::OffsetPointer => format!(
            "\ttoken, present := it.page.httpPaginationOffset()\n\tif !present {{\n\t\treturn false, nil\n\t}}\n\tif token.String() == it.previous {{\n\t\treturn false, paginationFailure({identity}, \"the server repeated the same continuation offset\")\n\t}}\n\tit.previous = token.String()\n\tit.input.{field} = {assignment}\n\treturn true, nil\n",
            identity = identity,
            field = control.field,
            assignment = write_control("token"),
        ),
    };
    code.push_str(&format!(
        "// nextInput rebuilds the pagination controls for the page after the\n// fetched one, reporting whether the walk continues. Every other input\n// member stays untouched.\nfunc (it *{page_iterator}) nextInput() (bool, error) {{\n{next_input}}}\n\n",
        page_iterator = operation.page_iterator,
        next_input = next_input,
    ));

    if let (Some(items_method), Some(item_iterator), Some(accessor)) = (
        operation.items.as_ref(),
        operation.item_iterator.as_ref(),
        operation.items_accessor.as_ref(),
    ) {
        let item = accessor.item.as_deref().unwrap_or("Value");
        code.push_str(&format!(
            "// {item_iterator} flattens the items of one {pages} walk in order. Next pulls\n// from the already-fetched page first, so early loop exit never issues\n// another request; cancellation and timeouts propagate through the page\n// walk's context.\ntype {item_iterator} struct {{\n\tpages *{page_iterator}\n\titems []{item}\n\tindex int\n\terr   error\n}}\n\n// {items_method} returns a lazy item walk over {method} pages. The context,\n// cancellation and error policy are the page walk's.\nfunc (c *Client) {items_method}(ctx context.Context, input {input}) *{item_iterator} {{\n\treturn &{item_iterator}{{pages: c.{pages}(ctx, input)}}\n}}\n\n// Next advances to the next item across pages and reports whether one is\n// available. Item returns it. After false, Err carries the page walk's cause.\nfunc (it *{item_iterator}) Next() bool {{\n\tfor {{\n\t\tif it.index < len(it.items) {{\n\t\t\tit.index++\n\t\t\treturn true\n\t\t}}\n\t\tif it.err != nil {{\n\t\t\treturn false\n\t\t}}\n\t\tif !it.pages.Next() {{\n\t\t\tit.err = it.pages.Err()\n\t\t\treturn false\n\t\t}}\n\t\tit.items = it.pages.page.httpPaginationItems()\n\t\tit.index = 0\n\t}}\n}}\n\n// Item returns the current item; valid only after a true Next.\nfunc (it *{item_iterator}) Item() {item} {{\n\treturn it.items[it.index-1]\n}}\n\n// Err returns the terminal cause after Next reports false.\nfunc (it *{item_iterator}) Err() error {{\n\treturn it.err\n}}\n\n",
            item_iterator = item_iterator,
            pages = operation.pages,
            page_iterator = operation.page_iterator,
            items_method = items_method,
            method = operation.method,
            input = operation.input_type,
            item = item,
        ));
    }

    code.push_str(&format!(
        "// {next_page} fetches the page for input and returns the rebuilt input for\n// the following page, reporting false at the end of the walk or when the page\n// fetch fails. Use [{pages}] when the failure cause matters.\nfunc (c *Client) {next_page}(ctx context.Context, input {input}) ({input}, bool) {{\n\titerator := c.{pages}(ctx, input)\n\tdefer iterator.Close()\n\tif !iterator.Next() {{\n\t\tvar zero {input}\n\t\treturn zero, false\n\t}}\n\tmore, err := iterator.nextInput()\n\tif err != nil || !more {{\n\t\tvar zero {input}\n\t\treturn zero, false\n\t}}\n\treturn iterator.input, true\n}}\n\n",
        next_page = operation.next_page,
        pages = operation.pages,
        input = operation.input_type,
    ));
    code
}
