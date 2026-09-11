//! V2 scoped evaluated locations. The v1 runtime remains a frozen emission asset.
use super::number::Exact;
use super::pattern::{self, PatternProgram};
use crate::{JsonNonNullValue as J, JsonValue, Nullable as N};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

/// Original schema/keyword and decoded instance location of a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationFinding {
    pub document: String,
    pub pointer: String,
    pub instance_path: String,
    pub message: String,
}
/// Resource failure is unknown validity, not an ordinary mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    Valid,
    Invalid(Vec<ValidationFinding>),
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
    #[inline(never)]
    fn failure(self, path: &str, message: impl Into<String>) -> Failure {
        Box::new(self.finding(path, message))
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
pub(super) struct CountLimit {
    pub(super) source: Source,
    pub(super) token: &'static str,
}
pub(super) struct Dependency {
    pub(super) source: Source,
    pub(super) trigger: &'static str,
    pub(super) required: &'static [&'static str],
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
    If {
        condition: usize,
        then_target: Option<usize>,
        else_target: Option<usize>,
    },
    DependentRequired(&'static [Dependency]),
    DependentSchemas(&'static [(&'static str, usize)]),
    Contains {
        target: usize,
        minimum: Option<CountLimit>,
        maximum: Option<CountLimit>,
    },
    PatternProperties(&'static [(&'static str, PatternProgram, usize)]),
    AdditionalPropertiesWithPatterns {
        declared: &'static [&'static str],
        target: usize,
    },
    PropertyNames(usize),
    UnevaluatedProperties(usize),
    UnevaluatedItems(usize),
}

