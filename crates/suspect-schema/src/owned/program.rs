//! Portable snapshots of proved, compiled instructions; no schema interpreter.

use serde::Serialize;

use super::*;

/// Experimental, owned serialization of a successfully compiled schema closure.
///
/// This is a versioned runtime input, not a stable ABI or a general JSON Schema
/// format. Consumers must recognize both `version` and `profile`, implement
/// every instruction, retain exact JSON number operands, and preserve distinct
/// valid, invalid, and evaluation-failure outcomes. An unrecognized instruction
/// or exhausted limit must never become a successful no-op.
///
/// The profile supports standard OAS 3.1/3.2 / JSON Schema 2020-12 static
/// references, boolean schemas, types, object/array applicators, composition,
/// exact numeric bounds and divisibility, cardinality, and structural equality.
/// Admitted OAS 3.0 schemas normalize to these same instructions. Annotation
/// keywords are not instructions. Unsupported assertions, dialects, embedded
/// resources, dynamic references and assertion-mode formats prevent compilation
/// before this snapshot can be produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OwnedProgram {
    /// Exact v1/v2 format discriminator; see the associated version constants.
    pub version: &'static str,
    /// Exact instruction profile paired with the version. Base closures retain
    /// v1; modern evaluated applicators opt into the v2 static-applicator profile.
    pub profile: &'static str,
    /// Selected root identities, sorted and deduplicated at compilation.
    pub roots: Vec<ProgramRoot>,
    /// Finite schema graph. All instruction targets index this array.
    pub nodes: Vec<ProgramNode>,
    /// Per-call limits copied from the compiler configuration.
    pub limits: ProgramLimits,
    /// V3-only immutable resource graph; entirely absent from v1/v2 JSON.
    #[serde(rename = "resourceContext", skip_serializing_if = "Option::is_none")]
    pub resource_context: Option<ProgramResourceContext>,
}

impl OwnedProgram {
    /// Frozen original program format.
    pub const V1_VERSION: &'static str = "suspect.validation.experimental.v1";
    /// Frozen original static instruction profile.
    pub const V1_PROFILE: &'static str = "oas31-jsonschema202012-static-subset";
    /// Additive evaluated-applicator format.
    pub const V2_VERSION: &'static str = "suspect.validation.experimental.v2";
    /// Static references and modern applicators with scoped evaluated locations.
    pub const V2_PROFILE: &'static str = "oas31-jsonschema202012-static-applicators";
    /// Canonical-resource/dynamic-reference format.
    pub const V3_VERSION: &'static str = "suspect.validation.experimental.v3";
    /// Resource-indexed dynamic scope plus the evaluated-applicator semantics.
    pub const V3_PROFILE: &'static str = "oas31-jsonschema202012-resources-dynamic";

    /// Checks portable structural and numeric invariants after public mutation.
    ///
    /// This shares exact operand semantics with the owned compiler and verifies
    /// source identities, graph targets, and applicator consistency. It does
    /// not reconstruct OpenAPI semantics or prove equivalence to the original
    /// Contract after mutation. Language-specific metadata limits remain the
    /// consuming emitter's responsibility. Literal `enum` / `const` number
    /// budgets are deliberately deferred until evaluation, as in compilation.
    ///
    /// # Errors
    /// Returns the first located malformed instruction, identity, target or
    /// exact operand; unsupported format versions/profiles have no source.
    pub fn check(&self) -> Result<(), ProgramCheckError> {
        program_check::check(self)
    }
}

/// A malformed or unsupported portable program found before runtime emission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramCheckError {
    /// Responsible original source identity; absent for global format errors.
    pub source: Option<ProgramSource>,
    /// Explanation of the failed portable invariant or numeric resource limit.
    pub message: String,
}

impl std::fmt::Display for ProgramCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)?;
        if let Some(source) = &self.source {
            write!(f, " ({}#{})", source.document, source.pointer)?;
        }
        Ok(())
    }
}

impl std::error::Error for ProgramCheckError {}

/// Document identity and escaped RFC 6901 pointer retained from the Contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramSource {
    /// Absolute source document URI, with no generated file substitution.
    pub document: String,
    /// Escaped JSON Pointer within that document; the empty string is its root.
    pub pointer: String,
}

