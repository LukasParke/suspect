//! Compiler/validator configuration.

/// Configuration for [`Compiler`](crate::Compiler) and
/// [`Schema`](crate::Schema) validation.
#[derive(Debug, Clone)]
pub struct Config {
    /// When `true`, `format` keywords are asserted (the instance string must
    /// match the requested format); when `false` (the 2020-12 default)
    /// `format` is annotation-only and never fails validation.
    pub format_assertion: bool,

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
}

impl Default for Config {
    fn default() -> Self {
        Self { format_assertion: false, max_depth: 512, max_errors: 100 }
    }
}
