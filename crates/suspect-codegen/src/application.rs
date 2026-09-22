//! Shared, source-addressed application contract used by every native target
//! (CLI, MCP, and future targets) that binds directly to one HTTP operation.
//!
//! Every native target selects the outgoing operation it drives from the
//! canonical suspect_ir::contract::Contract with the same exact,
//! case-sensitive selector: an OpenAPI operationId, or the literal
//! "METHOD /path" form. There is no normalization, prefix matching, or
//! heuristic guessing. A selector that fails to identify exactly one
//! outgoing operation is refused with a located Diagnostic - the same
//! shape every target uses, so callers can report and sort refusals
//! uniformly regardless of which target produced them.

use std::ops::Range;

use suspect_ir::contract::{Contract, HttpMethod, Operation, SourceId};

/// A located refusal shared by every native application target.
/// mapping_pointer addresses the caller's own mapping document (an
/// application-specific configuration source, not the contract itself);
/// source/at address the underlying canonical OpenAPI contract where
/// available.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Stable machine-readable code, distinct per refusal reason.
    pub code: &'static str,
    /// Human-readable reason; raw source data remains available separately.
    pub message: String,
    /// JSON Pointer into the caller's own mapping document, not the contract.
    pub mapping_pointer: String,
    /// Contract source addressed by this diagnostic.
    pub source: SourceId,
    /// Original byte range of source, when the contract recorded one.
    pub at: Range<usize>,
}

impl Diagnostic {
    /// Builds a diagnostic located at source; at is filled from the
    /// contract's own recorded byte span, defaulting to an empty range when
    /// the source predates span tracking.
    #[must_use]
    pub fn new(
        contract: &Contract,
        source: SourceId,
        mapping_pointer: impl Into<String>,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        let at = contract.source_span(&source).unwrap_or(0..0);
        Self {
            code,
            message: message.into(),
            mapping_pointer: mapping_pointer.into(),
            source,
            at,
        }
    }
}

/// Contract root address, used for refusals with no single narrower source.
fn root(contract: &Contract) -> SourceId {
    SourceId::new(contract.entry().clone(), Default::default())
}

/// Stable code recorded when a manifest-visible contract document lies
/// outside the entry document's own directory tree.
pub const DOCUMENT_OUTSIDE_ENTRY_TREE: &str = "application-document-outside-entry-tree";

/// Stable code recorded when a resolved operation is absent from the canonical
/// SDK plan that was built to carry exactly it.
pub const OPERATION_UNPLANNED: &str = "application-path-item-unplanned";

/// Refusal for a selector that resolves against the contract but whose
/// operation the canonical SDK plan does not carry at that same source.
///
/// Both application targets ask the canonical native planner to plan exactly
/// the sources their mapping selected, and the planner normally records each
/// planned operation at the source it was selected by. A Path Item reached
/// through a `$ref` is the known exception: the contract resolves the selector
/// to the operation's own declaration while the native plan records it
/// elsewhere, so the mapping has no planned operation to bind to.
///
/// This is a refusal rather than an assertion because it is reachable from
/// ordinary input, and an unsupported combination must fail during generation
/// with a source-linked diagnostic - never with a panic.
#[must_use]
pub fn unplanned_operation(
    contract: &Contract,
    mapping_pointer: impl Into<String>,
    selector: &str,
    source: &SourceId,
) -> Diagnostic {
    Diagnostic::new(
        contract,
        source.clone(),
        mapping_pointer,
        OPERATION_UNPLANNED,
        format!(
            "selector {selector:?} resolves to the operation declared at {} {}, which the canonical SDK plan does not carry at that source; a Path Item reached through a `$ref` is outside this application target's bounded profile - declare the operation in the entry document's own `paths` instead",
            source.document().as_str(),
            source.pointer()
        ),
    )
}

/// `uri` has no machine-independent form under one [`DocumentScope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutsideEntryTree;

/// The contract documents an application may name in its deterministic
/// surface manifest: the entry document's own directory tree.
///
/// A surface manifest exists to be compared between generations and between
/// machines, so it records every contract document as a path *relative* to the
/// entry document's directory. A document outside that tree has no such
/// relative form, and recording its absolute URI would write the generating
/// machine's own filesystem layout into a review artifact - so two generations
/// of identical inputs on two machines would no longer compare equal.
///
/// Both application targets therefore admit every manifest-visible document
/// through this scope during planning and refuse anything outside it, rather
/// than relativizing what they can and silently leaking the rest.
#[derive(Debug, Clone)]
pub struct DocumentScope {
    /// The entry document's directory URI, including its trailing separator.
    /// `None` when the entry URI carries no separator at all, in which case
    /// the entry document itself is the only document in scope.
    directory: Option<String>,
    entry: String,
}

