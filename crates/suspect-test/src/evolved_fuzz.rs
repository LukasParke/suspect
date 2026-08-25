//! Grammar-evolved, coverage-guided generative fuzzing.
//!
//! Unlike random mutation, this fuzzer learns the input grammar from
//! successful responses and uses response-shape coverage to guide
//! exploration toward untested behavior. Mutations producing novel response
//! shapes are kept as seeds; uninteresting ones are discarded.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use crate::fuzz::{MutantKind, ScalarField, mutant_value};

/// A response shape fingerprint: status + top-level keys + value types.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResponseShape {
    /// HTTP status code.
    pub status: u16,
    /// Top-level object keys sorted.
    pub keys: Vec<String>,
    /// Value-type signature of top-level fields (e.g. `name:string`).
    pub types: Vec<String>,
}

/// One seed in the corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seed {
    /// The input payload that produced this shape.
    pub payload: Value,
    /// The response shape it produced.
    pub shape: ResponseShape,
    /// How many rounds this seed has survived.
    pub age: u32,
}

/// Statistics for one fuzzing campaign.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CampaignStats {
    /// Total requests sent.
    pub requests: u32,
    /// Distinct response shapes discovered.
    pub shapes: u32,
    /// Crashes found (5xx or contract violations).
    pub crashes: u32,
    /// Seeds in the current corpus.
    pub corpus_size: u32,
    /// Rounds completed.
    pub rounds: u32,
}

/// The evolved fuzzer state.
#[derive(Debug, Default)]
pub struct EvolvedFuzzer {
    /// Seed corpus keyed by response shape.
    corpus: BTreeMap<ResponseShape, Seed>,
    /// All shapes ever seen.
    seen_shapes: BTreeSet<ResponseShape>,
    /// Fields available for mutation, from the schema.
    fields: Vec<ScalarField>,
    stats: CampaignStats,
}

/// Outcome of sending one fuzzed request.
#[derive(Debug, Clone)]
pub struct RequestOutcome {
    /// Status returned.
    pub status: u16,
    /// Parsed response body (if JSON).
    pub body: Option<Value>,
    /// Whether this counts as a crash (5xx).
    pub crash: bool,
}

impl EvolvedFuzzer {
    /// Creates a fuzzer seeded from schema scalar fields.
    #[must_use]
    pub fn new(fields: Vec<ScalarField>) -> Self {
        Self {
            fields,
            ..Self::default()
        }
    }

    /// Generates the next candidate payload.
    ///
    /// Strategy:
    /// 1. 50%: mutate a corpus seed (exploit known-interesting regions).
    /// 2. 50%: generate a fresh schema-derived mutant (explore new regions).
    #[must_use]
    pub fn next_payload(&self, rng: &mut impl rand::Rng) -> Value {
        if !self.corpus.is_empty() && rng.r#gen::<bool>() {
            // Exploit: pick a random seed, mutate one field
            let seeds: Vec<&Seed> = self.corpus.values().collect();
            let idx = rng.gen_range(0..seeds.len());
            let mut payload = seeds[idx].payload.clone();
            if !self.fields.is_empty() {
                let field_idx = rng.gen_range(0..self.fields.len());
                let field = &self.fields[field_idx];
                let kind = random_mutant_kind(rng);
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert(field.name.clone(), mutant_value(&field.schema, kind));
                }
            }
            payload
        } else {
            // Explore: fresh schema-derived payload with one mutant
            let mut obj = serde_json::Map::new();
            for field in &self.fields {
                let kind = if rng.r#gen::<f64>() < 0.15 {
                    random_mutant_kind(rng)
                } else {
                    MutantKind::Empty
                };
                obj.insert(field.name.clone(), mutant_value(&field.schema, kind));
            }
            Value::Object(obj)
        }
    }

    /// Records the outcome of one request, updating coverage and corpus.
    pub fn record(&mut self, payload: Value, outcome: &RequestOutcome) {
        self.stats.requests += 1;
        if outcome.crash {
            self.stats.crashes += 1;
        }

        let shape = fingerprint(outcome.status, outcome.body.as_ref());
        self.seen_shapes.insert(shape.clone());
        self.stats.shapes = self.seen_shapes.len() as u32;

        match self.corpus.get_mut(&shape) {
            Some(seed) => {
                seed.age += 1;
                // Replace old seeds with fresh inputs reaching the same shape
                if seed.age > 50 {
                    seed.payload = payload;
                    seed.age = 0;
                }
            }
            None => {
                // Novel shape: add to corpus
                self.corpus.insert(
                    shape,
                    Seed {
                        payload,
                        shape: fingerprint(outcome.status, outcome.body.as_ref()),
                        age: 0,
                    },
                );
            }
        }
        self.stats.corpus_size = self.corpus.len() as u32;
    }

    /// Marks one round complete.
    pub fn end_round(&mut self) {
        self.stats.rounds += 1;
    }

    /// Current campaign statistics.
    #[must_use]
    pub fn stats(&self) -> &CampaignStats {
        &self.stats
    }

    /// The discovered response-shape diversity.
    #[must_use]
    pub fn shape_coverage(&self) -> &BTreeSet<ResponseShape> {
        &self.seen_shapes
    }
}

