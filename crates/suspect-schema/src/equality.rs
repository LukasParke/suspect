//! One exact structural-equality algorithm over borrowed or owned JSON views.

use std::borrow::Cow;

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;
use suspect_low::{NodeRef, ValueKind};

use crate::Config;
use crate::number::ExactNumber;

type Entries<K, V> = Vec<(K, Option<V>)>;

/// Values provide their already parsed representation; equality never
/// serializes/reparses schemas or converts numbers through floating point.
pub(crate) trait EqualityValue<'a>: Copy {
    type Key;
    fn kind(self) -> ValueKind;
    fn number(self) -> &'a [u8];
    fn boolean(self) -> Option<bool>;
    fn text(self) -> Result<Cow<'a, [u8]>, String>;
    fn items(self) -> Vec<Self>;
    fn entries(self) -> Entries<Self::Key, Self>;
    fn key_text(key: Self::Key) -> Result<Cow<'a, [u8]>, String>;
}

fn decoded_text(node: NodeRef<'_>) -> Result<Cow<'_, [u8]>, String> {
    let text = node
        .try_decoded_scalar()
        .ok_or_else(|| "equality evaluation cannot decode malformed string text".to_owned())?;
    std::str::from_utf8(&text)
        .map_err(|_| "equality evaluation requires valid Unicode strings".to_owned())?;
    Ok(text)
}

impl<'a> EqualityValue<'a> for NodeRef<'a> {
    type Key = NodeRef<'a>;
    fn kind(self) -> ValueKind {
        NodeRef::kind(&self)
    }
    fn number(self) -> &'a [u8] {
        self.scalar_bytes()
    }
    fn boolean(self) -> Option<bool> {
        self.as_bool()
    }
    fn text(self) -> Result<Cow<'a, [u8]>, String> {
        decoded_text(self)
    }
    fn items(self) -> Vec<Self> {
        NodeRef::items(&self)
    }
    fn entries(self) -> Entries<Self::Key, Self> {
        NodeRef::entries(&self)
            .into_iter()
            .map(|entry| (entry.key_node, entry.value))
            .collect()
    }
    fn key_text(key: Self::Key) -> Result<Cow<'a, [u8]>, String> {
        decoded_text(key)
    }
}

impl<'a> EqualityValue<'a> for &'a Value {
    type Key = &'a str;
    fn kind(self) -> ValueKind {
        match self {
            Value::Null => ValueKind::Null,
            Value::Bool(_) => ValueKind::Bool,
            Value::Number(_) => ValueKind::Float,
            Value::String(_) => ValueKind::Str,
            Value::Array(_) => ValueKind::Array,
            Value::Object(_) => ValueKind::Object,
        }
    }
    fn number(self) -> &'a [u8] {
        self.as_number()
            .expect("numeric equality operand")
            .as_str()
            .as_bytes()
    }
    fn boolean(self) -> Option<bool> {
        self.as_bool()
    }
    fn text(self) -> Result<Cow<'a, [u8]>, String> {
        Ok(Cow::Borrowed(
            self.as_str().expect("string equality operand").as_bytes(),
        ))
    }
    fn items(self) -> Vec<Self> {
        self.as_array()
            .expect("array equality operand")
            .iter()
            .collect()
    }
    fn entries(self) -> Entries<Self::Key, Self> {
        self.as_object()
            .expect("object equality operand")
            .iter()
            .map(|(key, value)| (key.as_str(), Some(value)))
            .collect()
    }
    fn key_text(key: Self::Key) -> Result<Cow<'a, [u8]>, String> {
        Ok(Cow::Borrowed(key.as_bytes()))
    }
}

/// Per-call work allowance shared by all branches and equality keywords.
pub(crate) struct EqualityBudget {
    remaining: usize,
    cap: usize,
    max_depth: usize,
    max_number_bytes: usize,
}

impl EqualityBudget {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            remaining: config.max_equality_steps,
            cap: config.max_equality_steps,
            max_depth: config.max_depth,
            max_number_bytes: config.max_number_bytes,
        }
    }

    fn exhausted(&self) -> String {
        format!("equality evaluation exceeds {} node comparisons", self.cap)
    }

    /// Iterative structural equality: exact numbers, ordered arrays,
    /// unordered objects, decoded text, and noninvertible resource failures.
    pub(crate) fn compare<'a, 'b, A: EqualityValue<'a>, B: EqualityValue<'b>>(
        &mut self,
        a: A,
        b: B,
    ) -> Result<bool, String> {
        let mut pending = vec![(a, b, 0)];
        while let Some((a, b, depth)) = pending.pop() {
            if self.remaining == 0 {
                return Err(self.exhausted());
            }
            self.remaining -= 1;
            if depth > self.max_depth {
                return Err(format!(
                    "equality evaluation depth exceeds {}",
                    self.max_depth
                ));
            }
            let (ka, kb) = (a.kind(), b.kind());
            let equal = match (ka, kb) {
                (ValueKind::Int | ValueKind::Float, ValueKind::Int | ValueKind::Float) => {
                    let a = ExactNumber::parse(a.number(), self.max_number_bytes)
                        .map_err(|e| e.to_string())?;
                    let b = ExactNumber::parse(b.number(), self.max_number_bytes)
                        .map_err(|e| e.to_string())?;
                    a.cmp(&b).is_eq()
                }
                _ if ka != kb => false,
                (ValueKind::Null, ValueKind::Null) => true,
                (ValueKind::Bool, ValueKind::Bool) => a.boolean() == b.boolean(),
                (ValueKind::Str, ValueKind::Str) => a.text()? == b.text()?,
                (ValueKind::Array, ValueKind::Array) => {
                    let (ai, bi) = (a.items(), b.items());
                    if ai.len() != bi.len() {
                        return Ok(false);
                    }
                    if ai.len().saturating_add(pending.len()) > self.remaining {
                        return Err(self.exhausted());
                    }
                    pending.extend(ai.into_iter().zip(bi).rev().map(|(a, b)| (a, b, depth + 1)));
                    true
                }
                (ValueKind::Object, ValueKind::Object) => {
                    let (ae, be) = (a.entries(), b.entries());
                    if ae.len() != be.len() {
                        return Ok(false);
                    }
                    if ae.len().saturating_add(pending.len()) > self.remaining {
                        return Err(self.exhausted());
                    }
                    let mut map = FxHashMap::default();
                    for (key, value) in be {
                        let key = B::key_text(key)?;
                        if map.insert(key, value).is_some() {
                            return Err("equality evaluation cannot interpret duplicate decoded object keys".into());
                        }
                    }
                    let mut seen = FxHashSet::default();
                    for (key, value) in ae.into_iter().rev() {
                        let key = A::key_text(key)?;
                        if !seen.insert(key.clone()) {
                            return Err("equality evaluation cannot interpret duplicate decoded object keys".into());
                        }
                        match (value, map.get(&key)) {
                            (Some(a), Some(Some(b))) => pending.push((a, *b, depth + 1)),
                            _ => return Ok(false),
                        }
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
}
