#![deny(missing_docs)]
//! suspect-schema: JSON Schema 2020-12 compiler and validator.
//!
//! [`Compiler`] turns a schema [`NodeRef`] from the
//! `suspect-low` spine into a [`Schema`]: an eagerly compiled program of
//! checks. `$ref` targets compile lazily on first use and are cached on the
//! compiled schema, including compilation failures.
//!
//! References resolve within indexed schema resources. `$dynamicRef` first
//! resolves statically; a dynamic-anchor fragment can then rebind through the
//! outermost resource in evaluation scope (2020-12 Core §8.2.3.2). External
//! loading is not configured by this API and produces an evaluation failure.
//! This source-bound engine is not a complete OpenAPI dialect validator.
//!
//! [`OwnedCompiler`] provides a separate immutable boundary over an owned
//! OpenAPI Contract, suitable for concurrent validation without source-document
//! lifetimes. It compiles an explicit static subset and rejects unsupported
//! features; [`OwnedOutcome`] separates proven invalidity from incomplete
//! evaluation.
//!
//! ```no_run
//! # use suspect_schema::{Compiler, Config};
//! # use suspect_low::LowDoc;
//! # use suspect_source::{Source, Uri};
//! # use std::path::Path;
//! let doc = LowDoc::parse(
//!     Uri::from_path(Path::new("/schema.json")).unwrap(),
//!     Source::from_vec(br#"{"type":"integer"}"#.to_vec()),
//! );
//! let schema = Compiler::new(Config::default()).compile(doc.root()).unwrap();
//! assert!(schema.validate(doc.root()).is_empty());
//! ```

mod compile;
mod config;
mod equality;
mod errors;
mod exec;
mod keywords;
mod number;
mod owned;
mod pattern;
mod resources;

pub use compile::Compiler;
use std::cell::RefCell;

use rustc_hash::FxHashMap;
use suspect_low::{NodeRef, Pointer};

pub use config::Config;
pub use errors::{CompileError, SchemaError, SchemaErrorKind};
pub use owned::{
    OwnedCompileError, OwnedCompileErrorKind, OwnedCompiler, OwnedFinding, OwnedOutcome,
    OwnedProgram, OwnedSchema, ProgramCheck, ProgramCheckError, ProgramCountTarget,
    ProgramInstruction, ProgramLimits, ProgramNode, ProgramProperty, ProgramResource,
    ProgramResourceContext, ProgramRoot, ProgramSource, ProgramType,
};
pub use pattern::{PatternError, PatternErrorKind, PatternProgram, PatternState, compile_pattern};

use compile::Prg;
use resources::Scan;

/// A compiled JSON Schema 2020-12 validator bound to its source document.
///
/// **Not `Sync` and not `Send`:** `$ref` targets resolve lazily at first use
/// into a `RefCell` cache that also retains compilation failures. Use the
/// `Schema` by reference within one thread; compile one per thread for
/// parallel validation of the same document.
pub struct Schema<'d> {
    root: NodeRef<'d>,
    program: Prg<'d>,
    scan: Scan,
    config: Config,
    cache: RefCell<FxHashMap<Pointer, Result<Option<Prg<'d>>, CompileError>>>,
}

impl<'d> Schema<'d> {
    pub(crate) fn new(root: NodeRef<'d>, program: Prg<'d>, scan: Scan, config: Config) -> Self {
        Self {
            root,
            program,
            scan,
            config,
            cache: RefCell::new(FxHashMap::default()),
        }
    }

    /// Validates an instance against this schema.
    ///
    /// Returns every accumulated failure, capped by
    /// [`Config::max_errors`] (`0` = unlimited). An empty vector means valid.
    #[must_use]
    pub fn validate(&self, instance: NodeRef<'d>) -> Vec<SchemaError> {
        let mut ctx = exec::Ctx {
            sch: self,
            cap: self.config.max_errors,
            out: Vec::new(),
            evaluation_error: None,
            equality: keywords::types::EqualityBudget::new(&self.config),
            remaining_steps: self.config.max_evaluation_steps,
            aborted: false,
            depth: 0,
            dyn_scope: Vec::new(),
        };
        let mut st = exec::Stack::new();
        exec::eval(&mut ctx, &self.program, instance, &mut st);
        if let Some(error) = ctx.evaluation_error {
            // Evaluation failure takes precedence and is never hidden by the
            // error cap or a diverted logical branch.
            ctx.out.insert(0, error);
            if ctx.cap != 0 {
                ctx.out.truncate(ctx.cap);
            }
        }
        ctx.out
    }

    /// Validates with early exit: stops at the first failure and returns it,
    /// or `None` when the instance is valid.
    #[must_use]
    pub fn validate_first(&self, instance: NodeRef<'d>) -> Option<SchemaError> {
        let mut ctx = exec::Ctx {
            sch: self,
            cap: 1,
            out: Vec::new(),
            evaluation_error: None,
            equality: keywords::types::EqualityBudget::new(&self.config),
            remaining_steps: self.config.max_evaluation_steps,
            aborted: false,
            depth: 0,
            dyn_scope: Vec::new(),
        };
        let mut st = exec::Stack::new();
        exec::eval(&mut ctx, &self.program, instance, &mut st);
        ctx.evaluation_error.or_else(|| ctx.out.into_iter().next())
    }

    /// The schema node this validator was compiled from.
    #[must_use]
    pub fn root(&self) -> NodeRef<'d> {
        self.root
    }

    // -- internal accessors for the executor ---------------------------------
    pub(crate) fn config(&self) -> &Config {
        &self.config
    }
    pub(crate) fn scan(&self) -> &Scan {
        &self.scan
    }
    pub(crate) fn root_node(&self) -> NodeRef<'d> {
        self.root
    }
}
