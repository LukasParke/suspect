//! Shared, bounded example planning. Candidates become examples only after the
//! owned schema validator accepts them. Native backends reuse values and origins.

use crate::http_contract;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    io::{self, Write},
    ops::Range,
    sync::Arc,
};
use suspect_ir::contract::{Contract, Example, ParameterLocation, SchemaId, SourceId};
use suspect_schema::{Config, OwnedCompileErrorKind, OwnedCompiler, OwnedOutcome, OwnedSchema};

#[cfg(feature = "http-protocol")]
mod aggregate;
#[cfg(feature = "http-protocol")]
mod protocol;
#[cfg(feature = "http-protocol")]
pub use protocol::{plan_protocol_examples, plan_protocol_examples_v2, plan_protocol_examples_v3};

/// Finite example discovery, synthesis and validation policy.
#[derive(Debug, Clone)]
pub struct ExampleConfig {
    pub max_depth: usize,
    pub max_candidates: usize,
    /// Plan-wide schema visits, validation attempts and copied JSON bytes.
    pub max_work: usize,
    pub max_validation_steps: usize,
    pub max_string_length: usize,
    pub max_declared: usize,
}
impl Default for ExampleConfig {
    fn default() -> Self {
        Self {
            max_depth: 32,
            max_candidates: 8,
            max_work: 100_000,
            max_validation_steps: 100_000,
            max_string_length: 4096,
            max_declared: 32,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExampleOrigin {
    Declared,
    Synthesized,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExampleRole {
    RequestBody,
    Response {
        status: u16,
    },
    /// A declared status pattern, without inventing a concrete response code.
    ResponsePattern {
        status: String,
    },
    ResponseHeader {
        status: String,
        wire_name: String,
    },
    RequestPart {
        name: Option<String>,
        repeated: bool,
    },
    RequestPartHeader {
        part: Option<String>,
        wire_name: String,
    },
    ResponsePart {
        status: String,
        name: Option<String>,
        repeated: bool,
    },
    ResponsePartHeader {
        status: String,
        part: Option<String>,
        wire_name: String,
    },
    RequestItem,
    ResponseItem {
        status: String,
    },
    Parameter {
        wire_name: String,
        location: ParameterLocation,
    },
}
impl ExampleRole {
    pub fn is_response(&self) -> bool {
        matches!(
            self,
            Self::Response { .. }
                | Self::ResponsePattern { .. }
                | Self::ResponseHeader { .. }
                | Self::ResponseItem { .. }
                | Self::ResponsePart { .. }
                | Self::ResponsePartHeader { .. }
        )
    }
}

/// A positional wire occurrence, independent of a reusable item schema's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExamplePartPosition {
    Prefix(usize),
    Items,
}

/// A schema-valid value. Source prose and names are inert metadata.
#[derive(Debug, Clone)]
pub struct ExampleEntry {
    pub role: ExampleRole,
    pub media_type: String,
    pub schema: SchemaId,
    /// Real wire-slot container (parameter or media object), even through refs.
    pub container: SourceId,
    /// Positional prefixes and the remaining-item group can share a codec schema
    /// while requiring different encodings and independently absent values.
    pub part_position: Option<ExamplePartPosition>,
    pub origin: ExampleOrigin,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub declared_source: Option<SourceId>,
    pub value: Value,
}
#[derive(Debug, Clone)]
pub struct ExampleDiagnostic {
    pub source: SourceId,
    pub at: Range<usize>,
    pub code: &'static str,
    pub message: String,
}
#[derive(Debug)]
pub struct OperationExamples {
    pub source: SourceId,
    pub operation_id: String,
    pub entries: Vec<ExampleEntry>,
    /// Source-declared complete JSON aggregates, kept separate from the actual
    /// part codec inputs in `entries`. Byte aggregates remain unavailable.
    pub validated_aggregates: Vec<ExampleEntry>,
}
#[derive(Debug)]
pub struct ExamplePlan {
    contract: Arc<Contract>,
    operations: Vec<OperationExamples>,
    diagnostics: Vec<ExampleDiagnostic>,
    format: &'static str,
}
impl ExamplePlan {
    pub fn format(&self) -> &'static str {
        self.format
    }
    #[must_use]
    pub fn operations(&self) -> &[OperationExamples] {
        &self.operations
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[ExampleDiagnostic] {
        &self.diagnostics
    }
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
}
fn diagnostic(
    contract: &Contract,
    source: &SourceId,
    code: &'static str,
    message: impl Into<String>,
) -> ExampleDiagnostic {
    ExampleDiagnostic {
        source: source.clone(),
        at: contract.source_span(source).unwrap_or(0..0),
        code,
        message: message.into(),
    }
}

fn configuration_error(contract: &Contract, config: &ExampleConfig) -> Option<ExampleDiagnostic> {
    let valid = config.max_depth > 0
        && config.max_depth <= 64
        && config.max_candidates > 0
        && config.max_candidates <= 64
        && config.max_work > 0
        && config.max_work <= 10_000_000
        && config.max_validation_steps > 0
        && config.max_validation_steps <= 1_000_000
        && config.max_string_length > 0
        && config.max_string_length <= 65_536
        && config.max_declared > 0
        && config.max_declared <= 128;
    (!valid).then(|| {
        diagnostic(
            contract,
            &SourceId::new(contract.entry().clone(), Default::default()),
            "examples-config",
            "example budgets must be positive and within the documented finite profile ceilings",
        )
    })
}

/// Prefer source-declared values, retaining invalid declarations as findings.
/// A separate synthesized value can be emitted when no declared value is valid.
/// This profile validates neutral schemas; active directional annotations yield
/// explicit example-unavailable findings rather than neutral validation guesses.
#[must_use]
pub fn plan_examples(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: ExampleConfig,
) -> ExamplePlan {
    let mut plan = ExamplePlan {
        contract: contract.clone(),
        operations: Vec::new(),
        diagnostics: Vec::new(),
        format: "suspect-sdk-examples-v1",
    };
    if let Some(error) = configuration_error(&contract, &config) {
        plan.diagnostics.push(error);
        return plan;
    }
    let wire = match http_contract::plan(&contract, selected) {
        Ok(wire) => wire,
        Err(errors) => {
            plan.diagnostics
                .extend(errors.into_iter().map(|d| ExampleDiagnostic {
                    source: d.source,
                    at: d.at,
                    code: d.code,
                    message: d.message,
                }));
            return plan;
        }
    };
    let roots = crate::schema_view::closure(&contract, &wire.roots);
    let validator = match OwnedCompiler::new(Config {
        format_assertion: false,
        max_evaluation_steps: config.max_validation_steps,
        ..Config::default()
    })
    .compile(contract.clone(), &roots)
    {
        Ok(validator) => validator,
        Err(errors) => {
            plan.diagnostics
                .extend(errors.into_iter().map(|d| ExampleDiagnostic {
                    source: d.source,
                    at: d.span.unwrap_or(0..0),
                    code: match d.kind {
                        OwnedCompileErrorKind::ResourceLimit => "examples-schema-limit",
                        OwnedCompileErrorKind::Unsupported => "examples-schema-unsupported",
                        _ => "examples-schema-invalid",
                    },
                    message: d.message,
                }));
            return plan;
        }
    };
    let mut state = State {
        contract: &contract,
        validator: &validator,
        config: &config,
        remaining: config.max_work,
    };
    for wire in &wire.operations {
        let operation = contract
            .operations()
            .find(|op| op.source() == &wire.source)
            .expect("admitted operation");
        let mut slots = Vec::new();
        if let Some(body) = &wire.body {
            let media = operation
                .request_body()
                .unwrap()
                .content()
                .into_iter()
                .find(|media| media.name() == body.media_type)
                .unwrap();
            slots.push(Slot {
                role: ExampleRole::RequestBody,
                media: body.media_type.clone(),
                schema: body.schema.clone(),
                container: body.media_source.clone(),
                part_position: None,
                definition: body.media_source.clone(),
                named: media.examples(),
                schema_only: false,
            });
        }
        for response in &wire.responses {
            let declared = operation
                .responses()
                .into_iter()
                .find(|r| r.status_key() == response.status.to_string())
                .unwrap();
            let media = declared
                .content()
                .into_iter()
                .find(|media| media.name() == response.media_type)
                .unwrap();
            slots.push(Slot {
                role: ExampleRole::Response {
                    status: response.status,
                },
                media: response.media_type.clone(),
                schema: response.schema.clone(),
                container: response.media_source.clone(),
                part_position: None,
                definition: response.media_source.clone(),
                named: media.examples(),
                schema_only: false,
            });
        }
        for parameter in &wire.parameters {
            let declared = operation
                .parameters()
                .into_iter()
                .find(|p| {
                    p.name() == Some(parameter.wire_name.as_str())
                        && p.location() == Some(parameter.location)
                })
                .unwrap();
            slots.push(Slot {
                role: ExampleRole::Parameter {
                    wire_name: parameter.wire_name.clone(),
                    location: parameter.location,
                },
                media: String::new(),
                schema: parameter.schema.clone(),
                container: parameter.source.clone(),
                part_position: None,
                definition: declared
                    .resolved_source()
                    .unwrap_or_else(|| parameter.source.clone()),
                named: declared.examples(),
                schema_only: false,
            });
        }
        let mut entries = Vec::new();
        for slot in slots {
            state.slot(slot, &mut entries, &mut plan.diagnostics);
        }
        plan.operations.push(OperationExamples {
            source: wire.source.clone(),
            operation_id: wire.operation_id.clone(),
            entries,
            validated_aggregates: Vec::new(),
        });
    }
    plan
}

struct Slot<'a> {
    role: ExampleRole,
    media: String,
    schema: SchemaId,
    container: SourceId,
    part_position: Option<ExamplePartPosition>,
    // A parameter's use site remains its binding identity; examples belong to
    // the terminal definition when that parameter is a Reference Object.
    definition: SourceId,
    named: Vec<Example<'a>>,
    // Item/part examples belong to their schema, not an aggregate media value.
    schema_only: bool,
}
struct Candidate<'a> {
    value: &'a Value,
    source: SourceId,
    name: Option<String>,
    summary: Option<String>,
}
enum Failure {
    Unsupported(String),
    Limit,
    Evaluation(SourceId, String),
}
struct State<'a> {
    contract: &'a Contract,
    validator: &'a OwnedSchema,
    config: &'a ExampleConfig,
    remaining: usize,
}