impl From<&SourceId> for ProgramSource {
    fn from(source: &SourceId) -> Self {
        Self {
            document: source.document().to_string(),
            pointer: source.pointer().to_owned(),
        }
    }
}

/// One root selected during compilation and its finite schema index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramRoot {
    /// Original selected schema identity.
    pub source: ProgramSource,
    /// Index into [`OwnedProgram::nodes`].
    pub target: usize,
}

/// A compiled schema with conjunctive checks in evaluation order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramNode {
    /// Original schema identity.
    pub source: ProgramSource,
    /// All checks must pass; an empty list accepts every JSON value.
    pub checks: Vec<ProgramCheck>,
}

/// One source-located instruction, serialized with a flattened `op` tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramCheck {
    /// Original keyword location, or schema location for a boolean schema.
    /// A normalized OAS 3.0 exclusive bound retains its minimum/maximum source.
    pub source: ProgramSource,
    /// Already compiled assertion or applicator.
    #[serde(flatten)]
    pub instruction: ProgramInstruction,
}

/// A JSON type name. `integer` means mathematical integrality, not wire syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgramType {
    /// JSON null.
    Null,
    /// JSON true or false.
    Boolean,
    /// An exact number with no mathematical fractional component.
    Integer,
    /// Any JSON number, including mathematical integers.
    Number,
    /// A Unicode string.
    String,
    /// A JSON array.
    Array,
    /// A JSON object.
    Object,
}

/// The instance kind and size measure to which a cardinality check applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgramCountTarget {
    /// Unicode scalar values in a string, not UTF-8 bytes or UTF-16 code units.
    String,
    /// Elements of an array.
    Array,
    /// Members of an object.
    Object,
}

/// A decoded property name and its compiled schema target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramProperty {
    /// Decoded JSON member name, preserving arbitrary names such as `a/b~`.
    pub name: String,
    /// Index into [`OwnedProgram::nodes`].
    pub target: usize,
}

/// V3 resource metadata copied from Contract, separate from physical node IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramResourceContext {
    /// Resources ordered by physical source identity.
    pub resources: Vec<ProgramResource>,
    /// One (resource index, original schema-root source, canonical address) per node.
    pub node_scopes: Vec<(usize, ProgramSource, String)>,
}

/// One canonical resource and its selected, indexed dynamic bindings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramResource {
    /// Physical source boundary, never replaced by its canonical URI.
    pub source: ProgramSource,
    /// `document`, `openApiDocument`, or `schema`, as indexed by Contract.
    pub kind: &'static str,
    /// Logical canonical resource identifier.
    pub canonical_uri: String,
    /// Fragment-free base URI.
    pub base_uri: String,
    /// Accepted indexed names; these grant no acquisition authority.
    pub aliases: Vec<String>,
    /// Original `$id`/`$self` value, if the base was explicitly declared.
    pub declaration_source: Option<ProgramSource>,
    /// Ordered (anchor name, original `$dynamicAnchor` source, target node index).
    pub dynamic_anchors: Vec<(String, ProgramSource, usize)>,
}

