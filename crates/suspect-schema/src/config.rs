//! Compiler/validator configuration.

/// Configuration for [`Compiler`](crate::Compiler) and
/// [`Schema`](crate::Schema) validation.
#[derive(Debug, Clone)]
pub struct Config {
    /// When `true`, `format` keywords are asserted (the instance string must
    /// match the requested format); when `false` (the 2020-12 default)
    /// `format` is annotation-only and never fails validation.
    pub format_assertion: bool,

    /// When `true`, an OAS 3.0 `nullable` annotation on an OAS 3.1/3.2 Schema
    /// Object is interpreted with its 3.0 semantics: `nullable: true` appends
    /// `"null"` to the same-object type and `nullable: false` removes it. The
    /// default (2020-12) leaves the keyword annotation-only. This never
    /// applies to OAS 3.0 documents, whose dialect already gives `nullable`
    /// that meaning.
    pub oas30_nullable_in_31: bool,

    /// Maximum nesting depth, reused for two guards:
    ///
    /// - **compile time**: eagerly-compiled subschema nesting beyond this
    ///   yields [`CompileError::TooDeep`](crate::CompileError::TooDeep);
    /// - **execution**: recursive evaluation (including `$ref` cycles driven
    ///   by deep instances) beyond this depth produces a clean
    ///   [`SchemaError`](crate::SchemaError) instead of a stack overflow.
    ///
    /// Keep this value moderate: execution recursion consumes native stack,
    /// roughly 2 frames per instance-nesting level. The default (512) uses a
    /// few hundred kilobytes of stack; values above ~10_000 risk exhausting
    /// small stacks.
    ///
    /// Default: `512`.
    pub max_depth: usize,

    /// Maximum accumulated [`SchemaError`](crate::SchemaError)s per `validate` call before
    /// evaluation aborts early. `0` means unlimited.
    ///
    /// Default: `100`.
    pub max_errors: usize,

    /// Maximum source bytes in one numeric operand for exact bounds,
    /// divisibility, or equality. Exponents stay symbolic; their magnitude
    /// does not consume this budget, only their written length does.
    /// Exceeding this limit is an explicit evaluation/compilation failure,
    /// never a rounded number or an ordinary schema mismatch.
    ///
    /// Default: `4096`. `0` permits no numeric operands for these checks.
    pub max_number_bytes: usize,

    /// Maximum node pairs compared across `enum`, `const`, and `uniqueItems`
    /// in a validation call. Equality uses an explicit stack bounded by this
    /// work limit and [`Self::max_depth`]. Exceeding either limit produces an
    /// evaluation failure, including inside logical schema branches.
    ///
    /// Default: `100_000`. `0` permits no equality comparisons.
    pub max_equality_steps: usize,

    /// Maximum schema, keyword, and collection/branch visits in one validation
    /// call, in both source-bound and owned runtimes. The budget survives
    /// logical trial branches and lazy references, and caps
    /// repeated work through finite reference graphs, independently of depth
    /// and equality limits. Exhaustion is an explicit evaluation failure.
    ///
    /// This is a visit bound, not a byte-work or memory bound: decoding a
    /// string or comparing a numeric operand also uses its representation
    /// length. Source-bound collection materialization and lazy schema
    /// compilation are not bounded by this counter. The exact visit accounting
    /// is runtime-specific. Default: `100_000`. `0` permits no evaluation visits.
    pub max_evaluation_steps: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            format_assertion: false,
            oas30_nullable_in_31: false,
            max_depth: 512,
            max_errors: 100,
            max_number_bytes: 4096,
            max_equality_steps: 100_000,
            max_evaluation_steps: 100_000,
        }
    }
}
