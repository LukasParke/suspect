//! JSON conversion without serde's private arbitrary-number wire format.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use minijinja::value::{Kwargs, Object, ObjectRepr, ValueKind};
use minijinja::{Error, ErrorKind, Value};
use serde::ser::{Error as _, SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use serde_json::{Number, Value as Json};

/// A number that cannot pass through MiniJinja's native scalars unchanged.
/// Rendering and JSON serialization remain exact; arithmetic is unsupported
/// by MiniJinja for this plain object and returns an evaluation error.
#[derive(Debug)]
struct ExactNumber(Number);

impl Object for ExactNumber {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Plain
    }

    fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }

    fn is_true(self: &Arc<Self>) -> bool {
        self.0
            .as_str()
            .split(['e', 'E'])
            .next()
            .is_some_and(|mantissa| mantissa.bytes().any(|digit| matches!(digit, b'1'..=b'9')))
    }
}

pub(crate) fn to_template(value: &Json) -> Value {
    match value {
        Json::Null => Value::from(()),
        Json::Bool(value) => Value::from(*value),
        Json::String(value) => Value::from(value.as_str()),
        Json::Number(number) => {
            if let Some(value) = number.as_i64() {
                Value::from(value)
            } else if let Some(value) = number.as_u64() {
                Value::from(value)
            } else if let Some(value) = number
                .as_f64()
                .filter(|value| Number::from_f64(*value).as_ref() == Some(number))
            {
                Value::from(value)
            } else {
                Value::from_object(ExactNumber(number.clone()))
            }
        }
        Json::Array(values) => Value::from(values.iter().map(to_template).collect::<Vec<_>>()),
        Json::Object(values) => Value::from(
            values
                .iter()
                .map(|(key, value)| (key.clone(), to_template(value)))
                .collect::<BTreeMap<_, _>>(),
        ),
    }
}

/// Serializes nested template values while recovering only typed number
/// objects. A user map with serde's private key is still an ordinary map.
struct JsonValue<'a>(&'a Value);

impl Serialize for JsonValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(number) = self.0.downcast_object_ref::<ExactNumber>() {
            return number.0.serialize(serializer);
        }
        match self.0.kind() {
            ValueKind::Map => {
                let mut map = serializer.serialize_map(self.0.len())?;
                for key in self.0.try_iter().map_err(S::Error::custom)? {
                    let value = self.0.get_item(&key).map_err(S::Error::custom)?;
                    map.serialize_entry(&key, &JsonValue(&value))?;
                }
                map.end()
            }
            ValueKind::Seq | ValueKind::Iterable => {
                let mut seq = serializer.serialize_seq(self.0.len())?;
                for value in self.0.try_iter().map_err(S::Error::custom)? {
                    seq.serialize_element(&JsonValue(&value))?;
                }
                seq.end()
            }
            _ => self.0.serialize(serializer),
        }
    }
}

fn json_error(error: serde_json::Error) -> Error {
    Error::new(ErrorKind::InvalidOperation, "cannot serialize to JSON").with_source(error)
}

pub(crate) fn to_json(value: &Value) -> Result<Json, Error> {
    serde_json::to_value(JsonValue(value)).map_err(json_error)
}

pub(crate) fn tojson(value: &Value, indent: Option<Value>, args: Kwargs) -> Result<Value, Error> {
    let indent = match indent {
        Some(indent) => Some(indent),
        None => args.get("indent")?,
    };
    args.assert_all_used()?;
    let indent = match indent {
        None => None,
        Some(value) => match bool::try_from(value.clone()).ok() {
            Some(true) => Some(2),
            Some(false) => None,
            None => Some(usize::try_from(value)?),
        },
    };
    let json = if let Some(indent) = indent {
        let mut output = Vec::new();
        let indentation = vec![b' '; indent];
        let formatter = serde_json::ser::PrettyFormatter::with_indent(&indentation);
        let mut serializer = serde_json::Serializer::with_formatter(&mut output, formatter);
        JsonValue(value)
            .serialize(&mut serializer)
            .map_err(json_error)?;
        // serde_json writes UTF-8; preserve safe Rust if that ever changes.
        String::from_utf8(output).map_err(|error| {
            Error::new(ErrorKind::InvalidOperation, "invalid JSON UTF-8").with_source(error)
        })?
    } else {
        serde_json::to_string(&JsonValue(value)).map_err(json_error)?
    };
    let mut escaped = String::with_capacity(json.len());
    for character in json.chars() {
        match character {
            '<' => escaped.push_str("\\u003c"),
            '>' => escaped.push_str("\\u003e"),
            '&' => escaped.push_str("\\u0026"),
            '\'' => escaped.push_str("\\u0027"),
            '\u{2028}' => escaped.push_str("\\u2028"),
            '\u{2029}' => escaped.push_str("\\u2029"),
            _ => escaped.push(character),
        }
    }
    Ok(Value::from_safe_string(escaped))
}
