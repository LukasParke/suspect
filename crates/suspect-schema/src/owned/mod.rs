//! Immutable validation over a selected closure of an owned OpenAPI Contract.
//!
//! Compilation either proves support for every selected schema or returns
//! located limitations. OAS 3.0 Schema Objects and the standard OAS 3.1/3.2
//! and JSON Schema 2020-12 dialects lower to bounded instruction sets. The
//! original compiler keeps v1 admission; `compile_v2` adds modern applicators
//! and scoped evaluated locations. Dynamic references and embedded resources
//! still require canonical resource/scope metadata from Contract.
//! `pattern` uses a bounded portable ECMA-262 Unicode regular subset.

mod compile;
mod dialect;
mod eval;
mod program;
mod program_check;
mod resource;

pub use program::{
    OwnedProgram, ProgramCheck, ProgramCheckError, ProgramCountTarget, ProgramInstruction,
    ProgramLimits, ProgramNode, ProgramProperty, ProgramResource, ProgramResourceContext,
    ProgramRoot, ProgramSource, ProgramType,
};

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use serde_json::Value;
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_low::Pointer;

use crate::Config;
use crate::compile::TypeBits;
use crate::keywords::cardinality::CountBound;
use crate::number::{Divisor, ExactNumber};

/// Why an owned schema closure could not be compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedCompileErrorKind {
    /// A requested root does not identify an indexed Contract schema.
    UnknownRoot,
    /// A supported keyword has an invalid declaration or reference.
    Invalid,
    /// The declared dialect or validation feature is not supported.
    Unsupported,
    /// Compilation exceeded an exact arithmetic resource budget.
    ResourceLimit,
}

/// A source-located reason that an owned validator could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedCompileError {
    /// Stable document URI and JSON Pointer, independent of loader lifetimes.
    pub source: SourceId,
    /// Original source bytes, when the address names a retained source value.
    pub span: Option<Range<usize>>,
    /// Machine-readable distinction between invalid, unsupported, and limited.
    pub kind: OwnedCompileErrorKind,
    /// Human-readable explanation.
    pub message: String,
}

impl std::fmt::Display for OwnedCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({}#{})",
            self.message,
            self.source.document(),
            self.source.pointer()
        )
    }
}

impl std::error::Error for OwnedCompileError {}

/// An invalid instance value or the cause of incomplete evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedFinding {
    /// Stable source address of the keyword or schema responsible.
    pub source: SourceId,
    /// RFC 6901 pointer to the offending instance value.
    pub instance_path: Pointer,
    /// Human-readable explanation.
    pub message: String,
}

/// Result of one independent validation call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedOutcome {
    /// Every assertion in the selected supported program passed.
    Valid,
    /// Evaluation completed and one or more assertions rejected the instance.
    Invalid(Vec<OwnedFinding>),
    /// Validity is unknown. Logical branches cannot suppress or invert this
    /// outcome, including failures from resource limits or unknown root IDs.
    EvaluationFailure(OwnedFinding),
}

/// Compiler for an immutable, owned, statically indexed validation program.
pub struct OwnedCompiler {
    config: Config,
}

impl OwnedCompiler {
    /// Creates a compiler with per-call evaluation and exact arithmetic limits.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Compiles the selected roots' dialect-aware containment/reference closure.
    /// OAS 3.0 Reference Object siblings do not expand that closure.
    /// Unknown roots fail explicitly, and unrelated Contract diagnostics do
    /// not disable a supported selected closure. The retained Contract owns
    /// all source values; no source documents are reparsed or self-borrowed.
    ///
    /// # Errors
    /// Returns located unsupported features, invalid declarations, missing
    /// roots/references, or exhausted exact-number compilation budgets.
    pub fn compile(
        &self,
        contract: Arc<Contract>,
        roots: &[SchemaId],
    ) -> Result<OwnedSchema, Vec<OwnedCompileError>> {
        compile::compile(contract, roots, self.config.clone(), CompileProfile::V1)
    }

    /// Admits modern conditional, dependency, contains, pattern/name and
    /// unevaluated applicators. Base closures keep their exact v1 program;
    /// closures requiring new instructions use the checked v2 profile.
    ///
    /// # Errors
    /// Same source/dialect/resource errors as [`Self::compile`]. Canonical
    /// resources and dynamic references remain explicit unsupported features.
    pub fn compile_v2(
        &self,
        contract: Arc<Contract>,
        roots: &[SchemaId],
    ) -> Result<OwnedSchema, Vec<OwnedCompileError>> {
        compile::compile(contract, roots, self.config.clone(), CompileProfile::V2)
    }

