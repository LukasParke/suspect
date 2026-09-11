use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use suspect_ir::contract::{
    Contract, ContractSeverity, Header, MediaType, Operation, Parameter, ParameterLocation,
    Response, ResponseStatus, SchemaId, SourceId,
};

use super::{Change, CompatibilityError, Direction, Impact, Location, change, schema};

#[derive(Debug, Clone)]
pub(super) struct OperationMatch {
    pub before: Option<SourceId>,
    pub after: Option<SourceId>,
}

pub(super) struct WireResult {
    pub changes: Vec<Change>,
    pub matches: Vec<OperationMatch>,
    pub schemas: schema::Correspondence,
}

#[derive(Clone, Copy)]
struct Pair<'a, 'b> {
    before: Option<Operation<'a>>,
    after: Option<Operation<'b>>,
}

/// IDs have precedence over method/path. Only unmatched unique routes are used
/// to recognize a rename; an operation's position in the document is not identity.
fn pair<'a, 'b>(old: &[Operation<'a>], new: &[Operation<'b>]) -> Vec<Pair<'a, 'b>> {
    let mut old_used = BTreeSet::new();
    let mut new_used = BTreeSet::new();
    let mut result = Vec::new();
    for use_id in [true, false] {
        let key = |op: &Operation<'_>| {
            if use_id {
                op.operation_id()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            } else {
                Some(format!(
                    "{} {}",
                    op.method().as_str(),
                    op.path_template().unwrap_or("")
                ))
            }
        };
        let mut left: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut right: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, op) in old
            .iter()
            .enumerate()
            .filter(|(index, _)| !old_used.contains(index))
        {
            if let Some(key) = key(op) {
                left.entry(key).or_default().push(index);
            }
        }
        for (index, op) in new
            .iter()
            .enumerate()
            .filter(|(index, _)| !new_used.contains(index))
        {
            if let Some(key) = key(op) {
                right.entry(key).or_default().push(index);
            }
        }
        for (key, indices) in left {
            if let ([a], Some(b)) = (indices.as_slice(), right.get(&key))
                && let [b] = b.as_slice()
            {
                old_used.insert(*a);
                new_used.insert(*b);
                result.push(Pair {
                    before: Some(old[*a]),
                    after: Some(new[*b]),
                });
            }
        }
    }
    result.extend(
        old.iter()
            .enumerate()
            .filter(|(i, _)| !old_used.contains(i))
            .map(|(_, op)| Pair {
                before: Some(*op),
                after: None,
            }),
    );
    result.extend(
        new.iter()
            .enumerate()
            .filter(|(i, _)| !new_used.contains(i))
            .map(|(_, op)| Pair {
                before: None,
                after: Some(*op),
            }),
    );
    result.sort_by(|a, b| {
        (a.before.map(|o| o.source()), a.after.map(|o| o.source()))
            .cmp(&(b.before.map(|o| o.source()), b.after.map(|o| o.source())))
    });
    result
}

fn ids(operations: impl Iterator<Item = SourceId>) -> Vec<SourceId> {
    operations.collect::<BTreeSet<_>>().into_iter().collect()
}

pub(super) fn select_one(
    contract: &Contract,
    selected: &[String],
) -> Result<Vec<SourceId>, CompatibilityError> {
    if selected.is_empty() {
        return Ok(ids(contract.operations().map(|op| op.source().clone())));
    }
    let mut found = Vec::new();
    for id in selected.iter().collect::<BTreeSet<_>>() {
        let operations: Vec<_> = contract
            .operations()
            .filter(|op| op.operation_id() == Some(id.as_str()))
            .collect();
        if operations.len() != 1 {
            return Err(CompatibilityError(format!(
                "operationId {id:?} must identify exactly one operation in this snapshot"
            )));
        }
        found.push(operations[0].source().clone());
    }
    Ok(ids(found.into_iter()))
}

