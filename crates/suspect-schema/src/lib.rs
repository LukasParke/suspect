#![deny(missing_docs)]
//! suspect-schema: JSON Schema 2020-12 compiler and validator.
//!
//! [`Compiler`] turns a schema [`NodeRef`] from the
//! `suspect-low` spine into a [`Schema`]: an eagerly compiled program of
//! checks. `$ref` targets compile lazily on first use (memoized in a
//! thread-local cache), so recursive schemas work out of the box.
//!
//! `$dynamicRef`/`$dynamicAnchor` implement RFC 3093 *basic* semantics: the
//! dynamic scope is walked outermost-first and the first fragment declaring
//! the anchor wins, falling back to static resolution through the document's
//! `$dynamicAnchor` registry.
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
mod errors;
mod exec;
mod keywords;

pub use compile::Compiler;
use std::cell::RefCell;

use rustc_hash::FxHashMap;
use suspect_low::{NodeRef, Pointer};

pub use config::Config;
pub use errors::{CompileError, SchemaError};

use compile::{Prg, Scan};

/// A compiled JSON Schema 2020-12 validator bound to its source document.
///
/// **Not `Sync` and not `Send`:** `$ref` targets resolve lazily at first use
/// into a `RefCell<FxHashMap<Pointer, Option<Schema>>>` cache. Use the
/// `Schema` by reference within one thread; compile one per thread for
/// parallel validation of the same document.
pub struct Schema<'d> {
    root: NodeRef<'d>,
    program: Prg<'d>,
    scan: Scan,
    root_base: String,
    config: Config,
    cache: RefCell<FxHashMap<Pointer, Option<Prg<'d>>>>,
}

impl<'d> Schema<'d> {
    pub(crate) fn new(
        root: NodeRef<'d>,
        program: Prg<'d>,
        scan: Scan,
        root_base: String,
        config: Config,
    ) -> Self {
        Self { root, program, scan, root_base, config, cache: RefCell::new(FxHashMap::default()) }
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
            masks: exec::Masks::default(),
            out: Vec::new(),
            aborted: false,
            depth: 0,
            dyn_scope: Vec::new(),
        };
        let mut st = exec::Stack::new();
        exec::eval(&mut ctx, &self.program, instance, &mut st);
        ctx.out
    }

    /// Validates with early exit: stops at the first failure and returns it,
    /// or `None` when the instance is valid.
    #[must_use]
    pub fn validate_first(&self, instance: NodeRef<'d>) -> Option<SchemaError> {
        let mut ctx = exec::Ctx {
            sch: self,
            cap: 1,
            masks: exec::Masks::default(),
            out: Vec::new(),
            aborted: false,
            depth: 0,
            dyn_scope: Vec::new(),
        };
        let mut st = exec::Stack::new();
        exec::eval(&mut ctx, &self.program, instance, &mut st);
        ctx.out.into_iter().next()
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
    pub(crate) fn root_base(&self) -> &str {
        &self.root_base
    }
}