#[derive(Default)]
struct Annotations {
    properties: BTreeSet<String>,
    items: BTreeSet<usize>,
}
struct Evaluated {
    valid: bool,
    annotations: Annotations,
}
// Heap-owned outcomes keep recursive debug frames bounded independently of the
// instruction set and diagnostic payload size. No annotations are shared.
type Failure = Box<ValidationFinding>;
type Evaluation = Result<Box<Evaluated>, Failure>;
impl Evaluated {
    fn plain(valid: bool) -> Box<Self> {
        Box::new(Self {
            valid,
            annotations: Annotations::default(),
        })
    }
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

/// Validate one selected node with fresh budgets and fresh annotation scopes.
/// Every anyOf/oneOf/contains candidate is evaluated; only if's unselected
/// branch is skipped. A failed or negated schema exports no evaluated locations.
pub fn validate(root_node_index: usize, instance: &JsonValue) -> ValidationOutcome {
    ValidationSession::new().validate_at(root_node_index, instance, "")
}
fn selected_source(root: usize, path: &str) -> Result<Source, ValidationFinding> {
    let p = super::program();
    let source = p.nodes.get(root).map_or(
        Source {
            document: "",
            pointer: "",
        },
        |n| n.source,
    );
    if p.roots.contains(&root) {
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
    active: HashSet<(usize, *const JsonValue)>,
    depth: usize,
}
impl ValidationSession {
    pub(crate) fn new() -> Self {
        let p = super::program();
        Self {
            out: Vec::new(),
            remaining_steps: p.limits.max_evaluation_steps,
            remaining_equality: p.limits.max_equality_steps,
            remaining_numeric: p.limits.max_evaluation_steps,
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
            Ok(result) if result.valid => ValidationOutcome::Valid,
            Ok(_) => ValidationOutcome::Invalid(std::mem::take(&mut self.out)),
            Err(e) => ValidationOutcome::EvaluationFailure(*e),
        }
    }
    fn step(&mut self, source: Source, path: &str) -> Result<(), Failure> {
        if self.remaining_steps == 0 {
            return Err(source.failure(
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
    fn merge(
        &mut self,
        into: &mut Annotations,
        other: Annotations,
        source: Source,
        path: &str,
    ) -> Result<(), Failure> {
        for name in other.properties {
            self.step(source, path)?;
            into.properties.insert(name);
        }
        for index in other.items {
            self.step(source, path)?;
            into.items.insert(index);
        }
        Ok(())
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
    #[inline(never)]
    fn enter(&mut self, index: usize, instance: &JsonValue, path: &str) -> Result<(), Failure> {
        let source = super::program().nodes[index].source;
        self.step(source, path)?;
        if self.depth >= super::program().limits.max_depth {
            return Err(source.failure(
                path,
                format!(
                    "schema evaluation depth exceeds {}",
                    super::program().limits.max_depth
                ),
            ));
        }
        if !self.active.insert((index, std::ptr::from_ref(instance))) {
            return Err(source.failure(path,"recursive schema evaluation revisited the same schema and instance without progress"));
        }
        self.depth += 1;
        Ok(())
    }
    fn eval(&mut self, index: usize, instance: &JsonValue, path: &str) -> Evaluation {
        let node = &super::program().nodes[index];
        self.enter(index, instance, path)?;
        let identity = (index, std::ptr::from_ref(instance));
        let result = self.run(node, instance, path);
        self.depth -= 1;
        self.active.remove(&identity);
        result
    }
    fn trial(&mut self, index: usize, instance: &JsonValue, path: &str) -> Evaluation {
        let findings = std::mem::take(&mut self.out);
        let result = self.eval(index, instance, path);
        self.out = findings;
        result
    }
    fn run(&mut self, node: &Node, instance: &JsonValue, path: &str) -> Evaluation {
        use Instruction as I;
        let mut valid = true;
        let mut annotations = Annotations::default();
        for check in &node.checks {
            let at = check.source;
            self.step(at, path)?;
            let result =
                match &check.instruction {
                    I::Ref(target) => self.eval(*target, instance, path),
                    I::Properties(properties) => self.properties(properties, at, instance, path),
                    I::AdditionalProperties { declared, target } => self
                        .additional_properties(node, declared, *target, false, at, instance, path),
                    I::AdditionalPropertiesWithPatterns { declared, target } => self
                        .additional_properties(node, declared, *target, true, at, instance, path),
                    I::Items { target, start } => self.items(*target, *start, at, instance, path),
                    I::PrefixItems(targets) => self.prefix_items(targets, at, instance, path),
                    I::AllOf(targets) => self.all_of(targets, at, instance, path),
                    I::AnyOf(targets) => self.union(targets, false, at, instance, path),
                    I::OneOf(targets) => self.union(targets, true, at, instance, path),
                    I::Not(target) => self.negated(*target, at, instance, path),
                    I::If {
                        condition,
                        then_target,
                        else_target,
                    } => self.conditional(
                        *condition,
                        *then_target,
                        *else_target,
                        &mut annotations,
                        at,
                        instance,
                        path,
                    ),
                    I::DependentRequired(dependencies) => {
                        self.dependent_required(dependencies, at, instance, path)
                    }
                    I::DependentSchemas(dependencies) => {
                        self.dependent_schemas(dependencies, at, instance, path)
                    }
                    I::Contains {
                        target,
                        minimum,
                        maximum,
                    } => self.contains(
                        *target,
                        minimum.as_ref(),
                        maximum.as_ref(),
                        &mut annotations,
                        at,
                        instance,
                        path,
                    ),
                    I::PatternProperties(patterns) => {
                        self.pattern_properties(patterns, at, instance, path)
                    }
                    I::PropertyNames(target) => self.property_names(*target, at, instance, path),
                    I::UnevaluatedProperties(target) => {
                        self.unevaluated_properties(*target, &annotations, at, instance, path)
                    }
                    I::UnevaluatedItems(target) => {
                        self.unevaluated_items(*target, &annotations, at, instance, path)
                    }
                    _ => self.scalar(check, instance, path).map(Evaluated::plain),
                }?;
            valid &= result.valid;
            if result.valid {
                self.merge(&mut annotations, result.annotations, at, path)?;
            }
        }
        Ok(Box::new(Evaluated {
            valid,
            annotations: if valid {
                annotations
            } else {
                Annotations::default()
            },
        }))
    }
    #[inline(never)]
    fn properties(
        &mut self,
        properties: &[(&str, usize)],
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut result = Evaluated::plain(true);
        if let N::Value(J::Object(object)) = instance {
            for (name, target) in properties {
                self.step(at, path)?;
                if let Some(value) = object.get(*name) {
                    result.valid &= self.eval(*target, value, &child(path, name))?.valid;
                    result.annotations.properties.insert((*name).into());
                }
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn additional_properties(
        &mut self,
        node: &Node,
        declared: &[&str],
        target: usize,
        with_patterns: bool,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut result = Evaluated::plain(true);
        let patterns = if with_patterns {
            node.checks
                .iter()
                .find_map(|check| {
                    if let Instruction::PatternProperties(patterns) = &check.instruction {
                        Some(*patterns)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    at.failure(
                        path,
                        "pattern-aware additionalProperties has no checked adjacent patterns",
                    )
                })?
        } else {
            &[]
        };
        if let N::Value(J::Object(object)) = instance {
            'members: for (name, value) in object {
                self.step(at, path)?;
                if declared.contains(&name.as_str()) {
                    continue;
                }
                for (_, pattern, _) in patterns {
                    self.step(at, path)?;
                    if self.pattern(pattern, name, at, path)? {
                        continue 'members;
                    }
                }
                result.valid &= self.eval(target, value, &child(path, name))?.valid;
                result.annotations.properties.insert(name.clone());
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn items(
        &mut self,
        target: usize,
        start: usize,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut result = Evaluated::plain(true);
        if let N::Value(J::Array(array)) = instance {
            for (index, value) in array.iter().enumerate().skip(start) {
                self.step(at, path)?;
                result.valid &= self
                    .eval(target, value, &child(path, &index.to_string()))?
                    .valid;
                result.annotations.items.insert(index);
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn prefix_items(
        &mut self,
        targets: &[usize],
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut result = Evaluated::plain(true);
        if let N::Value(J::Array(array)) = instance {
            for (index, (value, target)) in array.iter().zip(targets).enumerate() {
                self.step(at, path)?;
                result.valid &= self
                    .eval(*target, value, &child(path, &index.to_string()))?
                    .valid;
                result.annotations.items.insert(index);
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn all_of(
        &mut self,
        targets: &[usize],
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut result = Evaluated::plain(true);
        let mut passing = Vec::new();
        for target in targets {
            self.step(at, path)?;
            let branch = self.eval(*target, instance, path)?;
            result.valid &= branch.valid;
            if branch.valid {
                passing.push(branch.annotations);
            }
        }
        if result.valid {
            for annotations in passing {
                self.merge(&mut result.annotations, annotations, at, path)?;
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn union(
        &mut self,
        targets: &[usize],
        exclusive: bool,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut accepted = 0;
        let mut passing = Vec::new();
        for target in targets {
            self.step(at, path)?;
            let branch = self.trial(*target, instance, path)?;
            if branch.valid {
                accepted += 1;
                passing.push(branch.annotations);
            }
        }
        let mut result = Evaluated::plain(if exclusive {
            accepted == 1
        } else {
            accepted != 0
        });
        if result.valid {
            for annotations in passing {
                self.merge(&mut result.annotations, annotations, at, path)?;
            }
        } else {
            self.emit(
                at,
                path,
                format!("composition matched {accepted} alternatives"),
            );
        }
        Ok(result)
    }
    #[inline(never)]
    fn negated(
        &mut self,
        target: usize,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let valid = !self.trial(target, instance, path)?.valid;
        if !valid {
            self.emit(at, path, "instance matches the negated schema");
        }
        Ok(Evaluated::plain(valid))
    }
    #[inline(never)]
    fn conditional(
        &mut self,
        condition: usize,
        then_target: Option<usize>,
        else_target: Option<usize>,
        local: &mut Annotations,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let condition = self.trial(condition, instance, path)?;
        let selected = if condition.valid {
            then_target
        } else {
            else_target
        };
        // A successful if contributes independently of a failed then. Neither
        // the condition nor the caller's sets seed the selected branch's scope.
        if condition.valid {
            self.merge(local, condition.annotations, at, path)?;
        }
        match selected {
            Some(target) => self.eval(target, instance, path),
            None => Ok(Evaluated::plain(true)),
        }
    }
    #[inline(never)]
    fn dependent_required(
        &mut self,
        dependencies: &[Dependency],
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut valid = true;
        if let N::Value(J::Object(object)) = instance {
            for dependency in dependencies {
                self.step(at, path)?;
                if object.contains_key(dependency.trigger) {
                    for name in dependency.required {
                        self.step(at, path)?;
                        if !object.contains_key(*name) {
                            valid = false;
                            self.emit(
                                dependency.source,
                                path,
                                format!(
                                    "required property {name:?} is absent while {:?} is present",
                                    dependency.trigger
                                ),
                            );
                        }
                    }
                }
            }
        }
        Ok(Evaluated::plain(valid))
    }
    #[inline(never)]
    fn dependent_schemas(
        &mut self,
        dependencies: &[(&str, usize)],
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut result = Evaluated::plain(true);
        let mut passing = Vec::new();
        if let N::Value(J::Object(object)) = instance {
            for (name, target) in dependencies {
                self.step(at, path)?;
                if object.contains_key(*name) {
                    let branch = self.eval(*target, instance, path)?;
                    result.valid &= branch.valid;
                    if branch.valid {
                        passing.push(branch.annotations);
                    }
                }
            }
        }
        if result.valid {
            for annotations in passing {
                self.merge(&mut result.annotations, annotations, at, path)?;
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn contains(
        &mut self,
        target: usize,
        minimum: Option<&CountLimit>,
        maximum: Option<&CountLimit>,
        local: &mut Annotations,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let N::Value(J::Array(array)) = instance else {
            return Ok(Evaluated::plain(true));
        };
        let mut matched = 0usize;
        let mut annotations = Annotations::default();
        for (index, value) in array.iter().enumerate() {
            self.step(at, path)?;
            if self
                .trial(target, value, &child(path, &index.to_string()))?
                .valid
            {
                matched += 1;
                annotations.items.insert(index);
            }
        }
        let count = Exact::parse(&matched.to_string(), usize::MAX).expect("machine cardinality");
        let zero = Exact::parse("0", usize::MAX).expect("machine cardinality");
        let lower = minimum
            .map(|bound| {
                self.number(bound.token)
                    .map_err(|e| bound.source.failure(path, e))
            })
            .transpose()?;
        // contains succeeds as an annotator independently of min/maxContains.
        if matched != 0 || lower.as_ref().is_some_and(|n| n.compare(&zero).is_eq()) {
            self.merge(local, annotations, at, path)?;
        }
        let minimum_ok = lower
            .as_ref()
            .map_or(matched != 0, |n| !count.compare(n).is_lt());
        let maximum_ok = if let Some(bound) = maximum {
            !count
                .compare(
                    self.number(bound.token)
                        .map_err(|e| bound.source.failure(path, e))?
                        .as_ref(),
                )
                .is_gt()
        } else {
            true
        };
        if !minimum_ok {
            self.emit(
                minimum.map_or(at, |b| b.source),
                path,
                format!("array has {matched} contains matches, fewer than required"),
            );
        }
        if !maximum_ok {
            self.emit(
                maximum.expect("explicit maximum failed").source,
                path,
                format!("array has {matched} contains matches, more than allowed"),
            );
        }
        Ok(Evaluated::plain(minimum_ok && maximum_ok))
    }
    fn pattern(
        &mut self,
        program: &PatternProgram,
        text: &str,
        at: Source,
        path: &str,
    ) -> Result<bool, Failure> {
        pattern::is_match(program, text, &mut self.remaining_steps).map_err(|()| {
            at.failure(
                path,
                format!(
                    "schema evaluation exceeds {} evaluation steps",
                    super::program().limits.max_evaluation_steps
                ),
            )
        })
    }
    #[inline(never)]
    fn pattern_properties(
        &mut self,
        patterns: &[(&str, PatternProgram, usize)],
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut result = Evaluated::plain(true);
        if let N::Value(J::Object(object)) = instance {
            for (name, value) in object {
                self.step(at, path)?;
                for (_, pattern, target) in patterns {
                    self.step(at, path)?;
                    if self.pattern(pattern, name, at, path)? {
                        result.valid &= self.eval(*target, value, &child(path, name))?.valid;
                        result.annotations.properties.insert(name.clone());
                    }
                }
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn property_names(
        &mut self,
        target: usize,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut valid = true;
        if let N::Value(J::Object(object)) = instance {
            for name in object.keys() {
                self.step(at, path)?;
                let key = N::Value(J::String(name.clone()));
                valid &= self.eval(target, &key, &child(path, name))?.valid;
            }
        }
        Ok(Evaluated::plain(valid))
    }
    #[inline(never)]
    fn unevaluated_properties(
        &mut self,
        target: usize,
        local: &Annotations,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut result = Evaluated::plain(true);
        if let N::Value(J::Object(object)) = instance {
            for (name, value) in object {
                self.step(at, path)?;
                if !local.properties.contains(name) {
                    result.valid &= self.eval(target, value, &child(path, name))?.valid;
                    result.annotations.properties.insert(name.clone());
                }
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn unevaluated_items(
        &mut self,
        target: usize,
        local: &Annotations,
        at: Source,
        instance: &JsonValue,
        path: &str,
    ) -> Evaluation {
        let mut result = Evaluated::plain(true);
        if let N::Value(J::Array(array)) = instance {
            for (index, value) in array.iter().enumerate() {
                self.step(at, path)?;
                if !local.items.contains(&index) {
                    result.valid &= self
                        .eval(target, value, &child(path, &index.to_string()))?
                        .valid;
                    result.annotations.items.insert(index);
                }
            }
        }
        Ok(result)
    }
    // Keep exact arithmetic, pattern work and formatting off recursive frames.
    #[inline(never)]
    fn scalar(&mut self, check: &Check, instance: &JsonValue, path: &str) -> Result<bool, Failure> {
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
                            .map_err(|e| at.failure(path, e))?
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
                    let a = self.number(token).map_err(|e| at.failure(path, e))?;
                    let b = self.number(value).map_err(|e| at.failure(path, e))?;
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
                    let a = self.number(token).map_err(|e| at.failure(path, e))?;
                    let b = self.number(value).map_err(|e| at.failure(path, e))?;
                    let ok = a
                        .multiple_of(&b, &mut self.remaining_numeric)
                        .map_err(|e| at.failure(path, e))?;
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
                    let a =
                        Exact::parse(&count.to_string(), usize::MAX).expect("machine cardinality");
                    let b = self.number(value).map_err(|e| at.failure(path, e))?;
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
                        .map_err(|e| at.failure(path, e))?
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
                    .map_err(|e| at.failure(path, e))?;
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
                                .map_err(|e| at.failure(path, e))?
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
                    let ok = self.pattern(program, text, at, path)?;
                    if !ok {
                        self.emit(at, path, "string does not match `pattern`");
                    }
                    ok
                } else {
                    true
                }
            }
            _ => return Err(at.failure(path, "applicator reached the scalar evaluator")),
        })
    }
}
