//! Native constructions from accepted ExamplePlan values and retained Python
//! descriptors. This is a bounded documentation lowering, not schema inference.
//! In particular, property names never act as schema keywords and a union uses
//! the same first-valid-branch rule as the native decoder.

use serde_json::Value;
use suspect_ir::contract::SchemaId;
use suspect_schema::{OwnedCompiler, OwnedOutcome, OwnedSchema};

use super::HttpPlan;
use crate::python_models::{PyDecl, PyField, PyType};

const MAX_DEPTH: usize = 64;
const MAX_WORK: usize = 1_000_000;
const MAX_EXPRESSION_BYTES: usize = 65_536;
const MAX_INTEGER_DIGITS: usize = 4096;

pub(super) fn quote(text: &str) -> String {
    serde_json::to_string(text)
        .expect("Python string literal")
        .replace('\u{85}', "\\u0085")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

pub(super) fn indent(text: &str, width: usize) -> String {
    let prefix = " ".repeat(width);
    text.lines()
        .map(|line| format!("{prefix}{line}\n"))
        .collect()
}

#[derive(Debug, Clone)]
pub(super) struct Unavailable {
    pub code: &'static str,
    pub source: SchemaId,
    pub message: String,
}

impl Unavailable {
    fn new(source: &SchemaId, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            source: source.clone(),
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Construction {
    /// Statements needed for typed extra-property setters, before the expression.
    pub setup: Vec<String>,
    pub expression: String,
    pub branches: Vec<SchemaId>,
}

impl Construction {
    pub fn assign(&self, name: &str, annotation: &str) -> String {
        let mut code = self.prelude();
        code.push_str(&format!("{name}: {annotation} = {}\n", self.expression));
        code
    }

    pub fn prelude(&self) -> String {
        self.setup.iter().map(|line| format!("{line}\n")).collect()
    }

    pub fn returning(&self) -> String {
        format!("{}return {}\n", self.prelude(), self.expression)
    }
}

pub(super) struct Renderer<'a> {
    plan: &'a HttpPlan,
    validator: Result<OwnedSchema, Unavailable>,
    remaining: usize,
}

impl<'a> Renderer<'a> {
    pub fn new(plan: &'a HttpPlan) -> Self {
        // ExamplePlan retains accepted values, not its validator. Compile once
        // for this documentation render, with the codec's semantic policy and
        // the same source/model/branch roots. Never import or parse emitted code.
        let mut roots: Vec<_> = plan
            .codecs()
            .models()
            .declarations()
            .keys()
            .cloned()
            .collect();
        for declaration in plan.codecs().models().declarations().values() {
            declaration.collect_validation_roots(&mut roots);
        }
        roots.sort();
        roots.dedup();
        let roots = crate::schema_view::closure(plan.contract(), &roots);
        let compiler = OwnedCompiler::new(plan.config.codecs.schema.clone());
        let validator = if plan.codecs().validation_program().version
            == suspect_schema::OwnedProgram::V3_VERSION
        {
            compiler.compile_v3(plan.contract().clone(), &roots)
        } else {
            compiler.compile_v2(plan.contract().clone(), &roots)
        }
        .map_err(|errors| {
            let error = &errors[0];
            Unavailable::new(
                &error.source,
                "native-example-schema-compilation",
                &error.message,
            )
        });
        Self {
            plan,
            validator,
            remaining: MAX_WORK,
        }
    }

    fn charge(&mut self, source: &SchemaId, cost: usize) -> Result<(), Unavailable> {
        self.remaining = self.remaining.checked_sub(cost).ok_or_else(|| {
            Unavailable::new(
                source,
                "native-example-limit",
                "native example rendering exhausted its finite work budget",
            )
        })?;
        Ok(())
    }

    pub fn valid(&mut self, source: &SchemaId, value: &Value) -> Result<bool, Unavailable> {
        self.charge(source, 1)?;
        let validator = self.validator.as_ref().map_err(Clone::clone)?;
        match validator.validate(source, value) {
            OwnedOutcome::Valid => Ok(true),
            OwnedOutcome::Invalid(_) => Ok(false),
            OwnedOutcome::EvaluationFailure(finding) => Err(Unavailable::new(
                &finding.source,
                "native-example-evaluation-incomplete",
                finding.message,
            )),
        }
    }

    pub fn render(
        &mut self,
        source: &SchemaId,
        value: &Value,
        prefix: &str,
    ) -> Result<Construction, Unavailable> {
        if !self.valid(source, value)? {
            return Err(Unavailable::new(
                source,
                "native-example-schema-mismatch",
                "the example is not valid under the configured codec schema policy",
            ));
        }
        let mut construction = Construction {
            setup: Vec::new(),
            expression: String::new(),
            branches: Vec::new(),
        };
        let mut state = State {
            renderer: self,
            prefix,
            next: 0,
            construction: &mut construction,
        };
        let expression = state.named(source, value, 0)?;
        let size = expression.len() + construction.setup.iter().map(String::len).sum::<usize>();
        if size > MAX_EXPRESSION_BYTES {
            return Err(Unavailable::new(
                source,
                "native-example-limit",
                "one native construction exceeds 65536 source bytes",
            ));
        }
        self.charge(source, size)?;
        construction.expression = expression;
        Ok(construction)
    }

    /// Follow actual aliases to find an object suitable for a presence recipe.
    /// A union or nullable object needs its own branch-specific recipe.
    pub fn object(&self, source: &SchemaId) -> Option<(&'a SchemaId, &'a [PyField])> {
        let declarations = self.plan.codecs().models().declarations();
        let mut source = source;
        for _ in 0..MAX_DEPTH {
            let (id, declaration) = declarations.get_key_value(source)?;
            match declaration {
                PyDecl::Dataclass { fields, .. } => return Some((id, fields)),
                PyDecl::Alias(PyType::Named(target)) => source = target,
                _ => return None,
            }
        }
        None
    }
}

struct State<'r, 'a> {
    renderer: &'r mut Renderer<'a>,
    prefix: &'r str,
    next: usize,
    construction: &'r mut Construction,
}

impl State<'_, '_> {
    fn step(&mut self, source: &SchemaId, depth: usize) -> Result<(), Unavailable> {
        if depth > MAX_DEPTH {
            return Err(Unavailable::new(
                source,
                "native-example-limit",
                "native construction exceeds 64 descriptor/value nesting levels",
            ));
        }
        self.renderer.charge(source, 1)
    }

    fn mismatch(&self, source: &SchemaId) -> Unavailable {
        Unavailable::new(
            source,
            "native-example-representation",
            "accepted JSON has no supported construction for this native descriptor",
        )
    }

    fn named(
        &mut self,
        source: &SchemaId,
        value: &Value,
        depth: usize,
    ) -> Result<String, Unavailable> {
        self.step(source, depth)?;
        let declaration = self
            .renderer
            .plan
            .codecs()
            .models()
            .declarations()
            .get(source)
            .ok_or_else(|| self.mismatch(source))?;
        match declaration {
            PyDecl::Alias(ty) => self.ty(source, ty, value, depth + 1),
            PyDecl::Dataclass { fields, extras } => {
                let object = value.as_object().ok_or_else(|| self.mismatch(source))?;
                let mut arguments = Vec::new();
                for field in fields {
                    self.renderer.charge(source, 1 + field.name.len())?;
                    if field.fixed.is_some() {
                        // init=False fields are supplied by the dataclass, just
                        // as they are when the source codec decodes this value.
                        continue;
                    }
                    match object.get(&field.wire) {
                        Some(value) => arguments.push(format!(
                            "{}={}",
                            field.name,
                            self.ty(&field.source, &field.ty, value, depth + 1)?
                        )),
                        None if field.required => return Err(self.mismatch(&field.source)),
                        None => {} // Omission is UNSET, never None.
                    }
                }
                let model = &self.renderer.plan.symbols()[source];
                self.renderer.charge(source, model.len())?;
                let expression = collection(&format!("models.{model}("), ")", arguments);
                let extra_values: Vec<_> = object
                    .iter()
                    .filter(|(wire, _)| !fields.iter().any(|field| field.wire == **wire))
                    .collect();
                if extra_values.is_empty() {
                    return Ok(expression);
                }
                let extra = extras.as_ref().ok_or_else(|| self.mismatch(source))?;
                let name = format!("{}_extra_{}", self.prefix, self.next);
                self.next += 1;
                self.construction
                    .setup
                    .push(format!("{name} = {expression}"));
                for (wire, value) in extra_values {
                    self.renderer.charge(source, wire.len())?;
                    let value = self.ty(source, extra, value, depth + 1)?;
                    self.construction
                        .setup
                        .push(format!("{name}.set_extra({}, {value})", quote(wire)));
                }
                Ok(name)
            }
        }
    }

    fn ty(
        &mut self,
        source: &SchemaId,
        ty: &PyType,
        value: &Value,
        depth: usize,
    ) -> Result<String, Unavailable> {
        self.step(source, depth)?;
        match ty {
            PyType::Named(target) => self.named(target, value, depth + 1),
            PyType::Optional(inner) => self.ty(source, inner, value, depth + 1),
            PyType::Nullable(_) if value.is_null() => Ok("None".into()),
            PyType::Nullable(inner) => self.ty(source, inner, value, depth + 1),
            PyType::Primitive("int") => {
                self.renderer.charge(source, value.to_string().len())?;
                integer(value).ok_or_else(|| {
                    Unavailable::new(
                        source,
                        "native-example-integer-limit",
                        "integer construction needs an exact integral value of at most 4096 digits",
                    )
                })
            }
            PyType::Primitive("_json.JsonNumber") if value.is_number() => {
                self.renderer.charge(source, value.to_string().len())?;
                Ok(format!("JsonNumber({})", quote(&value.to_string())))
            }
            PyType::Primitive("str") if value.is_string() => {
                self.renderer
                    .charge(source, value.as_str().unwrap().len())?;
                Ok(quote(value.as_str().unwrap()))
            }
            PyType::Primitive("bool") if value.is_boolean() => Ok(if value == &Value::Bool(true) {
                "True"
            } else {
                "False"
            }
            .into()),
            PyType::Primitive("None" | "types.NoneType" | "type(None)") if value.is_null() => {
                Ok("None".into())
            }
            PyType::Literal(values) => {
                // Literal decoding returns the declared native literal. Integral
                // example spellings such as 1.0 must therefore produce int 1.
                for literal in values {
                    self.renderer.charge(source, 1)?;
                    let equal = if literal.is_number() && value.is_number() {
                        integer(literal)
                            .zip(integer(value))
                            .is_some_and(|(a, b)| a == b)
                    } else {
                        literal == value
                    };
                    if equal {
                        return self.json(source, literal, depth + 1);
                    }
                }
                Err(self.mismatch(source))
            }
            PyType::List(inner) => {
                let values = value.as_array().ok_or_else(|| self.mismatch(source))?;
                let items = values
                    .iter()
                    .map(|value| self.ty(source, inner, value, depth + 1))
                    .collect::<Result<_, _>>()?;
                Ok(collection("[", "]", items))
            }
            PyType::Map(inner) => {
                let values = value.as_object().ok_or_else(|| self.mismatch(source))?;
                let mut items = Vec::new();
                for (key, value) in values {
                    self.renderer.charge(source, key.len())?;
                    items.push(format!(
                        "{}: {}",
                        quote(key),
                        self.ty(source, inner, value, depth + 1)?
                    ));
                }
                Ok(collection("{", "}", items))
            }
            PyType::Union(alternatives) => {
                for (branch, ty) in alternatives {
                    if self.renderer.valid(branch, value)? {
                        self.construction.branches.push(branch.clone());
                        return self.ty(branch, ty, value, depth + 1);
                    }
                }
                Err(self.mismatch(source))
            }
            PyType::JsonValue => self.json(source, value, depth + 1),
            _ => Err(self.mismatch(source)),
        }
    }

    fn json(
        &mut self,
        source: &SchemaId,
        value: &Value,
        depth: usize,
    ) -> Result<String, Unavailable> {
        self.step(source, depth)?;
        match value {
            Value::Null => Ok("None".into()),
            Value::Bool(value) => Ok(if *value { "True" } else { "False" }.into()),
            Value::String(value) => {
                self.renderer.charge(source, value.len())?;
                Ok(quote(value))
            }
            Value::Number(number) => {
                let token = number.to_string();
                self.renderer.charge(source, token.len())?;
                // Exact JSON supports int as well as JsonNumber. Keep decimal,
                // exponent and negative-zero spelling; never round via float.
                if token != "-0"
                    && token.bytes().all(|b| b.is_ascii_digit() || b == b'-')
                    && token.len() <= MAX_INTEGER_DIGITS
                {
                    Ok(token)
                } else {
                    Ok(format!("JsonNumber({})", quote(&token)))
                }
            }
            Value::Array(values) => Ok(collection(
                "[",
                "]",
                values
                    .iter()
                    .map(|value| self.json(source, value, depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
            Value::Object(values) => {
                let mut items = Vec::new();
                for (key, value) in values {
                    self.renderer.charge(source, key.len())?;
                    items.push(format!(
                        "{}: {}",
                        quote(key),
                        self.json(source, value, depth + 1)?
                    ));
                }
                Ok(collection("{", "}", items))
            }
        }
    }
}

fn collection(open: &str, close: &str, items: Vec<String>) -> String {
    let flat = format!("{open}{}{close}", items.join(", "));
    if !flat.contains('\n') && flat.len() <= 88 {
        return flat;
    }
    format!(
        "{open}\n{}{close}",
        items
            .iter()
            .map(|item| indent(&format!("{item},"), 4))
            .collect::<String>()
    )
}

/// Expand only a bounded mathematical integer. No binary float, exponent-sized
/// allocation, dependency addition or global Python digit-limit change is used.
fn integer(value: &Value) -> Option<String> {
    let token = value.as_number()?.to_string();
    let negative = token.starts_with('-');
    let unsigned = token.trim_start_matches('-');
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let fraction = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let digits = mantissa.replace('.', "");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some("0".into());
    }
    let scale = exponent
        .parse::<i64>()
        .ok()?
        .checked_sub(i64::try_from(fraction).ok()?)?;
    let mut digits = if scale < 0 {
        let trim = usize::try_from(scale.unsigned_abs()).ok()?;
        if trim > digits.len()
            || !digits.as_bytes()[digits.len() - trim..]
                .iter()
                .all(|b| *b == b'0')
        {
            return None;
        }
        digits[..digits.len() - trim].to_owned()
    } else {
        let extra = usize::try_from(scale).ok()?;
        if digits.len().checked_add(extra)? > MAX_INTEGER_DIGITS {
            return None;
        }
        format!("{digits}{}", "0".repeat(extra))
    };
    if digits.len() > MAX_INTEGER_DIGITS {
        return None;
    }
    if negative {
        digits.insert(0, '-');
    }
    Some(digits)
}