/// Typed instructions for the experimental v1/v2 static profiles.
///
/// Applicability follows JSON Schema: numeric checks apply only to numbers;
/// object/array/string checks apply only to their instance kind and never
/// imply a type or insert defaults. Composition evaluates every alternative;
/// evaluation failures cannot be suppressed or inverted by logical branches.
/// All `target` / `targets` indices refer to [`OwnedProgram::nodes`], except
/// `Count::target`, which names the cardinality instance kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ProgramInstruction {
    /// The truth value of a boolean schema.
    Always {
        /// Whether every instance is accepted.
        value: bool,
    },
    /// Union of the declared JSON types, in canonical type order.
    Type {
        /// Accepted type names; no coercion is permitted.
        types: Vec<ProgramType>,
    },
    /// A resolved static reference. Sibling checks still apply.
    Ref {
        /// Referenced schema index.
        target: usize,
    },
    /// V3: initial resolution plus outermost matching entered-resource override.
    DynamicRef {
        /// Statically resolved initial target; retained if no override applies.
        target: usize,
        /// Resource containing the initial target.
        initial_resource: usize,
        /// Dynamic plain-name anchor, or None for pointer/empty/static-anchor fallback.
        anchor: Option<String>,
    },
    /// Validate present declared members; absent members remain absent.
    Properties {
        /// Decoded property names and their schema indices.
        properties: Vec<ProgramProperty>,
    },
    /// Validate members not declared in this schema's `properties` keyword.
    AdditionalProperties {
        /// Sorted decoded names excluded from this check.
        declared: Vec<String>,
        /// Schema applied to each undeclared member.
        target: usize,
    },
    /// Assert presence of members on object instances.
    Required {
        /// Decoded required member names in declaration order.
        names: Vec<String>,
    },
    /// Validate array elements beyond this schema's `prefixItems`.
    Items {
        /// Schema applied to each remaining element.
        target: usize,
        /// First zero-based array index to validate.
        start: usize,
    },
    /// Validate each present positional element, without requiring its presence.
    PrefixItems {
        /// Schema indices in positional order.
        targets: Vec<usize>,
    },
    /// Every branch must accept the instance.
    AllOf {
        /// Schema indices in declaration order.
        targets: Vec<usize>,
    },
    /// At least one branch must accept the instance.
    AnyOf {
        /// Schema indices in declaration order.
        targets: Vec<usize>,
    },
    /// Exactly one branch must accept the instance.
    OneOf {
        /// Schema indices in declaration order.
        targets: Vec<usize>,
    },
    /// The child must reject the instance with a completed invalid result.
    Not {
        /// Negated schema index.
        target: usize,
    },
    /// Exact comparison against an inclusive or exclusive numeric bound.
    Bound {
        /// Validated JSON numeric token; never parse through binary floating point.
        value: String,
        /// Upper bound when true; lower bound when false.
        maximum: bool,
        /// Equality is rejected when true.
        exclusive: bool,
    },
    /// Exact divisibility by a positive JSON number.
    MultipleOf {
        /// Validated positive numeric token, preserving the compiled spelling.
        value: String,
    },
    /// Nonnegative mathematical integer cardinality, with an unbounded exponent.
    Count {
        /// Validated numeric token, including values beyond machine integers.
        value: String,
        /// Maximum size when true; minimum size when false.
        maximum: bool,
        /// Applicable instance kind and count measure.
        target: ProgramCountTarget,
    },
    /// Exact structural equality with at least one retained JSON literal.
    Enum {
        /// Compiled literals, including arbitrary-precision JSON numbers.
        values: Vec<Value>,
    },
    /// Exact structural equality with a retained JSON literal.
    Const {
        /// Compiled literal, including arbitrary-precision JSON numbers.
        value: Value,
    },
    /// All pairs of array items must differ under exact structural equality.
    UniqueItems,
    /// Portable ECMA-262 Unicode pattern NFA.
    Pattern {
        /// Compiled finite pattern program.
        program: crate::PatternProgram,
    },
    /// V2: trial the condition once, then evaluate only the selected branch.
    If {
        /// The schema at the original adjacent `if` location.
        condition: usize,
        /// The adjacent `then` schema, if declared.
        then_target: Option<usize>,
        /// The adjacent `else` schema, if declared.
        else_target: Option<usize>,
    },
    /// V2: object-only presence triggers; null-valued members are present.
    DependentRequired {
        /// Ordered (decoded trigger, required names) pairs. No annotations.
        dependencies: Vec<(String, Vec<String>)>,
    },
    /// V2: apply each triggered schema to the whole object in a fresh scope.
    DependentSchemas {
        /// Ordered trigger names and original child schema targets.
        dependencies: Vec<ProgramProperty>,
    },
    /// V2: trial every array element and retain successful-match indices.
    Contains {
        /// Original `contains` schema target.
        target: usize,
        /// Exact explicit minContains token; absence means the semantic default 1.
        minimum: Option<String>,
        /// Exact explicit maxContains token; absence means no maximum.
        maximum: Option<String>,
    },
    /// V2: every matching pattern applies; overlapping patterns are conjunctive.
    PatternProperties {
        /// Ordered (decoded pattern text, portable NFA, original schema target).
        patterns: Vec<(String, crate::PatternProgram, usize)>,
    },
    /// V2: excludes declared names and any same-node PatternProperties match.
    AdditionalPropertiesWithPatterns {
        /// Sorted decoded names from adjacent properties.
        declared: Vec<String>,
        /// Original additionalProperties schema target.
        target: usize,
    },
    /// V2: evaluates decoded names as strings; never marks property values.
    PropertyNames {
        /// Original propertyNames schema target.
        target: usize,
    },
    /// V2: applies after other checks to locally unmarked property values.
    UnevaluatedProperties {
        /// Original unevaluatedProperties schema target.
        target: usize,
    },
    /// V2: applies after other checks to locally unmarked array elements.
    UnevaluatedItems {
        /// Original unevaluatedItems schema target.
        target: usize,
    },
}

