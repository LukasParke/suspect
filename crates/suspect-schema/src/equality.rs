//! One exact structural-equality algorithm over borrowed or owned JSON views.

use std::borrow::Cow;
use std::hash::Hasher;

use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
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

/// Arrays up to this long are compared pair by pair by `uniqueItems`.
/// Below it the pairs cost no more than the fingerprints would.
const PAIRWISE_ITEMS: usize = 128;

/// Most members of one array item a fingerprint reads.
const FINGERPRINT_WIDTH: usize = 64;

const ARRAY_TAG: u8 = 4;
const OBJECT_TAG: u8 = 5;

/// Which earlier items each item of a `uniqueItems` array is compared
/// with; built by [`EqualityBudget::unique_scan`].
pub(crate) struct UniqueScan {
    /// One fingerprint per item; `None` when every pair is compared.
    prints: Option<Vec<u64>>,
    /// Fingerprint → the items already visited that carry it, in order.
    groups: FxHashMap<u64, Vec<usize>>,
}

impl UniqueScan {
    fn pairwise() -> Self {
        Self {
            prints: None,
            groups: FxHashMap::default(),
        }
    }

    /// The items before `index` that may equal it, in array order.
    pub(crate) fn earlier(&self, index: usize) -> Earlier<'_> {
        match &self.prints {
            None => Earlier::All(0..index),
            Some(prints) => Earlier::Group(
                self.groups
                    .get(&prints[index])
                    .map_or(&[][..], Vec::as_slice)
                    .iter(),
            ),
        }
    }

    /// Records that item `index` was visited.
    pub(crate) fn visited(&mut self, index: usize) {
        if let Some(prints) = &self.prints {
            self.groups.entry(prints[index]).or_default().push(index);
        }
    }
}

/// The indices [`UniqueScan::earlier`] yields.
pub(crate) enum Earlier<'s> {
    All(std::ops::Range<usize>),
    Group(std::slice::Iter<'s, usize>),
}

impl Iterator for Earlier<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        match self {
            Self::All(range) => range.next(),
            Self::Group(indices) => indices.next().copied(),
        }
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

    /// Spends one step of the allowance.
    fn spend(&mut self) -> Result<(), String> {
        if self.remaining == 0 {
            return Err(self.exhausted());
        }
        self.remaining -= 1;
        Ok(())
    }

    /// Plans the comparisons `uniqueItems` needs for `items`.
    ///
    /// A short array is compared pair by pair. A longer one is grouped by
    /// fingerprint first, so only items that may be equal are compared and
    /// the work grows with the array instead of its square. Reading a
    /// fingerprint spends the same allowance as a comparison. An item whose
    /// fingerprint cannot be read (a numeric operand over the limit,
    /// undecodable text) sends the whole array back to pairwise comparison,
    /// which reports that failure as it always has.
    pub(crate) fn unique_scan<'a, V: EqualityValue<'a>>(
        &mut self,
        items: &[V],
    ) -> Result<UniqueScan, String> {
        if items.len() <= PAIRWISE_ITEMS {
            return Ok(UniqueScan::pairwise());
        }
        let mut prints = Vec::with_capacity(items.len());
        for item in items {
            match self.fingerprint(*item)? {
                Some(print) => prints.push(print),
                None => return Ok(UniqueScan::pairwise()),
            }
        }
        Ok(UniqueScan {
            prints: Some(prints),
            groups: FxHashMap::default(),
        })
    }

    /// A hash equal values share: of a scalar's value, or of a container's
    /// length and its first [`FINGERPRINT_WIDTH`] members read one level
    /// deep. `None` when the value cannot be read.
    fn fingerprint<'a, V: EqualityValue<'a>>(&mut self, value: V) -> Result<Option<u64>, String> {
        let mut hasher = FxHasher::default();
        match value.kind() {
            ValueKind::Array => {
                self.spend()?;
                let items = value.items();
                hasher.write_u8(ARRAY_TAG);
                hasher.write_usize(items.len());
                for item in items.into_iter().take(FINGERPRINT_WIDTH) {
                    match self.shallow_print(item)? {
                        Some(print) => hasher.write_u64(print),
                        None => return Ok(None),
                    }
                }
            }
            ValueKind::Object => {
                self.spend()?;
                let entries = value.entries();
                hasher.write_u8(OBJECT_TAG);
                hasher.write_usize(entries.len());
                if entries.len() <= FINGERPRINT_WIDTH {
                    // Members are unordered: their hashes are summed.
                    let mut members = 0u64;
                    let mut keys = FxHashSet::default();
                    for (key, member) in entries {
                        let Ok(key) = V::key_text(key) else {
                            return Ok(None);
                        };
                        let mut entry = FxHasher::default();
                        entry.write(&key);
                        entry.write_u8(0xff);
                        if let Some(member) = member {
                            match self.shallow_print(member)? {
                                Some(print) => entry.write_u64(print),
                                None => return Ok(None),
                            }
                        }
                        // Comparison refuses duplicate keys; so does this.
                        if !keys.insert(key) {
                            return Ok(None);
                        }
                        members = members.wrapping_add(entry.finish());
                    }
                    hasher.write_u64(members);
                }
            }
            _ => return self.shallow_print(value),
        }
        Ok(Some(hasher.finish()))
    }

    /// The fingerprint of a scalar, or of a container by its length alone.
    fn shallow_print<'a, V: EqualityValue<'a>>(&mut self, value: V) -> Result<Option<u64>, String> {
        self.spend()?;
        let mut hasher = FxHasher::default();
        match value.kind() {
            ValueKind::Null => hasher.write_u8(0),
            ValueKind::Bool => match value.boolean() {
                Some(flag) => {
                    hasher.write_u8(1);
                    hasher.write_u8(u8::from(flag));
                }
                None => return Ok(None),
            },
            ValueKind::Int | ValueKind::Float => {
                let Ok(number) = ExactNumber::parse(value.number(), self.max_number_bytes) else {
                    return Ok(None);
                };
                hasher.write_u8(2);
                number.hash_value(&mut hasher);
            }
            ValueKind::Str => {
                let Ok(text) = value.text() else {
                    return Ok(None);
                };
                hasher.write_u8(3);
                hasher.write(&text);
            }
            ValueKind::Array => {
                hasher.write_u8(ARRAY_TAG);
                hasher.write_usize(value.items().len());
            }
            ValueKind::Object => {
                hasher.write_u8(OBJECT_TAG);
                hasher.write_usize(value.entries().len());
            }
        }
        Ok(Some(hasher.finish()))
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