/// Fingerprints a response into a comparable shape.
#[must_use]
pub fn fingerprint(status: u16, body: Option<&Value>) -> ResponseShape {
    let (keys, types) = match body {
        Some(Value::Object(map)) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            let types: Vec<String> = keys
                .iter()
                .map(|k| format!("{k}:{}", json_type(map.get(k).unwrap())))
                .collect();
            (keys, types)
        }
        Some(Value::Array(items)) => (
            vec![format!("[{}]", items.len())],
            vec![
                items
                    .first()
                    .map(|v| format!("item:{}", json_type(v)))
                    .unwrap_or_else(|| "item:empty".to_owned()),
            ],
        ),
        _ => (Vec::new(), Vec::new()),
    };
    ResponseShape {
        status,
        keys,
        types,
    }
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_i64() || n.is_u64() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn random_mutant_kind(rng: &mut impl rand::Rng) -> MutantKind {
    use crate::fuzz::MUTANT_KINDS;
    MUTANT_KINDS[rng.gen_range(0..MUTANT_KINDS.len())]
}

/// Runs a coverage-guided campaign against a sender function.
///
/// `send` receives a payload and returns the outcome; it is called
/// `rounds * per_round` times total.
pub fn run_campaign<F>(
    fields: Vec<ScalarField>,
    rounds: u32,
    per_round: u32,
    send: &mut F,
) -> CampaignStats
where
    F: FnMut(&Value) -> RequestOutcome,
{
    let mut fuzzer = EvolvedFuzzer::new(fields);
    let mut rng = rand::thread_rng();

    for _ in 0..rounds {
        for _ in 0..per_round {
            let payload = fuzzer.next_payload(&mut rng);
            let outcome = send(&payload);
            fuzzer.record(payload, &outcome);
        }
        fuzzer.end_round();
    }

    fuzzer.stats().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::scalar_fields;

    #[test]
    fn fingerprint_distinguishes_shapes() {
        let a = fingerprint(200, Some(&serde_json::json!({"id": 1})));
        let b = fingerprint(200, Some(&serde_json::json!({"id": "x"})));
        let c = fingerprint(404, Some(&serde_json::json!({"id": 1})));
        assert_ne!(a, b, "type change alters shape");
        assert_ne!(a, c, "status change alters shape");
        assert_eq!(a, fingerprint(200, Some(&serde_json::json!({"id": 99}))));
    }

    #[test]
    fn novel_shapes_grow_corpus() {
        let schema = serde_json::json!({"type":"object","properties":{"q":{"type":"string"}}});
        let mut fuzzer = EvolvedFuzzer::new(scalar_fields(&schema));
        fuzzer.record(
            serde_json::json!({"q": "ok"}),
            &RequestOutcome {
                status: 200,
                body: Some(serde_json::json!({"r": 1})),
                crash: false,
            },
        );
        assert_eq!(fuzzer.stats().corpus_size, 1);
        fuzzer.record(
            serde_json::json!({"q": null}),
            &RequestOutcome {
                status: 500,
                body: None,
                crash: true,
            },
        );
        assert_eq!(fuzzer.stats().corpus_size, 2);
        assert_eq!(fuzzer.stats().crashes, 1);
        assert_eq!(fuzzer.stats().shapes, 2);
    }

    #[test]
    fn campaign_runs_and_reports() {
        let schema = serde_json::json!({"type":"object","properties":{"q":{"type":"string"}}});
        let fields = scalar_fields(&schema);
        let mut seen = 0u32;
        let stats = run_campaign(fields, 2, 5, &mut |_payload| {
            seen += 1;
            RequestOutcome {
                status: if seen.is_multiple_of(3) { 500 } else { 200 },
                body: Some(serde_json::json!({"ok": true})),
                crash: seen.is_multiple_of(3),
            }
        });
        assert_eq!(stats.requests, 10);
        assert_eq!(stats.rounds, 2);
        assert_eq!(stats.crashes, 3);
    }
}