impl ProgramInstruction {
    /// Whether canonical resource/dynamic-scope semantics are required.
    #[must_use]
    pub fn requires_v3(&self) -> bool {
        matches!(self, Self::DynamicRef { .. })
    }
    /// Whether this instruction requires v2 scoped-applicator semantics.
    #[must_use]
    pub fn requires_v2(&self) -> bool {
        matches!(
            self,
            Self::If { .. }
                | Self::DependentRequired { .. }
                | Self::DependentSchemas { .. }
                | Self::Contains { .. }
                | Self::PatternProperties { .. }
                | Self::AdditionalPropertiesWithPatterns { .. }
                | Self::PropertyNames { .. }
                | Self::UnevaluatedProperties { .. }
                | Self::UnevaluatedItems { .. }
        )
    }
}

/// Per-call execution limits with the same zero semantics as [`Config`].
///
/// Schema/check/collection visits and equality comparisons share their own
/// budgets across all logical trials. These bound visits, not total byte work
/// or allocation. A runtime must not interpret exhaustion as ordinary invalidity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramLimits {
    /// Maximum recursive schema and structural equality depth.
    pub max_depth: usize,
    /// Maximum reported mismatch findings; zero means unlimited reporting.
    pub max_errors: usize,
    /// Maximum bytes in one exact numeric operand; zero allows no such operands.
    pub max_number_bytes: usize,
    /// Maximum structural equality node-pair comparisons; zero allows none.
    pub max_equality_steps: usize,
    /// Maximum schema, check and collection/branch visits; zero allows none.
    pub max_evaluation_steps: usize,
}

pub(super) fn snapshot(program: &OwnedSchema) -> OwnedProgram {
    OwnedProgram {
        version: if program.resources.is_some() {
            OwnedProgram::V3_VERSION
        } else if program.applicators {
            OwnedProgram::V2_VERSION
        } else {
            OwnedProgram::V1_VERSION
        },
        profile: if program.resources.is_some() {
            OwnedProgram::V3_PROFILE
        } else if program.applicators {
            OwnedProgram::V2_PROFILE
        } else {
            OwnedProgram::V1_PROFILE
        },
        roots: program
            .roots
            .iter()
            .map(|(source, &target)| ProgramRoot {
                source: source.into(),
                target,
            })
            .collect(),
        nodes: program
            .nodes
            .iter()
            .map(|node| ProgramNode {
                source: (&node.source).into(),
                checks: node
                    .checks
                    .iter()
                    .map(|check| ProgramCheck {
                        source: (&check.source).into(),
                        instruction: instruction(program, check),
                    })
                    .collect(),
            })
            .collect(),
        limits: ProgramLimits {
            max_depth: program.config.max_depth,
            max_errors: program.config.max_errors,
            max_number_bytes: program.config.max_number_bytes,
            max_equality_steps: program.config.max_equality_steps,
            max_evaluation_steps: program.config.max_evaluation_steps,
        },
        resource_context: program.resources.clone(),
    }
}