pub(super) fn select_pair(
    old: &Contract,
    new: &Contract,
    selected: &[String],
) -> Result<(Vec<SourceId>, Vec<SourceId>), CompatibilityError> {
    let old_ops: Vec<_> = old.operations().collect();
    let new_ops: Vec<_> = new.operations().collect();
    if selected.is_empty() {
        return Ok((
            ids(old_ops.iter().map(|o| o.source().clone())),
            ids(new_ops.iter().map(|o| o.source().clone())),
        ));
    }
    let wanted: BTreeSet<_> = selected.iter().map(String::as_str).collect();
    for id in &wanted {
        let left = old_ops
            .iter()
            .filter(|o| o.operation_id() == Some(*id))
            .count();
        let right = new_ops
            .iter()
            .filter(|o| o.operation_id() == Some(*id))
            .count();
        if left > 1 || right > 1 {
            return Err(CompatibilityError(format!(
                "operationId {id:?} is ambiguous"
            )));
        }
        if left + right == 0 {
            return Err(CompatibilityError(format!(
                "operationId {id:?} is absent from both contracts"
            )));
        }
    }
    let pairs: Vec<_> = pair(&old_ops, &new_ops)
        .into_iter()
        .filter(|p| {
            p.before
                .and_then(|o| o.operation_id())
                .is_some_and(|id| wanted.contains(id))
                || p.after
                    .and_then(|o| o.operation_id())
                    .is_some_and(|id| wanted.contains(id))
        })
        .collect();
    Ok((
        ids(pairs
            .iter()
            .filter_map(|p| p.before.map(|o| o.source().clone()))),
        ids(pairs
            .iter()
            .filter_map(|p| p.after.map(|o| o.source().clone()))),
    ))
}

pub(super) fn compare(
    old: &Contract,
    new: &Contract,
    old_selected: &[SourceId],
    new_selected: &[SourceId],
) -> WireResult {
    let left: Vec<_> = old
        .operations()
        .filter(|o| old_selected.contains(o.source()))
        .collect();
    let right: Vec<_> = new
        .operations()
        .filter(|o| new_selected.contains(o.source()))
        .collect();
    let pairs = pair(&left, &right);
    let mut result = WireResult {
        changes: Vec::new(),
        matches: pairs
            .iter()
            .map(|p| OperationMatch {
                before: p.before.map(|o| o.source().clone()),
                after: p.after.map(|o| o.source().clone()),
            })
            .collect(),
        schemas: BTreeMap::new(),
    };
    for (contract, operations, before) in [(old, &left, true), (new, &right, false)] {
        diagnostics(contract, operations, before, &mut result.changes);
    }
    if old.openapi_version() != new.openapi_version() {
        let mut finding = change(
            "wire-openapi-version-changed",
            Impact::Unknown,
            "OpenAPI version",
            "OpenAPI semantics changed; cross-version transport equivalence is not established.",
        );
        let old_id = SourceId::new(old.entry().clone(), Default::default()).child("openapi");
        let new_id = SourceId::new(new.entry().clone(), Default::default()).child("openapi");
        finding.source_before = Some(Location::at(old, &old_id));
        finding.source_after = Some(Location::at(new, &new_id));
        finding.before = Some(json!(old.openapi_version()));
        finding.after = Some(json!(new.openapi_version()));
        finding.migration =
            "Review the changed dialect/version rules and rerun the target conformance gates."
                .into();
        result.changes.push(finding);
    }
    for pair in pairs {
        let mut context = Context {
            old,
            new,
            pair,
            changes: &mut result.changes,
            schemas: &mut result.schemas,
        };
        match (pair.before, pair.after) {
            (Some(left), Some(right)) => context.operation(left, right),
            (Some(left), None) => context.push(
                "wire-operation-removed",
                Impact::Breaking,
                "operation",
                None,
                Some(left.source()),
                None,
                Some(operation_value(left)),
                None,
                "The selected endpoint was removed.",
                "Replace calls to this endpoint or retain a compatible server route.",
            ),
            (None, Some(right)) => context.push(
                "wire-operation-added",
                Impact::Compatible,
                "operation",
                None,
                None,
                Some(right.source()),
                None,
                Some(operation_value(right)),
                "A selected endpoint was added.",
                "Existing wire calls need no change.",
            ),
            _ => {}
        }
    }
    result
}

fn operation_value(op: Operation<'_>) -> Value {
    json!({"operationId":op.operation_id(),"method":op.method().as_str(),"path":op.path_template()})
}

struct Context<'a, 'b, 'c> {
    old: &'a Contract,
    new: &'b Contract,
    pair: Pair<'a, 'b>,
    changes: &'c mut Vec<Change>,
    schemas: &'c mut schema::Correspondence,
}