impl State<'_> {
    fn charge(&mut self, cost: usize) -> Result<(), Failure> {
        self.remaining = self.remaining.checked_sub(cost).ok_or(Failure::Limit)?;
        Ok(())
    }
    fn copy(&mut self, value: &Value) -> Result<Value, Failure> {
        let mut pending = vec![(value, 1)];
        while let Some((value, depth)) = pending.pop() {
            self.charge(1)?;
            if depth > self.config.max_depth {
                return Err(Failure::Limit);
            }
            match value {
                Value::Array(values) => {
                    if values.len().saturating_add(pending.len()) > self.remaining {
                        return Err(Failure::Limit);
                    }
                    pending.extend(values.iter().map(|value| (value, depth + 1)));
                }
                Value::Object(values) => {
                    if values.len().saturating_add(pending.len()) > self.remaining {
                        return Err(Failure::Limit);
                    }
                    pending.extend(values.values().map(|value| (value, depth + 1)));
                }
                _ => {}
            }
        }
        // A bounded counting sink checks cost before cloning a declared value.
        let mut counter = Counter {
            left: self.remaining,
            used: 0,
        };
        serde_json::to_writer(&mut counter, value).map_err(|_| Failure::Limit)?;
        self.charge(counter.used.saturating_add(1))?;
        Ok(value.clone())
    }
    fn outcome(&mut self, schema: &SchemaId, value: &Value) -> Result<OwnedOutcome, Failure> {
        self.charge(1)?;
        match self.validator.validate(schema, value) {
            OwnedOutcome::EvaluationFailure(finding) => {
                Err(Failure::Evaluation(finding.source, finding.message))
            }
            complete => Ok(complete),
        }
    }
    fn valid(&mut self, schema: &SchemaId, value: &Value) -> Result<bool, Failure> {
        Ok(matches!(self.outcome(schema, value)?, OwnedOutcome::Valid))
    }
    fn report(
        &self,
        source: &SourceId,
        failure: Failure,
        diagnostics: &mut Vec<ExampleDiagnostic>,
    ) {
        let (source, code, message) = match failure {
            Failure::Unsupported(message) => (source, "examples-synthesis-unsupported", message),
            Failure::Limit => (
                source,
                "examples-synthesis-limit",
                "example planning exhausted its finite work/depth/value budget".into(),
            ),
            Failure::Evaluation(ref at, ref message) => {
                diagnostics.push(diagnostic(
                    self.contract,
                    at,
                    "examples-evaluation-incomplete",
                    format!("example validity is unknown: {message}"),
                ));
                return;
            }
        };
        diagnostics.push(diagnostic(self.contract, source, code, message));
    }
    fn slot(
        &mut self,
        slot: Slot<'_>,
        entries: &mut Vec<ExampleEntry>,
        diagnostics: &mut Vec<ExampleDiagnostic>,
    ) {
        if !self.supported(&slot.schema, diagnostics) {
            return;
        }
        let declared =
            declared_candidates(self.contract, &slot, self.config.max_declared, diagnostics);
        let validated = self.declared_values(&slot, declared, diagnostics);
        if !validated.is_empty() {
            entries.extend(validated);
            return;
        }
        match self.first(&slot.schema, &mut Vec::new()) {
            Ok(value) => entries.push(ExampleEntry {
                role: slot.role,
                media_type: slot.media,
                schema: slot.schema,
                container: slot.container,
                part_position: slot.part_position,
                origin: ExampleOrigin::Synthesized,
                name: None,
                summary: None,
                declared_source: None,
                value,
            }),
            Err(error) => self.report(&slot.schema, error, diagnostics),
        }
    }
    fn supported(&self, schema: &SchemaId, diagnostics: &mut Vec<ExampleDiagnostic>) -> bool {
        let directional = crate::schema_view::closure(self.contract, std::slice::from_ref(schema))
            .into_iter()
            .find_map(|id| {
                ["readOnly", "writeOnly"]
                    .into_iter()
                    .find(|key| {
                        self.contract.schema(&id).is_some_and(|schema| {
                            !crate::schema_view::reference_only(schema)
                                && schema
                                    .raw()
                                    .get(key)
                                    .is_some_and(|value| value != &Value::Bool(false))
                        })
                    })
                    .map(|key| id.child(key))
            });
        if let Some(source) = directional {
            diagnostics.push(diagnostic(self.contract,&source,"examples-directional-unsupported","contextual example validation requires the backend's proven directional projection; neutral example planning is unavailable for this slot"));
            return false;
        }
        true
    }
    fn declared_values(
        &mut self,
        slot: &Slot<'_>,
        declared: Vec<Candidate<'_>>,
        diagnostics: &mut Vec<ExampleDiagnostic>,
    ) -> Vec<ExampleEntry> {
        let mut entries = Vec::new();
        for candidate in declared {
            let value = match self.copy(candidate.value) {
                Ok(value) => value,
                Err(error) => {
                    self.report(&candidate.source, error, diagnostics);
                    continue;
                }
            };
            match self.outcome(&slot.schema, &value) {
                Ok(OwnedOutcome::Valid) => {
                    entries.push(ExampleEntry {
                        role: slot.role.clone(),
                        media_type: slot.media.clone(),
                        schema: slot.schema.clone(),
                        container: slot.container.clone(),
                        part_position: slot.part_position,
                        origin: ExampleOrigin::Declared,
                        name: candidate.name,
                        summary: candidate.summary,
                        declared_source: Some(candidate.source),
                        value,
                    });
                }
                Ok(OwnedOutcome::Invalid(findings)) => diagnostics.push(diagnostic(
                    self.contract,
                    &candidate.source,
                    "examples-declared-invalid",
                    format!(
                        "declared example does not satisfy its source schema: {}",
                        findings
                            .iter()
                            .take(3)
                            .map(|finding| format!(
                                "{} at {}",
                                finding.message, finding.instance_path
                            ))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                )),
                Ok(OwnedOutcome::EvaluationFailure(_)) => {
                    unreachable!("incomplete outcomes are errors")
                }
                Err(error) => self.report(&candidate.source, error, diagnostics),
            }
        }
        entries
    }
    fn first(&mut self, id: &SchemaId, stack: &mut Vec<SchemaId>) -> Result<Value, Failure> {
        for value in self.candidates(id, stack)? {
            if self.valid(id, &value)? {
                return Ok(value);
            }
        }
        Err(Failure::Unsupported(
            "bounded synthesis found no source-valid candidate".into(),
        ))
    }
    fn candidates(
        &mut self,
        id: &SchemaId,
        stack: &mut Vec<SchemaId>,
    ) -> Result<Vec<Value>, Failure> {
        self.charge(1)?;
        if stack.len() >= self.config.max_depth {
            return Err(Failure::Limit);
        }
        if stack.contains(id) {
            return Err(Failure::Unsupported(
                "required recursive structure has no finite candidate under this synthesis policy"
                    .into(),
            ));
        }
        stack.push(id.clone());
        let result = self.node_candidates(id, stack);
        stack.pop();
        result
    }
    fn node_candidates(
        &mut self,
        id: &SchemaId,
        stack: &mut Vec<SchemaId>,
    ) -> Result<Vec<Value>, Failure> {
        let schema = self
            .contract
            .schema(id)
            .ok_or_else(|| Failure::Unsupported("schema is not indexed".into()))?;
        let viewed = crate::schema_view::raw(schema);
        let raw = viewed.as_ref();
        if let Some(constant) = raw.get("const") {
            return Ok(vec![self.copy(constant)?]);
        }
        if let Some(values) = raw.get("enum").and_then(Value::as_array) {
            return values
                .iter()
                .take(self.config.max_candidates)
                .map(|value| self.copy(value))
                .collect();
        }
        if let Some(target) = schema
            .references()
            .iter()
            .find(|r| r.keyword == "$ref")
            .and_then(|r| r.target.as_ref())
        {
            return self.candidates(target, stack);
        }
        for keyword in ["oneOf", "anyOf", "allOf"] {
            if let Some(branches) = raw.get(keyword).and_then(Value::as_array) {
                let mut values = Vec::new();
                for index in 0..branches.len().min(self.config.max_candidates) {
                    match self.candidates(&id.child(keyword).child(&index.to_string()), stack) {
                        Ok(candidates) => values.extend(
                            candidates
                                .into_iter()
                                .take(self.config.max_candidates - values.len()),
                        ),
                        Err(Failure::Unsupported(_)) => {}
                        Err(error) => return Err(error),
                    }
                    if values.len() == self.config.max_candidates {
                        break;
                    }
                }
                return Ok(values);
            }
        }
        let types = match raw.get("type") {
            Some(Value::String(name)) => vec![name.as_str()],
            Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).collect(),
            _ => vec!["null"],
        };
        let mut values = Vec::new();
        for name in types.into_iter().take(self.config.max_candidates) {
            if matches!(name, "number" | "integer") {
                for field in ["minimum", "maximum", "multipleOf"] {
                    if let Some(value) = raw.get(field).filter(|value| value.is_number()) {
                        values.push(self.copy(value)?);
                    }
                }
                values.extend([Value::from(0), Value::from(1), Value::from(-1)]);
            } else {
                let result = match name {
                    "null" => Ok(Value::Null),
                    "boolean" => Ok(Value::Bool(true)),
                    "string" => self.string(raw),
                    "array" => self.array(id, raw, stack),
                    "object" => self.object(id, raw, stack),
                    _ => Err(Failure::Unsupported("unsupported synthesis type".into())),
                };
                match result {
                    Ok(value) => values.push(value),
                    Err(Failure::Unsupported(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            values.truncate(self.config.max_candidates);
        }
        Ok(values)
    }
    fn cardinality(&self, raw: &Value, key: &str, default: usize) -> Result<usize, Failure> {
        match raw.get(key) {
            None => Ok(default),
            Some(value) => value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| {
                    Failure::Unsupported(format!(
                        "{key} has no bounded native cardinality for synthesis"
                    ))
                }),
        }
    }
    fn string(&mut self, raw: &Value) -> Result<Value, Failure> {
        let min = self.cardinality(raw, "minLength", 0)?;
        let max = self.cardinality(raw, "maxLength", self.config.max_string_length)?;
        if min > max || min > self.config.max_string_length {
            return Err(Failure::Unsupported(
                "string length constraints exceed bounded synthesis".into(),
            ));
        }
        let size = if min == 0 && max > 0 { 1 } else { min };
        self.charge(size.saturating_add(1))?;
        Ok(Value::String("x".repeat(size)))
    }
    fn array(
        &mut self,
        id: &SchemaId,
        raw: &Value,
        stack: &mut Vec<SchemaId>,
    ) -> Result<Value, Failure> {
        let size = self.cardinality(raw, "minItems", 0)?;
        if size > self.config.max_candidates {
            return Err(Failure::Unsupported(
                "minItems exceeds the bounded sample array policy".into(),
            ));
        }
        let mut values = Vec::new();
        for index in 0..size {
            let prefix = id.child("prefixItems").child(&index.to_string());
            let item = if self.contract.schema(&prefix).is_some() {
                prefix
            } else {
                id.child("items")
            };
            values.push(if self.contract.schema(&item).is_some() {
                self.first(&item, stack)?
            } else {
                Value::Null
            });
        }
        Ok(Value::Array(values))
    }
    fn object(
        &mut self,
        id: &SchemaId,
        raw: &Value,
        stack: &mut Vec<SchemaId>,
    ) -> Result<Value, Failure> {
        let mut object = serde_json::Map::new();
        if let Some(required) = raw.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                self.charge(name.len().saturating_add(1))?;
                let property = id.child("properties").child(name);
                let extra = id.child("additionalProperties");
                let value = if self.contract.schema(&property).is_some() {
                    self.first(&property, stack)?
                } else if self.contract.schema(&extra).is_some() {
                    self.first(&extra, stack)?
                } else {
                    Value::Null
                };
                object.insert(name.into(), value);
            }
        }
        Ok(Value::Object(object))
    }
}