fn instruction(program: &OwnedSchema, check: &Check) -> ProgramInstruction {
    match &check.kind {
        Kind::Always(value) => ProgramInstruction::Always { value: *value },
        Kind::Type(bits) => ProgramInstruction::Type {
            types: [
                (TypeBits::NULL, ProgramType::Null),
                (TypeBits::BOOL, ProgramType::Boolean),
                (TypeBits::INT, ProgramType::Integer),
                (TypeBits::NUM, ProgramType::Number),
                (TypeBits::STR, ProgramType::String),
                (TypeBits::ARR, ProgramType::Array),
                (TypeBits::OBJ, ProgramType::Object),
            ]
            .into_iter()
            .filter_map(|(bit, ty)| (bits.0 & bit != 0).then_some(ty))
            .collect(),
        },
        Kind::Reference(target) => ProgramInstruction::Ref { target: *target },
        Kind::DynamicReference {
            target,
            initial_resource,
            anchor,
        } => ProgramInstruction::DynamicRef {
            target: *target,
            initial_resource: *initial_resource,
            anchor: anchor.clone(),
        },
        Kind::Properties(properties) => ProgramInstruction::Properties {
            properties: properties
                .iter()
                .map(|(name, target)| ProgramProperty {
                    name: name.clone(),
                    target: *target,
                })
                .collect(),
        },
        Kind::AdditionalProperties { declared, schema } => {
            ProgramInstruction::AdditionalProperties {
                declared: declared.iter().cloned().collect(),
                target: *schema,
            }
        }
        Kind::Required(names) => ProgramInstruction::Required {
            names: names.clone(),
        },
        Kind::Items { schema, start } => ProgramInstruction::Items {
            target: *schema,
            start: *start,
        },
        Kind::PrefixItems(targets) => ProgramInstruction::PrefixItems {
            targets: targets.clone(),
        },
        Kind::AllOf(targets) => ProgramInstruction::AllOf {
            targets: targets.clone(),
        },
        Kind::AnyOf(targets) => ProgramInstruction::AnyOf {
            targets: targets.clone(),
        },
        Kind::OneOf(targets) => ProgramInstruction::OneOf {
            targets: targets.clone(),
        },
        Kind::Not(target) => ProgramInstruction::Not { target: *target },
        Kind::If {
            condition,
            then_target,
            else_target,
        } => ProgramInstruction::If {
            condition: *condition,
            then_target: *then_target,
            else_target: *else_target,
        },
        Kind::DependentRequired(dependencies) => ProgramInstruction::DependentRequired {
            dependencies: dependencies.clone(),
        },
        Kind::DependentSchemas(dependencies) => ProgramInstruction::DependentSchemas {
            dependencies: dependencies
                .iter()
                .map(|(name, target)| ProgramProperty {
                    name: name.clone(),
                    target: *target,
                })
                .collect(),
        },
        Kind::Contains {
            schema,
            minimum,
            maximum,
        } => ProgramInstruction::Contains {
            target: *schema,
            minimum: minimum.as_ref().map(|bound| bound.token.clone()),
            maximum: maximum.as_ref().map(|bound| bound.token.clone()),
        },
        Kind::PatternProperties(patterns) => ProgramInstruction::PatternProperties {
            patterns: patterns.clone(),
        },
        Kind::AdditionalPropertiesWithPatterns { declared, schema } => {
            ProgramInstruction::AdditionalPropertiesWithPatterns {
                declared: declared.iter().cloned().collect(),
                target: *schema,
            }
        }
        Kind::PropertyNames(target) => ProgramInstruction::PropertyNames { target: *target },
        Kind::UnevaluatedProperties(target) => {
            ProgramInstruction::UnevaluatedProperties { target: *target }
        }
        Kind::UnevaluatedItems(target) => ProgramInstruction::UnevaluatedItems { target: *target },
        Kind::Bound {
            number,
            maximum,
            exclusive,
        } => ProgramInstruction::Bound {
            value: number.to_string(),
            maximum: *maximum,
            exclusive: *exclusive,
        },
        Kind::MultipleOf(divisor) => ProgramInstruction::MultipleOf {
            value: divisor.to_string(),
        },
        Kind::Count {
            token,
            maximum,
            target,
            ..
        } => ProgramInstruction::Count {
            value: token.clone(),
            maximum: *maximum,
            target: match target {
                CountTarget::String => ProgramCountTarget::String,
                CountTarget::Array => ProgramCountTarget::Array,
                CountTarget::Object => ProgramCountTarget::Object,
            },
        },
        // The compiled instruction stores a source address for its literal
        // operand; resolve that address exactly as the owned executor does.
        // There is no keyword discovery, declaration checking, or recursion
        // through raw schemas here.
        Kind::Enum => ProgramInstruction::Enum {
            values: program
                .contract
                .source(&check.source)
                .expect("retained enum")
                .as_array()
                .expect("compiled enum")
                .clone(),
        },
        Kind::Const => ProgramInstruction::Const {
            value: program
                .contract
                .source(&check.source)
                .expect("retained const")
                .clone(),
        },
        Kind::UniqueItems => ProgramInstruction::UniqueItems,
        Kind::Pattern(program) => ProgramInstruction::Pattern {
            program: program.clone(),
        },
    }
}