impl Context<'_, '_, '_> {
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        code: &str,
        impact: Impact,
        subject: &str,
        direction: Option<Direction>,
        old: Option<&SourceId>,
        new: Option<&SourceId>,
        before: Option<Value>,
        after: Option<Value>,
        message: &str,
        migration: &str,
    ) {
        let mut finding = change(code, impact, subject, message);
        finding.operation_id_before = self
            .pair
            .before
            .and_then(|o| o.operation_id())
            .map(str::to_owned);
        finding.operation_id_after = self
            .pair
            .after
            .and_then(|o| o.operation_id())
            .map(str::to_owned);
        finding.direction = direction;
        finding.source_before = old.map(|id| Location::at(self.old, id));
        finding.source_after = new.map(|id| Location::at(self.new, id));
        finding.before = before;
        finding.after = after;
        finding.migration = migration.into();
        self.changes.push(finding);
    }

    fn operation(&mut self, old: Operation<'_>, new: Operation<'_>) {
        for (code, subject, before, after) in [
            (
                "wire-method-changed",
                "HTTP method",
                old.method().as_str(),
                new.method().as_str(),
            ),
            (
                "wire-path-changed",
                "HTTP path",
                old.path_template().unwrap_or(""),
                new.path_template().unwrap_or(""),
            ),
        ] {
            if before != after {
                self.push(code, Impact::Breaking, subject, None, Some(old.source()), Some(new.source()), Some(json!(before)), Some(json!(after)),
                    "Existing requests address a different endpoint.", "Migrate callers and routing, or retain the previous method/path as a compatible alias.");
            }
        }
        if old.operation_id() != new.operation_id() {
            self.push("wire-operation-id-renamed", Impact::Compatible, "operationId", None,
                Some(&old.source().child("operationId")), Some(&new.source().child("operationId")),
                old.operation_id().map(|id| json!(id)), new.operation_id().map(|id| json!(id)),
                "A unique unchanged method/path identifies an operationId rename; operationId is not sent on the wire.",
                "Consult each native target's renamed operation and related types.");
        }
        let (before, after) = (servers(self.old, old), servers(self.new, new));
        if before != after {
            self.push(
                "wire-servers-changed",
                Impact::PotentiallyBreaking,
                "effective servers",
                None,
                old.server_source().or(Some(old.source())),
                new.server_source().or(Some(new.source())),
                Some(before),
                Some(after),
                "Effective server URLs, physical document-relative resolution, ordering or substitutions changed.",
                "Review deployment endpoints and explicit server overrides.",
            );
        }
        let (before, after) = (security(self.old, old), security(self.new, new));
        if before != after {
            let compatible = security_includes(&before, &after);
            self.push("wire-security-changed", if compatible { Impact::Compatible } else { Impact::PotentiallyBreaking }, "effective security", Some(Direction::Request),
                old.security_source().or(Some(old.source())), new.security_source().or(Some(new.source())), Some(before), Some(after),
                if compatible { "The new OR/AND security requirements admit every old alternative." } else { "Security schemes, required scopes or OR/AND alternatives changed; old credentials may no longer suffice." },
                "Review credential configuration and any new scopes. No authorization flow is inferred.");
        }
        self.parameters(old, new);
        self.body(old, new);
        self.responses(old, new);
        let remainder = |op: Operation<'_>| {
            without(
                op.raw(),
                &[
                    "operationId",
                    "summary",
                    "description",
                    "tags",
                    "deprecated",
                    "externalDocs",
                    "parameters",
                    "requestBody",
                    "responses",
                    "servers",
                    "security",
                ],
            )
        };
        let (before, after) = (remainder(old), remainder(new));
        if before != after {
            self.push("wire-operation-metadata-unknown", Impact::Unknown, "other operation metadata", None,
                Some(old.source()), Some(new.source()), Some(before), Some(after),
                "An operation keyword outside the HTTP comparison profile changed.", "Review callbacks, extensions and other changed metadata using the original source.");
        }
    }

    fn parameters(&mut self, old: Operation<'_>, new: Operation<'_>) {
        let left: BTreeMap<_, _> = old
            .parameters()
            .into_iter()
            .map(|p| (parameter_key(&p), p))
            .collect();
        let right: BTreeMap<_, _> = new
            .parameters()
            .into_iter()
            .map(|p| (parameter_key(&p), p))
            .collect();
        for key in left.keys().chain(right.keys()).collect::<BTreeSet<_>>() {
            let subject = format!("parameter {key}");
            match (left.get(key), right.get(key)) {
                (Some(l), Some(r)) => {
                    let (ls, rs) = (l.resolved_source().unwrap_or_else(|| l.source().clone()), r.resolved_source().unwrap_or_else(|| r.source().clone()));
                    self.presence(&subject, Direction::Request, l.required().unwrap_or(false), r.required().unwrap_or(false), &field_source(self.old, &ls, "required"), &field_source(self.new, &rs, "required"));
                    let transport = |p: &Parameter<'_>| json!({"style":p.effective_style().map(|s|format!("{s:?}")),"explode":p.effective_explode(),"allowReserved":p.allow_reserved().unwrap_or(false),"allowEmptyValue":p.allow_empty_value().unwrap_or(false)});
                    let (before, after) = (transport(l), transport(r));
                    if before != after {
                        self.push("wire-parameter-serialization-changed", Impact::PotentiallyBreaking, &subject, Some(Direction::Request), Some(&ls), Some(&rs), Some(before), Some(after),
                            "Parameter serialization or empty/reserved-value handling changed.", "Update serializers and verify exact URL/header/cookie bytes.");
                    }
                    self.schema(&subject, Direction::Request, l.schema().map(|s| s.id().clone()), r.schema().map(|s| s.id().clone()), &ls, &rs);
                    self.content(&subject, Direction::Request, l.content(), r.content());
                },
                (Some(l), None) => self.push("wire-parameter-removed", Impact::PotentiallyBreaking, &subject, Some(Direction::Request), Some(l.source()), None, Some(json!({"name":l.name(),"required":l.required().unwrap_or(false)})), None,
                    "A previously declared parameter was removed; behavior for old callers still sending it is unspecified.", "Remove or replace the parameter after checking server behavior."),
                (None, Some(r)) => self.push("wire-parameter-added", if r.required().unwrap_or(false) { Impact::Breaking } else { Impact::Compatible }, &subject, Some(Direction::Request), None, Some(r.source()), None, Some(json!({"name":r.name(),"required":r.required().unwrap_or(false)})),
                    if r.required().unwrap_or(false) { "Existing requests omit a newly required parameter." } else { "An optional parameter was added." }, "Supply newly required inputs; consult native input-member changes."),
                _ => {},
            }
        }
    }

    fn body(&mut self, old: Operation<'_>, new: Operation<'_>) {
        match (old.request_body(), new.request_body()) {
            (Some(l), Some(r)) => {
                let (ls, rs) = (l.resolved_source().unwrap_or_else(|| l.source().clone()), r.resolved_source().unwrap_or_else(|| r.source().clone()));
                self.presence("request body", Direction::Request, l.required().unwrap_or(false), r.required().unwrap_or(false), &field_source(self.old, &ls, "required"), &field_source(self.new, &rs, "required"));
                self.content("request body", Direction::Request, l.content(), r.content());
            },
            (Some(l), None) => self.push("wire-request-body-removed", Impact::PotentiallyBreaking, "request body", Some(Direction::Request), Some(l.source()), None, Some(l.raw().clone()), None,
                "The request payload binding was removed; acceptance of old body values is unspecified.", "Review server behavior before removing or replacing payloads."),
            (None, Some(r)) => self.push("wire-request-body-added", if r.required().unwrap_or(false) { Impact::Breaking } else { Impact::Compatible }, "request body", Some(Direction::Request), None, Some(r.source()), None, Some(r.raw().clone()),
                if r.required().unwrap_or(false) { "Existing body-less requests no longer meet requiredness." } else { "An optional request body was added." }, "Supply any newly required body using the target's planned model/codec."),
            _ => {},
        }
    }

    fn presence(
        &mut self,
        subject: &str,
        direction: Direction,
        old: bool,
        new: bool,
        old_source: &SourceId,
        new_source: &SourceId,
    ) {
        if old == new {
            return;
        }
        let compatible = match direction {
            Direction::Request => !new,
            Direction::Response => new,
        };
        self.push("wire-requiredness-changed", if compatible { Impact::Compatible } else { Impact::Breaking }, subject, Some(direction), Some(old_source), Some(new_source), Some(json!(old)), Some(json!(new)),
            "Required presence changed independently of nullability.", if compatible { "Review the separate generated API changes." } else { "Handle missing output values or supply newly required request values as appropriate." });
    }

    fn schema(
        &mut self,
        subject: &str,
        direction: Direction,
        old: Option<SchemaId>,
        new: Option<SchemaId>,
        old_container: &SourceId,
        new_container: &SourceId,
    ) {
        match (old, new) {
            (Some(l), Some(r)) => {
                let assessment =
                    schema::assess(self.old, &l, self.new, &r, direction, self.schemas);
                if !assessment.changed {
                    return;
                }
                self.push("wire-schema-changed", assessment.impact, subject, Some(direction), Some(&l), Some(&r), self.old.source(&l).cloned(), self.new.source(&r).cloned(),
                    "The resolved value contract changed.", "Review presence, null, literals, unions and constraints in the source deltas; regenerate codecs and verify affected values.");
                let finding = self.changes.last_mut().expect("just inserted");
                finding.reasoning = assessment.reasoning;
                finding.schema_deltas = assessment.deltas;
            }
            (None, None) => {}
            (old, new) => {
                let compatible = match direction {
                    Direction::Request => new.is_none(),
                    Direction::Response => old.is_none(),
                };
                self.push("wire-schema-binding-changed", if compatible { Impact::Compatible } else { Impact::PotentiallyBreaking }, subject, Some(direction), old.as_ref().or(Some(old_container)), new.as_ref().or(Some(new_container)), old.as_ref().and_then(|id| self.old.source(id)).cloned(), new.as_ref().and_then(|id| self.new.source(id)).cloned(),
                    "A schema binding was added or removed. An absent schema supplies no value-domain guarantee.", "Check callers against the newly specified or unspecified value domain.");
            }
        }
    }

    fn content(
        &mut self,
        subject: &str,
        direction: Direction,
        old: Vec<MediaType<'_>>,
        new: Vec<MediaType<'_>>,
    ) {
        if direction == Direction::Response && old.is_empty() != new.is_empty() {
            self.push("wire-response-content-presence-changed", Impact::PotentiallyBreaking, subject, Some(direction),
                self.pair.before.map(|op| op.source()), self.pair.after.map(|op| op.source()),
                Some(json!(old.iter().map(|m|m.name()).collect::<Vec<_>>())), Some(json!(new.iter().map(|m|m.name()).collect::<Vec<_>>())),
                "The response changed between declared content and no declared content; this is not merely removal of a possible media alternative.",
                "Check body-less responses and old decoder expectations explicitly.");
        }
        let left: BTreeMap<_, _> = old.into_iter().map(|m| (m.name().to_owned(), m)).collect();
        let right: BTreeMap<_, _> = new.into_iter().map(|m| (m.name().to_owned(), m)).collect();
        for name in left.keys().chain(right.keys()).collect::<BTreeSet<_>>() {
            let subject = format!("{subject}, {name}");
            match (left.get(name), right.get(name)) {
                (Some(l), Some(r)) => {
                    self.schema(
                        &subject,
                        direction,
                        l.schema().map(|s| s.id().clone()),
                        r.schema().map(|s| s.id().clone()),
                        l.source(),
                        r.source(),
                    );
                    self.schema(
                        &format!("{subject}, stream item"),
                        direction,
                        l.item_schema().map(|s| s.id().clone()),
                        r.item_schema().map(|s| s.id().clone()),
                        l.source(),
                        r.source(),
                    );
                    let (before, after) =
                        (media_metadata(self.old, l), media_metadata(self.new, r));
                    let encoded = |contract: &Contract, media: &MediaType<'_>| {
                        let raw = media_value(contract, media);
                        !media.encoding().is_empty()
                            || raw.get("prefixEncoding").is_some()
                            || raw.get("itemEncoding").is_some()
                    };
                    if before != after || encoded(self.old, l) || encoded(self.new, r) {
                        self.push("wire-media-metadata-unknown", Impact::Unknown, &subject, Some(direction), Some(l.source()), Some(r.source()), Some(before), Some(after),
                            "Encoding or other media metadata needs a protocol-specific comparison, including referenced encoding headers.", "Verify multipart/form/item encodings and their source-linked declarations.");
                    }
                }
                (l, r) => {
                    let added = l.is_none();
                    let compatible = match direction {
                        Direction::Request => added,
                        Direction::Response => !added,
                    };
                    let wildcard = name.contains('*')
                        || left.keys().chain(right.keys()).any(|key| key.contains('*'));
                    self.push(if added { "wire-media-type-added" } else { "wire-media-type-removed" }, if wildcard { Impact::Unknown } else if compatible { Impact::Compatible } else { Impact::PotentiallyBreaking }, &subject, Some(direction), l.map(|m| m.source()), r.map(|m| m.source()), l.map(|m| m.raw().clone()), r.map(|m| m.raw().clone()),
                        "Declared request/response media coverage changed.", "Review content negotiation and decoders; wildcard precedence needs a separate proof.");
                }
            }
        }
    }

    fn responses(&mut self, old: Operation<'_>, new: Operation<'_>) {
        let left: BTreeMap<_, _> = old
            .responses()
            .into_iter()
            .map(|r| (r.status_key().to_owned(), r))
            .collect();
        let right: BTreeMap<_, _> = new
            .responses()
            .into_iter()
            .map(|r| (r.status_key().to_owned(), r))
            .collect();
        // Exact > range > default, over the finite HTTP status domain. This
        // avoids treating an exact status covered by an old default as novel.
        let mut groups: BTreeMap<(Option<String>, Option<String>), Vec<u16>> = BTreeMap::new();
        for status in 100..600 {
            let key = (response_key(&left, status), response_key(&right, status));
            if key != (None, None) {
                groups.entry(key).or_default().push(status);
            }
        }
        for ((l, r), statuses) in groups {
            let subject = if statuses.len() == 1 {
                format!("response {}", statuses[0])
            } else {
                format!(
                    "responses {}…{} ({} statuses)",
                    statuses[0],
                    statuses.last().unwrap(),
                    statuses.len()
                )
            };
            match (l.as_ref().and_then(|key| left.get(key)), r.as_ref().and_then(|key| right.get(key))) {
                (Some(l), Some(r)) => {
                    self.content(&subject, Direction::Response, l.content(), r.content());
                    self.headers(&subject, l.headers(), r.headers());
                    if !l.links().is_empty() || !r.links().is_empty() {
                        let links = |response: &Response<'_>| json!(response.links().iter().map(|link| json!({"name":link.name(),"operationId":link.operation_id(),"operationRef":link.operation_ref(),"parameters":link.parameters(),"requestBody":link.request_body()})).collect::<Vec<_>>());
                        let (before, after) = (links(l), links(r));
                        if before != after {
                            self.push("wire-response-links-changed", Impact::Unknown, &subject, Some(Direction::Response), Some(l.source()), Some(r.source()), Some(before), Some(after), "Response link metadata changed.", "Review runtime expressions and linked operations.");
                        }
                    }
                },
                (l, r) => self.push(if l.is_none() { "wire-response-added" } else { "wire-response-removed" }, if l.is_none() { Impact::PotentiallyBreaking } else { Impact::Compatible }, &subject, Some(Direction::Response), l.map(|r| r.source()), r.map(|r| r.source()), l.map(|_| json!({"statuses":statuses})), r.map(|_| json!({"statuses":statuses})),
                    if l.is_none() { "New response statuses may escape the old consumer's declared response domain." } else { "Removing possible response statuses narrows the wire output domain; native exported variants can still break source consumers." },
                    "Review status/error handling and the separate per-language response types."),
            }
        }
    }

    fn headers(&mut self, subject: &str, old: Vec<Header<'_>>, new: Vec<Header<'_>>) {
        let left: BTreeMap<_, _> = old
            .into_iter()
            .map(|h| (h.name().to_ascii_lowercase(), h))
            .collect();
        let right: BTreeMap<_, _> = new
            .into_iter()
            .map(|h| (h.name().to_ascii_lowercase(), h))
            .collect();
        for key in left.keys().chain(right.keys()).collect::<BTreeSet<_>>() {
            let subject = format!("{subject}, header {key}");
            match (left.get(key), right.get(key)) {
                (Some(l), Some(r)) => {
                    self.presence(
                        &subject,
                        Direction::Response,
                        l.required().unwrap_or(false),
                        r.required().unwrap_or(false),
                        l.source(),
                        r.source(),
                    );
                    self.schema(
                        &subject,
                        Direction::Response,
                        l.schema().map(|s| s.id().clone()),
                        r.schema().map(|s| s.id().clone()),
                        l.source(),
                        r.source(),
                    );
                    self.content(&subject, Direction::Response, l.content(), r.content());
                    if l.effective_style() != r.effective_style()
                        || l.effective_explode() != r.effective_explode()
                    {
                        self.push(
                            "wire-header-serialization-changed",
                            Impact::PotentiallyBreaking,
                            &subject,
                            Some(Direction::Response),
                            Some(l.source()),
                            Some(r.source()),
                            Some(l.raw().clone()),
                            Some(r.raw().clone()),
                            "Response header serialization changed.",
                            "Update header decoding and verify exact wire values.",
                        );
                    }
                }
                (l, r) => self.push(
                    "wire-response-header-binding-changed",
                    if l.is_some() {
                        Impact::PotentiallyBreaking
                    } else {
                        Impact::Compatible
                    },
                    &subject,
                    Some(Direction::Response),
                    l.map(|h| h.source()),
                    r.map(|h| h.source()),
                    l.map(|h| h.raw().clone()),
                    r.map(|h| h.raw().clone()),
                    "A header value/presence guarantee was added or removed.",
                    "Review accesses to the affected response header.",
                ),
            }
        }
    }
}