impl DocumentScope {
    /// The scope rooted at `contract`'s entry document directory.
    #[must_use]
    pub fn new(contract: &Contract) -> Self {
        let entry = contract.entry().as_str().to_owned();
        let directory = entry.rfind('/').map(|slash| entry[..=slash].to_owned());
        Self { directory, entry }
    }

    /// The manifest form of `uri`: its path relative to the entry directory.
    ///
    /// # Errors
    /// [`OutsideEntryTree`] when `uri` does not live under the entry
    /// document's directory.
    pub fn relative<'a>(&self, uri: &'a str) -> Result<&'a str, OutsideEntryTree> {
        match &self.directory {
            Some(directory) => uri.strip_prefix(directory.as_str()).ok_or(OutsideEntryTree),
            // A separator-free entry names no directory, so it is the only
            // document with a relative form, and that form is the entry itself.
            None => (uri == self.entry).then_some(uri).ok_or(OutsideEntryTree),
        }
    }

    /// Admit one manifest-visible document, addressed by the contract source
    /// that names it.
    ///
    /// # Errors
    /// A [`Diagnostic`] carrying [`DOCUMENT_OUTSIDE_ENTRY_TREE`], located at
    /// `source` and at `mapping_pointer`, when that source's document lies
    /// outside the entry document's directory tree.
    pub fn admit(
        &self,
        contract: &Contract,
        mapping_pointer: impl Into<String>,
        source: &SourceId,
    ) -> Result<(), Diagnostic> {
        let uri = source.document().as_str();
        if self.relative(uri).is_ok() {
            return Ok(());
        }
        Err(Diagnostic::new(
            contract,
            source.clone(),
            mapping_pointer,
            DOCUMENT_OUTSIDE_ENTRY_TREE,
            format!(
                "document {uri:?} lies outside the entry document's directory {:?}, so the application surface manifest cannot record it without writing an absolute generator path into a review artifact; move it inside the entry tree, or expose an operation that does not reach it",
                self.directory.as_deref().unwrap_or_default()
            ),
        ))
    }
}

/// Resolves selector to exactly one outgoing client operation.
///
/// selector is either an exact OpenAPI operationId, or the literal
/// "METHOD /path" form: a case-sensitive HTTP method token, a single space,
/// then the operation's exact path template including its leading slash.
/// Both forms require an exact match - no case-folding, trimming, prefix or
/// suffix matching, or path-parameter normalization is applied. Only
/// Contract::operations (outgoing client operations) are searched; webhook
/// and callback operations are never selected.
///
/// # Errors
/// A Diagnostic located at mapping_pointer when selector identifies zero
/// operations (application-operation-missing) or more than one
/// (application-operation-ambiguous).
pub fn select_operation<'a>(
    contract: &'a Contract,
    mapping_pointer: &str,
    selector: &str,
) -> Result<Operation<'a>, Diagnostic> {
    let matches: Vec<Operation<'a>> = match parse_selector(selector) {
        Selector::OperationId(id) => contract
            .operations()
            .filter(|op| op.operation_id() == Some(id))
            .collect(),
        Selector::MethodPath(method, path) => contract
            .operations()
            .filter(|op| op.method().as_str() == method && op.path_template() == Some(path))
            .collect(),
    };

    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(Diagnostic::new(
            contract,
            root(contract),
            mapping_pointer,
            "application-operation-missing",
            format!("selector {selector:?} does not identify any outgoing Contract operation"),
        )),
        found => Err(Diagnostic::new(
            contract,
            root(contract),
            mapping_pointer,
            "application-operation-ambiguous",
            format!(
                "selector {selector:?} must identify exactly one outgoing Contract operation (found {})",
                found.len()
            ),
        )),
    }
}

enum Selector<'a> {
    OperationId(&'a str),
    MethodPath(&'a str, &'a str),
}

/// Splits selector into its "METHOD /path" form when the text before the
/// first space is a valid HTTP method token and what follows starts with
/// a slash; otherwise the whole string is an operationId selector.
fn parse_selector(selector: &str) -> Selector<'_> {
    if let Some((method, path)) = selector.split_once(' ')
        && path.starts_with('/')
        && HttpMethod::parse(method).is_some()
    {
        return Selector::MethodPath(method, path);
    }
    Selector::OperationId(selector)
}
