//! Decode the tag through JSON values rather than serde's generic Content.
//! With arbitrary_precision enabled, Content buffers fractional numbers as
//! serde_json's private map representation, which cannot deserialize as f64.
//! JSON's value deserializer preserves exact metadata and converts only fields
//! whose declared type requests a float. The serialized JSONL format is unchanged.

use std::fmt;

use serde::de::{Error, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use crate::Record;

impl<'de> Deserialize<'de> for Record {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RecordVisitor)
    }
}

struct RecordVisitor;

impl<'de> Visitor<'de> for RecordVisitor {
    type Value = Record;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a journal record object with a kind field")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Record, A::Error> {
        let mut fields = Map::new();
        while let Some((name, value)) = input.next_entry::<String, Value>()? {
            if fields.insert(name.clone(), value).is_some() {
                return Err(A::Error::custom(format!("duplicate field `{name}`")));
            }
        }
        let kind = match fields.remove("kind") {
            Some(Value::String(kind)) => kind,
            Some(_) => return Err(A::Error::custom("record kind must be a string")),
            None => return Err(A::Error::missing_field("kind")),
        };
        let content = Value::Object(fields);
        match kind.as_str() {
            "meta" => serde_json::from_value(content).map(Record::Meta),
            "log" => serde_json::from_value(content).map(Record::Log),
            "traffic" => serde_json::from_value(content).map(Record::Traffic),
            "run_summary" => serde_json::from_value(content).map(Record::RunSummary),
            _ => {
                return Err(A::Error::unknown_variant(
                    &kind,
                    &["meta", "log", "traffic", "run_summary"],
                ));
            }
        }
        .map_err(A::Error::custom)
    }
}