fn parameter_key(parameter: &Parameter<'_>) -> String {
    let name = parameter.name().unwrap_or("<invalid>");
    let name = if parameter.location() == Some(ParameterLocation::Header) {
        name.to_ascii_lowercase()
    } else {
        name.to_owned()
    };
    format!("{:?}:{name}", parameter.location())
}

fn media_value<'a>(contract: &'a Contract, media: &MediaType<'a>) -> &'a Value {
    media
        .resolved_source()
        .and_then(|source| contract.source(&source))
        .unwrap_or_else(|| media.raw())
}

fn media_metadata(contract: &Contract, media: &MediaType<'_>) -> Value {
    // Only the context-aware typed view can consume itemSchema. An older
    // referenced document's unsupported field must remain visible as metadata.
    let omitted = if media.item_schema().is_some() {
        &["schema", "itemSchema", "example", "examples"][..]
    } else {
        &["schema", "example", "examples"][..]
    };
    without(media_value(contract, media), omitted)
}

fn reference_sources(contract: &Contract, source: &SourceId) -> BTreeSet<SourceId> {
    let mut sources = BTreeSet::new();
    let mut current = source;
    while sources.insert(current.clone()) {
        let Some(target) = contract.reference_target(current) else {
            break;
        };
        current = target;
    }
    sources
}

