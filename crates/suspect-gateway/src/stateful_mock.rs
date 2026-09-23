//! In-memory resource store for stateful mocking.
//!
//! POST on a collection path synthesizes a resource (deterministic
//! example + generated id), stores it, and returns `201` with the stored
//! body. GET/PUT/DELETE on `/collection/{id}` read, replace, and remove
//! the stored resource. Resources live in one shared map keyed by
//! `(collection template, id)`; ids are extracted from the path or
//! synthesized as increasing integers when the response body carries no
//! id field.
//!
//! The store is optional: when disabled (or for operations that declare
//! no id-bearing path parameter) the mock falls back to the stateless
//! example responses.

use std::collections::BTreeMap;
use std::sync::Mutex;

use bytes::Bytes;
use std::collections::HashMap;

/// One stored resource: body bytes plus the id extracted from it.
#[derive(Debug, Clone)]
pub struct StoredResource {
    /// Body bytes (JSON).
    pub body: Bytes,
    /// The id value carried by the resource (from the `id` path variable).
    pub id: String,
}

/// Shared resource store keyed by `(collection template, id)`.
#[derive(Default)]
pub struct ResourceStore {
    resources: Mutex<HashMap<(String, String), StoredResource>>,
    counters: Mutex<BTreeMap<String, u64>>,
}

impl ResourceStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores a resource under `(collection, id)`, returning the previous
    /// resource if one existed.
    pub fn insert(&self, collection: &str, id: &str, body: Bytes) -> Option<StoredResource> {
        self.resources
            .lock()
            .expect("resource store poisoned")
            .insert(
                (collection.to_owned(), id.to_owned()),
                StoredResource {
                    body,
                    id: id.to_owned(),
                },
            )
    }

    /// Looks up a stored resource.
    #[must_use]
    pub fn get(&self, collection: &str, id: &str) -> Option<StoredResource> {
        self.resources
            .lock()
            .expect("resource store poisoned")
            .get(&(collection.to_owned(), id.to_owned()))
            .cloned()
    }

    /// Removes a stored resource, returning it.
    pub fn remove(&self, collection: &str, id: &str) -> Option<StoredResource> {
        self.resources
            .lock()
            .expect("resource store poisoned")
            .remove(&(collection.to_owned(), id.to_owned()))
    }

    /// Allocates the next synthetic id for a collection (`"1"`, `"2"`, …).
    pub fn next_id(&self, collection: &str) -> String {
        let mut counters = self.counters.lock().expect("counter poisoned");
        let next = counters.entry(collection.to_owned()).or_insert(0);
        *next += 1;
        next.to_string()
    }
}

/// Extracts the `{id}`-style path variable from a request path matched
/// against a template: `/pets/{petId}` + `/pets/42` → `Some("42")`.
/// Returns `None` when the template carries no `{var}` or the shapes
/// mismatch.
#[must_use]
pub fn extract_path_id(template: &str, actual_path: &str) -> Option<String> {
    let t_segments: Vec<&str> = template.split('/').filter(|s| !s.is_empty()).collect();
    let a_segments: Vec<&str> = actual_path.split('/').filter(|s| !s.is_empty()).collect();
    if t_segments.len() != a_segments.len() {
        return None;
    }
    let mut id = None;
    for (t, a) in t_segments.iter().zip(a_segments.iter()) {
        if let Some(var) = t.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            // Heuristic: the first variable named like an id identifies the
            // resource; otherwise remember the last variable's value.
            if var.contains("id") || var.contains("Id") {
                id = Some((*a).to_owned());
            }
        } else if t != a {
            return None;
        }
    }
    id.or_else(|| {
        t_segments
            .iter()
            .zip(a_segments.iter())
            .rev()
            .find_map(|(t, a)| {
                t.strip_prefix('{')
                    .and_then(|s| s.strip_suffix('}'))
                    .map(|_| (*a).to_owned())
            })
    })
}

/// Derives the collection key from a route template: the path prefix
/// before the first `{var}` segment (`/pets/{petId}` → `/pets`).
#[must_use]
pub fn collection_base(template: &str) -> String {
    let mut base = String::new();
    for segment in template.split('/') {
        if segment.starts_with('{') {
            break;
        }
        if !segment.is_empty() {
            base.push('/');
            base.push_str(segment);
        }
    }
    if base.is_empty() {
        "/".to_owned()
    } else {
        base
    }
}

/// Rewrites a resource body's id field to the synthetic id so stored and
/// returned bodies agree. Recognizes `id`, `<collection>Id`-style keys.
#[must_use]
pub fn with_synthesized_id(body: &Bytes, id: &str) -> Bytes {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return body.clone();
    };
    if let serde_json::Value::Object(ref mut map) = value {
        match map.keys().find(|k| k.eq_ignore_ascii_case("id")) {
            Some(key) => {
                let key = key.clone();
                map.insert(key, serde_json::Value::String(id.to_owned()));
            }
            None => {
                map.insert("id".to_owned(), serde_json::Value::String(id.to_owned()));
            }
        }
    }
    Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_id_variable_from_matching_path() {
        assert_eq!(
            extract_path_id("/pets/{petId}", "/pets/42"),
            Some("42".to_owned())
        );
        assert_eq!(extract_path_id("/pets", "/pets/42"), None);
        assert_eq!(extract_path_id("/pets/{petId}", "/pets/"), None);
    }

    #[test]
    fn store_round_trips_resources() {
        let store = ResourceStore::new();
        let id = store.next_id("/pets");
        assert_eq!(id, "1");
        let body = Bytes::from_static(b"{\"name\":\"Rex\"}");
        assert!(store.insert("/pets", &id, body.clone()).is_none());
        assert_eq!(store.get("/pets", "1").unwrap().body, body);
        assert!(store.remove("/pets", "1").is_some());
        assert!(store.get("/pets", "1").is_none());
        assert_eq!(store.next_id("/pets"), "2");
    }

    #[test]
    fn synthesized_id_rewrites_the_id_field() {
        let body = Bytes::from_static(b"{\"id\":\"placeholder\",\"name\":\"Rex\"}");
        let rewritten = with_synthesized_id(&body, "7");
        let value: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(value["id"], "7");
        assert_eq!(value["name"], "Rex");
    }
}