    /// Admits canonical resources and 2020-12 dynamic references using only the
    /// immutable Contract registry. Always emits an explicit v3 resource profile;
    /// v1/v2 entrypoints and their instruction meanings remain unchanged.
    ///
    /// # Errors
    /// Located invalid/ambiguous resources, unresolved references, unsupported
    /// dialects/vocabularies/legacy recursion, or compilation resource limits.
    pub fn compile_v3(
        &self,
        contract: Arc<Contract>,
        roots: &[SchemaId],
    ) -> Result<OwnedSchema, Vec<OwnedCompileError>> {
        compile::compile(contract, roots, self.config.clone(), CompileProfile::V3)
    }
}

/// Immutable owned program, safe to share across threads with `Arc`.
///
/// Each call creates its own recursion state, findings and equality budget.
/// A program retains one `Arc<Contract>` plus finite indexed schema edges;
/// recursive reference cycles never create self-referential Rust values.
pub struct OwnedSchema {
    contract: Arc<Contract>,
    nodes: Vec<Node>,
    roots: BTreeMap<SchemaId, usize>,
    config: Config,
    applicators: bool,
    resources: Option<ProgramResourceContext>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CompileProfile {
    V1,
    V2,
    V3,
}

impl OwnedSchema {
    /// Retained immutable source contract.
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }

    /// Requested root IDs, sorted and deduplicated.
    pub fn roots(&self) -> impl Iterator<Item = &SchemaId> {
        self.roots.keys()
    }

    /// Snapshots the successfully compiled instructions for a portable runtime.
    ///
    /// This experimental v1/v2 representation preserves source identity, finite
    /// reference indices, exact operands, and evaluation limits. It is derived
    /// from this program, without interpreting or reparsing schema keywords.
    /// It is not a stable serialization ABI; consumers must check its version
    /// and implement its complete declared subset before evaluating it.
    #[must_use]
    pub fn program(&self) -> OwnedProgram {
        program::snapshot(self)
    }

    /// Validates an owned JSON value against a root selected at compilation.
    /// An unselected or unknown ID is an evaluation failure, never success.
    /// `max_errors` limits reported mismatches, not evaluation completeness.
    #[must_use]
    pub fn validate(&self, root: &SchemaId, instance: &Value) -> OwnedOutcome {
        let Some(&index) = self.roots.get(root) else {
            return OwnedOutcome::EvaluationFailure(OwnedFinding {
                source: root.clone(),
                instance_path: Pointer::root(),
                message: "schema ID was not selected as a root of this program".into(),
            });
        };
        eval::validate(self, index, instance)
    }
}

struct Node {
    source: SchemaId,
    checks: Vec<Check>,
}

struct Check {
    source: SourceId,
    kind: Kind,
}

enum Kind {
    Always(bool),
    Type(TypeBits),
    Reference(usize),
    DynamicReference {
        target: usize,
        initial_resource: usize,
        anchor: Option<String>,
    },
    Properties(Vec<(String, usize)>),
    AdditionalProperties {
        declared: BTreeSet<String>,
        schema: usize,
    },
    Required(Vec<String>),
    Items {
        schema: usize,
        start: usize,
    },
    PrefixItems(Vec<usize>),
    AllOf(Vec<usize>),
    AnyOf(Vec<usize>),
    OneOf(Vec<usize>),
    Not(usize),
    If {
        condition: usize,
        then_target: Option<usize>,
        else_target: Option<usize>,
    },
    DependentRequired(Vec<(String, Vec<String>)>),
    DependentSchemas(Vec<(String, usize)>),
    Contains {
        schema: usize,
        minimum: Option<ContainsLimit>,
        maximum: Option<ContainsLimit>,
    },
    PatternProperties(Vec<(String, crate::PatternProgram, usize)>),
    AdditionalPropertiesWithPatterns {
        declared: BTreeSet<String>,
        schema: usize,
    },
    PropertyNames(usize),
    UnevaluatedProperties(usize),
    UnevaluatedItems(usize),
    Bound {
        number: ExactNumber,
        maximum: bool,
        exclusive: bool,
    },
    MultipleOf(Divisor),
    Count {
        bound: CountBound,
        token: String,
        maximum: bool,
        target: CountTarget,
    },
    Enum,
    Const,
    UniqueItems,
    Pattern(crate::PatternProgram),
}

struct ContainsLimit {
    bound: CountBound,
    token: String,
}

#[derive(Clone, Copy)]
enum CountTarget {
    String,
    Array,
    Object,
}