fn media_sources(
    contract: &Contract,
    media: Vec<MediaType<'_>>,
    relevant: &mut Vec<SourceId>,
    roots: &mut Vec<SchemaId>,
) {
    for media in media {
        relevant.extend(reference_sources(contract, media.source()));
        relevant.extend(media.resolved_source());
        roots.extend(media.schema_roots());
    }
}

fn servers(contract: &Contract, operation: Operation<'_>) -> Value {
    Value::Array(operation.effective_servers().iter().map(|server| {
        let url = server.url();
        let document = server.source().or(operation.server_source())
            .map_or(contract.entry(), SourceId::document);
        // A literal scheme makes the template independent of the retrieval
        // address even if its authority/path contains substitution variables.
        let absolute = url.and_then(|url| url.split_once(':')).is_some_and(|(scheme, _)| {
            scheme.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        });
        let relative_resolution = if absolute {
            Value::Null
        } else if url::Url::parse(document.as_str()).is_ok_and(|url|
            matches!(url.scheme(), "http" | "https") && url.has_host()) {
            match url.filter(|url| !url.contains(['{', '}']))
                .and_then(|url| suspect_ir::contract::resource_uri::resolve_document(document.as_str(), url).ok()) {
                Some(resolved) => json!({"resolved":resolved}),
                // Template-dependent bases cannot be erased as source prose.
                // Comparing their literal bases is deliberately conservative.
                None => json!({"documentBase":document.as_str()}),
            }
        } else {
            json!({"requiresHttpDocumentUrl":true})
        };
        json!({"url":url,"relativeResolution":relative_resolution,
            "variables":server.variables().iter().map(|(name,v)| json!({"name":name,"default":v.default(),"enum":v.values()})).collect::<Vec<_>>()})
    }).collect())
}

