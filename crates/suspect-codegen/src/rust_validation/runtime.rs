//! Execution state owns budgets; logical trials never reset or invert failures.
use super::number::Exact;
use super::pattern::{self, PatternProgram};
use crate::{JsonNonNullValue as J, JsonValue, Nullable as N};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Original schema location and the instance location responsible for a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationFinding {
    /// Absolute original source document URI.
    pub document: String,
    /// Original schema or keyword RFC 6901 pointer.
    pub pointer: String,
    /// RFC 6901 pointer into the supplied exact JSON value.
    pub instance_path: String,
    /// Human-readable mismatch or resource failure reason.
    pub message: String,
}
/// A completed schema decision or an explicit inability to complete evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    /// Every compiled assertion completed and passed.
    Valid,
    /// Completed mismatches, capped by the configured finding limit.
    Invalid(Vec<ValidationFinding>),
    /// Validity is unknown. This must never be treated as ordinary invalidity.
    EvaluationFailure(ValidationFinding),
}
#[derive(Clone, Copy)]
pub(super) struct Source {
    pub(super) document: &'static str,
    pub(super) pointer: &'static str,
}
impl Source {
    fn finding(self, path: &str, message: impl Into<String>) -> ValidationFinding {
        ValidationFinding {
            document: self.document.into(),
            pointer: self.pointer.into(),
            instance_path: path.into(),
            message: message.into(),
        }
    }
}
pub(super) struct Limits {
    pub(super) max_depth: usize,
    pub(super) max_errors: usize,
    pub(super) max_number_bytes: usize,
    pub(super) max_equality_steps: usize,
    pub(super) max_evaluation_steps: usize,
}
pub(super) struct Program {
    pub(super) limits: Limits,
    pub(super) roots: &'static [usize],
    pub(super) nodes: Vec<Node>,
}
pub(super) struct Node {
    pub(super) source: Source,
    pub(super) checks: Vec<Check>,
}
pub(super) struct Check {
    pub(super) source: Source,
    pub(super) instruction: Instruction,
}
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(super) enum JsonType {
    Null,
    Boolean,
    Integer,
    Number,
    String,
    Array,
    Object,
}
#[allow(dead_code)]
pub(super) enum CountTarget {
    String,
    Array,
    Object,
}
#[allow(dead_code)]
pub(super) enum Instruction {
    Always(bool),
    Type(&'static [JsonType]),
    Ref(usize),
    Properties(&'static [(&'static str, usize)]),
    AdditionalProperties {
        declared: &'static [&'static str],
        target: usize,
    },
    Required(&'static [&'static str]),
    Items {
        target: usize,
        start: usize,
    },
    PrefixItems(&'static [usize]),
    AllOf(&'static [usize]),
    AnyOf(&'static [usize]),
    OneOf(&'static [usize]),
    Not(usize),
    Bound {
        value: &'static str,
        maximum: bool,
        exclusive: bool,
    },
    MultipleOf(&'static str),
    Count {
        value: &'static str,
        maximum: bool,
        target: CountTarget,
    },
    Enum(Vec<JsonValue>),
    Const(JsonValue),
    UniqueItems,
    Pattern(PatternProgram),
}
fn child(path: &str, token: &str) -> String {
    format!("{}/{}", path, token.replace('~', "~0").replace('/', "~1"))
}
fn number_token(value: &JsonValue) -> Option<&str> {
    if let N::Value(J::Number(n)) = value {
        Some(n.as_str())
    } else {
        None
    }
}

/// Validate a selected root node index against an exact JSON value.
///
/// Only node indices in the compiled program's selected roots are accepted.
/// Each call receives fresh finite depth, visit, pattern, numeric-work and
/// structural-equality budgets. Resource exhaustion and unproductive recursion
/// return `EvaluationFailure`, including inside `not`, `anyOf` and `oneOf`.
/// Numbers are compared mathematically, never using floating point or spelling.
pub fn validate(root_node_index: usize, instance: &JsonValue) -> ValidationOutcome {
    ValidationSession::new().validate_at(root_node_index, instance, "")
}

fn selected_source(root: usize, path: &str) -> Result<Source, ValidationFinding> {
    let program = super::program();
    let source = program.nodes.get(root).map_or(
        Source {
            document: "",
            pointer: "",
        },
        |node| node.source,
    );
    if program.roots.contains(&root) {
        Ok(source)
    } else {
        Err(source.finding(path, "requested node is not a selected validation root"))
    }
}

pub(crate) struct ValidationSession {
    out: Vec<ValidationFinding>,
    remaining_steps: usize,
    remaining_equality: usize,
    remaining_numeric: usize,
    numbers: HashMap<String, Rc<Exact>>,
    active: HashSet<(usize, usize)>,
    depth: usize,
}
impl ValidationSession {
    pub(crate) fn new() -> Self {
        let program = super::program();
        Self {
            out: Vec::new(),
            remaining_steps: program.limits.max_evaluation_steps,
            remaining_equality: program.limits.max_equality_steps,
            remaining_numeric: program.limits.max_evaluation_steps,
            numbers: HashMap::new(),
            active: HashSet::new(),
            depth: 0,
        }
    }
    pub(crate) fn validate_at(
        &mut self,
        root: usize,
        instance: &JsonValue,
        path: &str,
    ) -> ValidationOutcome {
        self.out.clear();
        if let Err(finding) = selected_source(root, path) {
            return ValidationOutcome::EvaluationFailure(finding);
        }
        match self.eval(root, instance, path) {
            Ok(true) => ValidationOutcome::Valid,
            Ok(false) => ValidationOutcome::Invalid(std::mem::take(&mut self.out)),
            Err(e) => ValidationOutcome::EvaluationFailure(e),
        }
    }
    fn step(&mut self, source: Source, path: &str) -> Result<(), ValidationFinding> {
        if self.remaining_steps == 0 {
            return Err(source.finding(
                path,
                format!(
                    "schema evaluation exceeds {} evaluation steps",
                    super::program().limits.max_evaluation_steps
                ),
            ));
        }
        self.remaining_steps -= 1;
        Ok(())
    }
    fn emit(&mut self, source: Source, path: &str, message: impl Into<String>) {
        let cap = super::program().limits.max_errors;
        if cap == 0 || self.out.len() < cap {
            self.out.push(source.finding(path, message));
        }
    }
    fn number(&mut self, token: &str) -> Result<Rc<Exact>, String> {
        if let Some(number) = self.numbers.get(token) {
            return Ok(Rc::clone(number));
        }
        let number = Rc::new(Exact::parse(
            token,
            super::program().limits.max_number_bytes,
        )?);
        self.numbers.insert(token.into(), Rc::clone(&number));
        Ok(number)
    }
    /// Codec literal selection shares the same equality and numeric budgets.
    #[allow(dead_code)]
    pub(crate) fn equal_at(
        &mut self,
        root: usize,
        a: &JsonValue,
        b: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let source = selected_source(root, path)?;
        self.compare(a, b).map_err(|e| source.finding(path, e))
    }
    fn compare(&mut self, a: &JsonValue, b: &JsonValue) -> Result<bool, String> {
        let limits = &super::program().limits;
        let exhausted = || {
            format!(
                "equality evaluation exceeds {} node comparisons",
                limits.max_equality_steps
            )
        };
        let mut pending = vec![(a, b, 0)];
        while let Some((a, b, depth)) = pending.pop() {
            if self.remaining_equality == 0 {
                return Err(exhausted());
            }
            self.remaining_equality -= 1;
            if depth > limits.max_depth {
                return Err(format!(
                    "equality evaluation depth exceeds {}",
                    limits.max_depth
                ));
            }
            let equal = match (a, b) {
                (N::Null, N::Null) => true,
                (N::Value(J::Bool(a)), N::Value(J::Bool(b))) => a == b,
                (N::Value(J::String(a)), N::Value(J::String(b))) => a == b,
                (N::Value(J::Number(a)), N::Value(J::Number(b))) => self
                    .number(a.as_str())?
                    .compare(self.number(b.as_str())?.as_ref())
                    .is_eq(),
                (N::Value(J::Array(a)), N::Value(J::Array(b))) => {
                    if a.len() != b.len() {
                        return Ok(false);
                    }
                    if a.len().saturating_add(pending.len()) > self.remaining_equality {
                        return Err(exhausted());
                    }
                    pending.extend(a.iter().zip(b).rev().map(|(a, b)| (a, b, depth + 1)));
                    true
                }
                (N::Value(J::Object(a)), N::Value(J::Object(b))) => {
                    if a.len() != b.len() {
                        return Ok(false);
                    }
                    if a.len().saturating_add(pending.len()) > self.remaining_equality {
                        return Err(exhausted());
                    }
                    for (key, a) in a.iter().rev() {
                        let Some(b) = b.get(key) else {
                            return Ok(false);
                        };
                        pending.push((a, b, depth + 1));
                    }
                    true
                }
                _ => false,
            };
            if !equal {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn eval(
        &mut self,
        index: usize,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let node = &super::program().nodes[index];
        self.step(node.source, path)?;
        if self.depth >= super::program().limits.max_depth {
            return Err(node.source.finding(
                path,
                format!(
                    "schema evaluation depth exceeds {}",
                    super::program().limits.max_depth
                ),
            ));
        }
        let identity = (index, instance as *const JsonValue as usize);
        if !self.active.insert(identity) {
            return Err(node.source.finding(path,"recursive schema evaluation revisited the same schema and instance without progress"));
        }
        self.depth += 1;
        let result = self.run(node, instance, path);
        self.depth -= 1;
        self.active.remove(&identity);
        result
    }
    fn trial(
        &mut self,
        index: usize,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let findings = std::mem::take(&mut self.out);
        let result = self.eval(index, instance, path);
        self.out = findings;
        result
    }
    fn run(
        &mut self,
        node: &Node,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        use Instruction as I;
        let mut valid = true;
        for check in &node.checks {
            let at = check.source;
            self.step(at, path)?;
            // One shared result slot avoids retaining every opcode's error and
            // iterator temporaries on each recursive call in debug builds.
            let result = match &check.instruction {
                I::Ref(target) => self.eval(*target, instance, path),
                I::Properties(properties) => self.properties(properties, at, instance, path),
                I::AdditionalProperties { declared, target } => {
                    self.additional_properties(declared, *target, at, instance, path)
                }
                I::Items { target, start } => self.items(*target, *start, at, instance, path),
                I::PrefixItems(targets) => self.prefix_items(targets, at, instance, path),
                I::AllOf(targets) => self.all_of(targets, at, instance, path),
                I::AnyOf(targets) => self.union(targets, false, at, instance, path),
                I::OneOf(targets) => self.union(targets, true, at, instance, path),
                I::Not(target) => self.negated(*target, at, instance, path),
                _ => self.scalar(check, instance, path),
            };
            valid &= result?;
        }
        Ok(valid)
    }

    #[inline(never)]
    fn properties(
        &mut self,
        properties: &[(&str, usize)],
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let mut ok = true;
        if let N::Value(J::Object(object)) = instance {
            for (name, target) in properties {
                self.step(at, path)?;
                if let Some(value) = object.get(*name) {
                    ok &= self.eval(*target, value, &child(path, name))?;
                }
            }
        }
        Ok(ok)
    }

    #[inline(never)]
    fn additional_properties(
        &mut self,
        declared: &[&str],
        target: usize,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let mut ok = true;
        if let N::Value(J::Object(object)) = instance {
            for (name, value) in object {
                self.step(at, path)?;
                if !declared.contains(&name.as_str()) {
                    ok &= self.eval(target, value, &child(path, name))?;
                }
            }
        }
        Ok(ok)
    }

    #[inline(never)]
    fn items(
        &mut self,
        target: usize,
        start: usize,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let mut ok = true;
        if let N::Value(J::Array(array)) = instance {
            for (index, value) in array.iter().enumerate().skip(start) {
                self.step(at, path)?;
                ok &= self.eval(target, value, &child(path, &index.to_string()))?;
            }
        }
        Ok(ok)
    }

    #[inline(never)]
    fn prefix_items(
        &mut self,
        targets: &[usize],
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let mut ok = true;
        if let N::Value(J::Array(array)) = instance {
            for (index, (value, target)) in array.iter().zip(targets).enumerate() {
                self.step(at, path)?;
                ok &= self.eval(*target, value, &child(path, &index.to_string()))?;
            }
        }
        Ok(ok)
    }

    #[inline(never)]
    fn all_of(
        &mut self,
        targets: &[usize],
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let mut ok = true;
        for target in targets {
            self.step(at, path)?;
            ok &= self.eval(*target, instance, path)?;
        }
        Ok(ok)
    }

    #[inline(never)]
    fn union(
        &mut self,
        targets: &[usize],
        exclusive: bool,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let mut accepted = 0;
        for target in targets {
            self.step(at, path)?;
            if self.trial(*target, instance, path)? {
                accepted += 1;
            }
        }
        let ok = if exclusive {
            accepted == 1
        } else {
            accepted != 0
        };
        if !ok {
            self.emit(
                at,
                path,
                format!("composition matched {accepted} alternatives"),
            );
        }
        Ok(ok)
    }

    #[inline(never)]
    fn negated(
        &mut self,
        target: usize,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        let ok = !self.trial(target, instance, path)?;
        if !ok {
            self.emit(at, path, "instance matches the negated schema");
        }
        Ok(ok)
    }

    // Keep formatting, exact arithmetic and pattern work off recursive frames.
    // Their debug-build temporaries otherwise consume stack at every schema hop.
    #[inline(never)]
    fn scalar(
        &mut self,
        check: &Check,
        instance: &JsonValue,
        path: &str,
    ) -> Result<bool, ValidationFinding> {
        use Instruction as I;
        let at = check.source;
        Ok(match &check.instruction {
            I::Always(value) => {
                if !value {
                    self.emit(at, path, "value is rejected by a false schema");
                }
                *value
            }
            I::Type(types) => {
                let kind = match instance {
                    N::Null => JsonType::Null,
                    N::Value(J::Bool(_)) => JsonType::Boolean,
                    N::Value(J::Number(_)) => JsonType::Number,
                    N::Value(J::String(_)) => JsonType::String,
                    N::Value(J::Array(_)) => JsonType::Array,
                    N::Value(J::Object(_)) => JsonType::Object,
                };
                let ok = types.contains(&kind)
                    || (kind == JsonType::Number
                        && types.contains(&JsonType::Integer)
                        && self
                            .number(number_token(instance).expect("numeric kind"))
                            .map_err(|e| at.finding(path, e))?
                            .integral());
                if !ok {
                    self.emit(at, path, "instance does not match the declared type");
                }
                ok
            }
            I::Required(names) => {
                let mut ok = true;
                if let N::Value(J::Object(object)) = instance {
                    for name in *names {
                        self.step(at, path)?;
                        if !object.contains_key(*name) {
                            self.emit(at, path, format!("required property {name:?} is absent"));
                            ok = false;
                        }
                    }
                }
                ok
            }
            I::Bound {
                value,
                maximum,
                exclusive,
            } => {
                if let Some(token) = number_token(instance) {
                    let a = self.number(token).map_err(|e| at.finding(path, e))?;
                    let b = self.number(value).map_err(|e| at.finding(path, e))?;
                    let order = a.compare(&b);
                    let ok = if *maximum {
                        order.is_lt() || (order.is_eq() && !exclusive)
                    } else {
                        order.is_gt() || (order.is_eq() && !exclusive)
                    };
                    if !ok {
                        self.emit(at, path, format!("numeric value violates bound {value}"));
                    }
                    ok
                } else {
                    true
                }
            }
            I::MultipleOf(value) => {
                if let Some(token) = number_token(instance) {
                    let a = self.number(token).map_err(|e| at.finding(path, e))?;
                    let b = self.number(value).map_err(|e| at.finding(path, e))?;
                    let ok = a
                        .multiple_of(&b, &mut self.remaining_numeric)
                        .map_err(|e| at.finding(path, e))?;
                    if !ok {
                        self.emit(
                            at,
                            path,
                            format!("numeric value is not a multiple of {value}"),
                        );
                    }
                    ok
                } else {
                    true
                }
            }
            I::Count {
                value,
                maximum,
                target,
            } => {
                let count = match (target, instance) {
                    (CountTarget::String, N::Value(J::String(v))) => Some(v.chars().count()),
                    (CountTarget::Array, N::Value(J::Array(v))) => Some(v.len()),
                    (CountTarget::Object, N::Value(J::Object(v))) => Some(v.len()),
                    _ => None,
                };
                if let Some(count) = count {
                    // Machine cardinality is internal, not a source numeric operand.
                    let a =
                        Exact::parse(&count.to_string(), usize::MAX).expect("machine cardinality");
                    let b = self.number(value).map_err(|e| at.finding(path, e))?;
                    let order = a.compare(&b);
                    let ok = if *maximum {
                        !order.is_gt()
                    } else {
                        !order.is_lt()
                    };
                    if !ok {
                        self.emit(
                            at,
                            path,
                            format!("size {count} violates cardinality {value}"),
                        );
                    }
                    ok
                } else {
                    true
                }
            }
            I::Enum(values) => {
                let mut ok = false;
                for value in values {
                    self.step(at, path)?;
                    if self
                        .compare(instance, value)
                        .map_err(|e| at.finding(path, e))?
                    {
                        ok = true;
                        break;
                    }
                }
                if !ok {
                    self.emit(at, path, "instance does not equal any enum value");
                }
                ok
            }
            I::Const(value) => {
                let ok = self
                    .compare(instance, value)
                    .map_err(|e| at.finding(path, e))?;
                if !ok {
                    self.emit(at, path, "instance does not equal the const value");
                }
                ok
            }
            I::UniqueItems => {
                let mut ok = true;
                if let N::Value(J::Array(array)) = instance {
                    'items: for (index, value) in array.iter().enumerate() {
                        self.step(at, path)?;
                        for previous in &array[..index] {
                            self.step(at, path)?;
                            if self
                                .compare(value, previous)
                                .map_err(|e| at.finding(path, e))?
                            {
                                self.emit(at, path, "array contains equal items");
                                ok = false;
                                break 'items;
                            }
                        }
                    }
                }
                ok
            }
            I::Pattern(program) => {
                if let N::Value(J::String(text)) = instance {
                    let ok = pattern::is_match(program, text, &mut self.remaining_steps).map_err(
                        |()| {
                            at.finding(
                                path,
                                format!(
                                    "schema evaluation exceeds {} evaluation steps",
                                    super::program().limits.max_evaluation_steps
                                ),
                            )
                        },
                    )?;
                    if !ok {
                        self.emit(at, path, "string does not match `pattern`");
                    }
                    ok
                } else {
                    true
                }
            }
            I::Ref(_)
            | I::Properties(_)
            | I::AdditionalProperties { .. }
            | I::Items { .. }
            | I::PrefixItems(_)
            | I::AllOf(_)
            | I::AnyOf(_)
            | I::OneOf(_)
            | I::Not(_) => {
                return Err(at.finding(path, "recursive instruction reached the scalar evaluator"));
            }
        })
    }
}