struct Counter {
    left: usize,
    used: usize,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.left {
            return Err(io::Error::other("example byte budget"));
        }
        self.left -= bytes.len();
        self.used += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn declared_candidates<'a>(
    contract: &'a Contract,
    slot: &Slot<'a>,
    limit: usize,
    diagnostics: &mut Vec<ExampleDiagnostic>,
) -> Vec<Candidate<'a>> {
    let mut candidates = Vec::new();
    let raw = contract
        .source(&slot.definition)
        .expect("admitted container");
    if !slot.schema_only && raw.get("example").is_some() && raw.get("examples").is_some() {
        diagnostics.push(diagnostic(
            contract,
            &slot.definition,
            "examples-declared-conflict",
            "example and examples are mutually exclusive",
        ));
        return candidates;
    }
    if !slot.schema_only
        && let Some(value) = raw.get("example")
    {
        candidates.push(Candidate {
            value,
            source: slot.definition.child("example"),
            name: None,
            summary: None,
        });
        return candidates;
    }
    if !slot.schema_only
        && let Some(named) = raw.get("examples")
    {
        if !named.is_object() {
            diagnostics.push(diagnostic(
                contract,
                &slot.definition.child("examples"),
                "examples-declared-malformed",
                "named examples must be an object",
            ));
            return candidates;
        }
        if slot.named.len() > limit {
            diagnostics.push(diagnostic(
                contract,
                &slot.definition.child("examples"),
                "examples-declared-limit",
                "declared examples beyond the finite candidate limit were not evaluated",
            ));
        }
        for example in slot.named.iter().take(limit) {
            let terminal = example
                .resolved_source()
                .unwrap_or_else(|| example.source().clone());
            let raw = contract.source(&terminal).unwrap_or(example.raw());
            if !raw.is_object() || raw.get("value").is_some() && raw.get("externalValue").is_some()
            {
                diagnostics.push(diagnostic(
                    contract,
                    &terminal,
                    "examples-declared-malformed",
                    "Example Object must be an object with mutually exclusive value/externalValue",
                ));
                continue;
            }
            if raw.get("externalValue").is_some() {
                if !raw["externalValue"].is_string() {
                    diagnostics.push(diagnostic(
                        contract,
                        &terminal.child("externalValue"),
                        "examples-declared-malformed",
                        "externalValue must be a string URL",
                    ));
                    continue;
                }
                // In OAS 3.2, dataValue is the independently supplied instance;
                // externalValue can identify its serialization. Use that data
                // without acquiring the external serialization.
                if example.data_value().is_none() {
                    diagnostics.push(diagnostic(contract,&terminal.child("externalValue"),"examples-declared-external-value","external example values require explicit acquisition; the planner does not fetch them"));
                    continue;
                }
            }
            for field in ["summary", "description"] {
                if raw.get(field).is_some_and(|value| !value.is_string()) {
                    diagnostics.push(diagnostic(
                        contract,
                        &terminal.child(field),
                        "examples-declared-malformed",
                        format!("Example Object {field} must be a string"),
                    ));
                }
            }
            if let (Some(value), Some(source)) = (
                example.data_value().or_else(|| example.value()),
                example
                    .data_value_source()
                    .or_else(|| example.value_source()),
            ) {
                candidates.push(Candidate {
                    value,
                    source,
                    name: Some(example.name().into()),
                    summary: example.summary().map(str::to_owned),
                });
            } else {
                diagnostics.push(diagnostic(
                    contract,
                    &terminal,
                    "examples-declared-unavailable",
                    "Example Object has no resolved value",
                ));
            }
        }
        return candidates;
    }
    // Container examples override schema annotations. Static schema references
    // may contribute annotations, but applicator branch examples are not root examples.
    let mut pending = vec![slot.schema.clone()];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if seen.len() > 64 {
            diagnostics.push(diagnostic(
                contract,
                &slot.schema,
                "examples-declared-limit",
                "schema annotation reference traversal reached its finite limit",
            ));
            break;
        }
        let Some(schema) = contract.schema(&id) else {
            continue;
        };
        if !crate::schema_view::reference_only(schema)
            && let Some(value) = schema.raw().get("example")
        {
            candidates.push(Candidate {
                value,
                source: id.child("example"),
                name: None,
                summary: None,
            });
        }
        if !crate::schema_view::reference_only(schema)
            && let Some(values) = schema.raw().get("examples")
        {
            if let Some(values) = values.as_array() {
                for (index, value) in values.iter().take(limit.saturating_add(1)).enumerate() {
                    candidates.push(Candidate {
                        value,
                        source: id.child("examples").child(&index.to_string()),
                        name: None,
                        summary: None,
                    });
                }
            } else {
                diagnostics.push(diagnostic(
                    contract,
                    &id.child("examples"),
                    "examples-declared-malformed",
                    "schema examples annotation must be an array",
                ));
            }
        }
        if candidates.len() > limit {
            candidates.truncate(limit);
            diagnostics.push(diagnostic(
                contract,
                &id,
                "examples-declared-limit",
                "declared examples beyond the finite candidate limit were not evaluated",
            ));
            break;
        }
        pending.extend(
            schema
                .references()
                .iter()
                .filter(|reference| reference.keyword == "$ref")
                .filter_map(|reference| reference.target.clone()),
        );
    }
    candidates
}