fn field_source(contract: &Contract, object: &SourceId, field: &str) -> SourceId {
    let source = object.child(field);
    if contract.source(&source).is_some() {
        source
    } else {
        object.clone()
    }
}

fn response_key(responses: &BTreeMap<String, Response<'_>>, status: u16) -> Option<String> {
    let exact = status.to_string();
    let range = format!("{}XX", status / 100);
    [exact.as_str(), range.as_str(), "default"]
        .into_iter()
        .find(|key| {
            responses.get(*key).is_some_and(|r| {
                matches!(
                    r.status(),
                    Some(
                        ResponseStatus::Exact(_)
                            | ResponseStatus::Range(_)
                            | ResponseStatus::Default
                    )
                )
            })
        })
        .map(str::to_owned)
}

fn without(raw: &Value, omitted: &[&str]) -> Value {
    match raw.as_object() {
        Some(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| !omitted.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        None => raw.clone(),
    }
}

/// Resolved scheme definitions and sorted scope sets, retaining AND/OR shape.
fn security(contract: &Contract, op: Operation<'_>) -> Value {
    let mut alternatives: Vec<Value> = op
        .effective_security()
        .iter()
        .map(|requirement| {
            Value::Object(
                requirement
                    .requirements()
                    .iter()
                    .map(|usage| {
                        let mut scopes = usage.scopes().unwrap_or_default();
                        scopes.sort();
                        scopes.dedup();
                        let definition = usage.scheme().map(|scheme| {
                            let resolved =
                                scheme.resolved_source().and_then(|id| contract.source(&id));
                            let mut value = without(
                                resolved.unwrap_or_else(|| scheme.raw()),
                                &["description", "summary", "bearerFormat"],
                            );
                            if let Some(flows) =
                                value.get_mut("flows").and_then(Value::as_object_mut)
                            {
                                for flow in flows.values_mut() {
                                    if let Some(scopes) =
                                        flow.get_mut("scopes").and_then(Value::as_object_mut)
                                    {
                                        for description in scopes.values_mut() {
                                            *description = Value::Null;
                                        }
                                    }
                                }
                            }
                            value
                        });
                        (
                            usage.name().into(),
                            json!({"scopes":scopes,"definition":definition}),
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    if alternatives.is_empty() {
        alternatives.push(json!({}));
    }
    alternatives.sort_by_cached_key(|v| serde_json::to_string(v).expect("security JSON"));
    alternatives.dedup();
    Value::Array(alternatives)
}

fn security_includes(old: &Value, new: &Value) -> bool {
    old.as_array().into_iter().flatten().all(|left| {
        new.as_array().into_iter().flatten().any(|right| {
            right.as_object().is_some_and(|right| {
                right.iter().all(|(name, requirement)| {
                    left.get(name).is_some_and(|previous| {
                        previous["definition"] == requirement["definition"]
                            && requirement["definition"] != Value::Null
                            && requirement["scopes"].as_array().is_some_and(|scopes| {
                                scopes.iter().all(|scope| {
                                    previous["scopes"]
                                        .as_array()
                                        .is_some_and(|values| values.contains(scope))
                                })
                            })
                    })
                })
            })
        })
    })
}

fn diagnostics(
    contract: &Contract,
    operations: &[Operation<'_>],
    before: bool,
    changes: &mut Vec<Change>,
) {
    let mut relevant = Vec::new();
    let mut path_items = Vec::new();
    let mut roots = Vec::new();
    for op in operations {
        relevant.push(op.source().clone());
        path_items.push(op.path_item_source().clone());
        relevant.extend(op.server_source().cloned());
        relevant.extend(op.security_source().cloned());
        for usage in op
            .effective_security()
            .iter()
            .flat_map(|r| r.requirements())
        {
            if let Some(scheme) = usage.scheme() {
                relevant.extend(reference_sources(contract, scheme.source()));
                relevant.extend(scheme.resolved_source());
            }
        }
        for parameter in op.parameters() {
            relevant.extend(reference_sources(contract, parameter.source()));
            relevant.extend(parameter.resolved_source());
            roots.extend(parameter.schema().map(|s| s.id().clone()));
            media_sources(contract, parameter.content(), &mut relevant, &mut roots);
        }
        if let Some(body) = op.request_body() {
            relevant.extend(reference_sources(contract, body.source()));
            relevant.extend(body.resolved_source());
            media_sources(contract, body.content(), &mut relevant, &mut roots);
        }
        for response in op.responses() {
            relevant.extend(reference_sources(contract, response.source()));
            relevant.extend(response.resolved_source());
            media_sources(contract, response.content(), &mut relevant, &mut roots);
            for header in response.headers() {
                relevant.extend(reference_sources(contract, header.source()));
                relevant.extend(header.resolved_source());
                roots.extend(header.schema().map(|s| s.id().clone()));
                media_sources(contract, header.content(), &mut relevant, &mut roots);
            }
        }
    }
    let closure = contract.effective_schema_closure(&roots);
    let within = |a: &SourceId, b: &SourceId| {
        a.document() == b.document()
            && (a.pointer() == b.pointer()
                || a.pointer()
                    .strip_prefix(b.pointer())
                    .is_some_and(|tail| tail.starts_with('/')))
    };
    for diagnostic in contract.diagnostics().iter().filter(|d| {
        if d.severity != ContractSeverity::Error {
            return false;
        }
        if contract.schema(&d.source).is_some() {
            // An address may also be an explicit HTTP object. Treat that exact
            // address as an additional use, while still applying Contract's
            // ignored-keyword rules: a 3.0 Reference Object can inhabit both
            // roles without activating any of its schema siblings.
            return contract.schema_diagnostic_applies(&closure, d)
                || relevant.contains(&d.source)
                    && contract.schema_diagnostic_applies(std::slice::from_ref(&d.source), d);
        }
        contract.schema_diagnostic_applies(&closure, d)
            || path_items
                .iter()
                .any(|source| d.source == *source || d.source == source.child("$ref"))
            || relevant
                .iter()
                .any(|source| within(&d.source, source) || within(source, &d.source))
    }) {
        let mut finding = change(
            "wire-contract-diagnostic",
            Impact::Unknown,
            diagnostic.code,
            &diagnostic.message,
        );
        let source = Some(Location::at(contract, &diagnostic.source));
        if before {
            finding.source_before = source;
        } else {
            finding.source_after = source;
        }
        finding.migration =
            "Resolve the original contract diagnostic before relying on compatibility proofs."
                .into();
        changes.push(finding);
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for id in operations.iter().filter_map(|o| o.operation_id()) {
        *counts.entry(id).or_default() += 1;
    }
    for (id, count) in counts.into_iter().filter(|(_, count)| *count > 1) {
        let mut finding = change(
            "wire-operation-identity-ambiguous",
            Impact::Unknown,
            id,
            format!(
                "{count} selected operations share this operationId; source identity is ambiguous."
            ),
        );
        finding.migration =
            "Assign unique operationIds before comparing the selected SDK surface.".into();
        changes.push(finding);
    }
}
