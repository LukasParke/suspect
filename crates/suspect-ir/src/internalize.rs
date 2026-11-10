//! Multi-file descriptions: references that leave the entry document.
//!
//! The snapshot addresses schemas, responses, parameters, request bodies
//! and headers by local component name. A description split across files
//! references them by URI instead (`$ref: './schemas/pet.yaml'`).
//! [`internalize`] closes that gap before the snapshot is built: every
//! reference that leaves the entry document is loaded through the
//! workspace, copied into the entry's `components` under a generated name,
//! and rewritten to point there. References inside a copied target are
//! rewritten the same way against that target's own document, so a file
//! that refers to itself or to its neighbours stays a graph of local
//! references — recursion included.
//!
//! What cannot be loaded is left exactly as written. The snapshot then
//! records the reference as unresolved and conformance checks report it;
//! nothing is guessed.

use std::collections::{HashMap, HashSet};

use suspect_low::Pointer;
use suspect_ref::{ParsedRef, Workspace, parse_ref};
use suspect_source::Uri;
use suspect_syntax::FastValue;

use crate::fast::value_from_node;

/// The `components` section a reference's position selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Schemas,
    Responses,
    Parameters,
    RequestBodies,
    Headers,
    PathItems,
}

impl Section {
    fn key(self) -> &'static str {
        match self {
            Self::Schemas => "schemas",
            Self::Responses => "responses",
            Self::Parameters => "parameters",
            Self::RequestBodies => "requestBodies",
            Self::Headers => "headers",
            Self::PathItems => "pathItems",
        }
    }
}

/// Schema keywords whose value is data, not a schema: a `$ref` key inside
/// them is part of an example, not a reference.
const DATA_KEYWORDS: [&str; 5] = ["const", "default", "enum", "example", "examples"];

/// Schema keywords whose value maps names to schemas. The names are the
/// description author's, not keywords: a property may be called `default`.
const SCHEMA_MAPS: [&str; 5] = [
    "$defs",
    "definitions",
    "dependentSchemas",
    "patternProperties",
    "properties",
];

/// HTTP method keys of a Path Item.
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Whether a parsed tree holds a reference that leaves its document.
pub(crate) fn has_external_refs(value: &FastValue) -> bool {
    match value {
        FastValue::Object(entries) => entries.iter().any(|(key, child)| {
            (key == "$ref"
                && matches!(child, FastValue::Scalar { raw, .. } if !raw.starts_with('#')))
                || has_external_refs(child)
        }),
        FastValue::Array(items) => items.iter().any(has_external_refs),
        FastValue::Scalar { .. } => false,
    }
}

/// Copies every externally referenced schema, response, parameter, request
/// body, header and path item into `root`'s `components` and rewrites the
/// references to them. `root` is the entry document at `entry`.
pub(crate) fn internalize(ws: &Workspace, entry: &Uri, root: &mut FastValue) {
    if !has_external_refs(root) {
        return;
    }
    let mut state = State {
        ws,
        entry,
        names: HashMap::new(),
        taken: HashMap::new(),
        next_suffix: HashMap::new(),
        pending: Vec::new(),
        added: Vec::new(),
    };
    state.reserve_existing(root);
    state.document(root);
    // Each copied target may reference further documents; the queue drains
    // when the closure is complete. `names` already holds every queued
    // target, so a cycle resolves to the name it started with.
    while let Some(target) = state.pending.pop() {
        let Some(mut value) = state.load(&target.uri, &target.pointer) else {
            continue;
        };
        state.position(&mut value, target.section, &target.uri);
        state.added.push((target.section, target.name, value));
    }
    let added = std::mem::take(&mut state.added);
    merge(root, added);
}

/// One externally referenced value waiting to be copied in.
struct Target {
    uri: Uri,
    pointer: Pointer,
    section: Section,
    name: String,
}

struct State<'a> {
    ws: &'a Workspace,
    entry: &'a Uri,
    /// `(document, pointer, section)` → the component name it was given.
    names: HashMap<(Uri, String, Section), String>,
    /// Names in use per section: the entry's own components and every
    /// generated name.
    taken: HashMap<Section, HashSet<String>>,
    /// The next numeric suffix to try for a stem already in use, so many
    /// targets that share a stem (`schema.yaml` in every directory) are
    /// named without retrying every earlier suffix.
    next_suffix: HashMap<(Section, String), usize>,
    pending: Vec<Target>,
    added: Vec<(Section, String, FastValue)>,
}

impl State<'_> {
    /// Records the entry document's own component names so a generated
    /// name never replaces one.
    fn reserve_existing(&mut self, root: &FastValue) {
        let Some(components) = root.get("components") else {
            return;
        };
        for section in [
            Section::Schemas,
            Section::Responses,
            Section::Parameters,
            Section::RequestBodies,
            Section::Headers,
            Section::PathItems,
        ] {
            if let Some(entries) = components.get(section.key()) {
                self.taken
                    .entry(section)
                    .or_default()
                    .extend(entries.entries().iter().map(|(name, _)| name.clone()));
            }
        }
    }

    /// The value at `pointer` of the document at `uri`, loaded on demand.
    fn load(&self, uri: &Uri, pointer: &Pointer) -> Option<FastValue> {
        let handle = self.ws.open(uri.as_str()).ok()?;
        let node = handle.node_at_pointer(pointer).ok()?;
        value_from_node(node).ok()
    }

    /// Walks the entry document: paths and the components it declares.
    fn document(&mut self, root: &mut FastValue) {
        let entry = self.entry.clone();
        if let Some(paths) = child_mut(root, "paths") {
            for (_, item) in entries_mut(paths) {
                self.position(item, Section::PathItems, &entry);
            }
        }
        if let Some(components) = child_mut(root, "components") {
            for section in [
                Section::Schemas,
                Section::Responses,
                Section::Parameters,
                Section::RequestBodies,
                Section::Headers,
                Section::PathItems,
            ] {
                if let Some(entries) = child_mut(components, section.key()) {
                    for (_, value) in entries_mut(entries) {
                        self.position(value, section, &entry);
                    }
                }
            }
        }
    }

    /// Rewrites the references of one value that sits in a `section`
    /// position of the document at `base`.
    fn position(&mut self, value: &mut FastValue, section: Section, base: &Uri) {
        if section == Section::Schemas {
            self.schema_position(value, base);
            return;
        }
        if self.reference(value, section, base) {
            return;
        }
        match section {
            Section::Schemas => {}
            Section::PathItems => {
                self.each(value, "parameters", Section::Parameters, base);
                for method in METHODS {
                    if let Some(operation) = child_mut(value, method) {
                        self.each(operation, "parameters", Section::Parameters, base);
                        if let Some(body) = child_mut(operation, "requestBody") {
                            self.position(body, Section::RequestBodies, base);
                        }
                        self.each(operation, "responses", Section::Responses, base);
                    }
                }
            }
            Section::Parameters | Section::Headers => {
                if let Some(schema) = child_mut(value, "schema") {
                    self.position(schema, Section::Schemas, base);
                }
                self.content(value, base);
            }
            Section::RequestBodies => self.content(value, base),
            Section::Responses => {
                self.each(value, "headers", Section::Headers, base);
                self.content(value, base);
            }
        }
    }

    /// Applies [`State::position`] to every member of the array or map at
    /// `key`.
    fn each(&mut self, parent: &mut FastValue, key: &str, section: Section, base: &Uri) {
        match child_mut(parent, key) {
            Some(FastValue::Array(items)) => {
                for item in items {
                    self.position(item, section, base);
                }
            }
            Some(FastValue::Object(entries)) => {
                for (_, value) in entries {
                    self.position(value, section, base);
                }
            }
            _ => {}
        }
    }

    /// The schema of every media type of a `content` map.
    fn content(&mut self, parent: &mut FastValue, base: &Uri) {
        let Some(content) = child_mut(parent, "content") else {
            return;
        };
        for (_, media) in entries_mut(content) {
            if let Some(schema) = child_mut(media, "schema") {
                self.position(schema, Section::Schemas, base);
            }
        }
    }

    /// One schema: its own reference is rewritten, and so is everything
    /// beside and below it. A schema reference may carry sibling keywords
    /// (`{$ref: ..., properties: {...}}`), and those are schema too.
    fn schema_position(&mut self, value: &mut FastValue, base: &Uri) {
        self.reference(value, Section::Schemas, base);
        self.schema(value, base);
    }

    /// The schemas nested in a schema. Data keywords are skipped, and the
    /// members of a name-to-schema map are schemas whatever their names.
    fn schema(&mut self, value: &mut FastValue, base: &Uri) {
        match value {
            FastValue::Object(entries) => {
                for (key, child) in entries {
                    if DATA_KEYWORDS.contains(&key.as_str()) {
                        continue;
                    }
                    if SCHEMA_MAPS.contains(&key.as_str()) {
                        for (_, member) in entries_mut(child) {
                            self.schema_position(member, base);
                        }
                        continue;
                    }
                    self.schema_position(child, base);
                }
            }
            FastValue::Array(items) => {
                for item in items {
                    self.schema_position(item, base);
                }
            }
            FastValue::Scalar { .. } => {}
        }
    }

    /// Rewrites `value`'s own `$ref`, when it has one that needs it.
    /// Returns whether `value` is a reference object, in which case its
    /// other members are not walked: they describe the reference, not a
    /// value of this position.
    fn reference(&mut self, value: &mut FastValue, section: Section, base: &Uri) -> bool {
        let FastValue::Object(entries) = value else {
            return false;
        };
        let Some((_, FastValue::Scalar { raw, .. })) =
            entries.iter_mut().find(|(key, _)| key == "$ref")
        else {
            return false;
        };
        let local_to_entry = base == self.entry && raw.starts_with('#');
        if !local_to_entry && let Some(rewritten) = self.rewrite(raw, section, base) {
            *raw = rewritten;
        }
        true
    }

    /// The local spelling of a reference found in the document at `base`,
    /// or `None` when it is left as written.
    fn rewrite(&mut self, raw: &str, section: Section, base: &Uri) -> Option<String> {
        let (uri, pointer) = match parse_ref(base, raw).ok()? {
            ParsedRef::Local(pointer) => (base.clone(), pointer),
            ParsedRef::External { uri, pointer } => (uri, pointer),
            // Anchors are resolved by the schema compiler, not by name.
            ParsedRef::PlainName(_) | ParsedRef::ExternalAnchor { .. } => return None,
        };
        if &uri == self.entry {
            return Some(format!("#{}", pointer.to_path()));
        }
        let key = (uri.clone(), pointer.to_path(), section);
        let name = match self.names.get(&key) {
            Some(name) => name.clone(),
            None => {
                // Only a target that exists gets a name; a dangling
                // reference stays as written and is reported downstream.
                self.load(&uri, &pointer)?;
                let name = self.name_for(&uri, &pointer, section);
                self.names.insert(key, name.clone());
                self.pending.push(Target {
                    uri,
                    pointer,
                    section,
                    name: name.clone(),
                });
                name
            }
        };
        Some(format!("#/components/{}/{name}", section.key()))
    }

    /// A component name for an external target: its last pointer token, or
    /// its file stem for a whole document, made unique within the section.
    fn name_for(&mut self, uri: &Uri, pointer: &Pointer, section: Section) -> String {
        let stem = pointer.tokens().last().map_or_else(
            || {
                let path = uri.as_str().split(['?', '#']).next().unwrap_or_default();
                let file = path.rsplit('/').next().unwrap_or_default();
                file.split('.').next().unwrap_or_default().to_owned()
            },
            |token| token.to_string(),
        );
        // Component names are `[A-Za-z0-9._-]`; anything else would need
        // escaping in the reference that points here.
        let mut base: String = stem
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        if base.is_empty() {
            base.push_str("external");
        }
        let taken = self.taken.entry(section).or_default();
        let mut name = base.clone();
        let suffix = self.next_suffix.entry((section, base.clone())).or_insert(2);
        while taken.contains(&name) {
            name = format!("{base}_{suffix}");
            *suffix += 1;
        }
        taken.insert(name.clone());
        name
    }
}

/// The value stored under `key`, mutably.
fn child_mut<'v>(value: &'v mut FastValue, key: &str) -> Option<&'v mut FastValue> {
    match value {
        FastValue::Object(entries) => entries
            .iter_mut()
            .find(|(name, _)| name == key)
            .map(|(_, child)| child),
        _ => None,
    }
}

/// The entries of a mapping, mutably; empty for anything else.
fn entries_mut(value: &mut FastValue) -> &mut [(String, FastValue)] {
    match value {
        FastValue::Object(entries) => entries,
        _ => &mut [],
    }
}

/// Appends the copied targets to `root`'s `components`, creating the
/// object and its sections as needed, in the order targets were named.
fn merge(root: &mut FastValue, mut added: Vec<(Section, String, FastValue)>) {
    if added.is_empty() {
        return;
    }
    // Targets were queued depth-first from a stack; name order is stable
    // across runs and independent of traversal order.
    added.sort_by(|left, right| (left.0.key(), &left.1).cmp(&(right.0.key(), &right.1)));
    let FastValue::Object(top) = root else {
        return;
    };
    // `components:` and its sections may be absent, or present and empty
    // (`schemas:` with nothing under it is a null): either way they become
    // objects, since references already point into them.
    let components = object_entry(top, "components");
    for (section, name, value) in added {
        object_entry(components, section.key()).push((name, value));
    }
}

/// The entries of the mapping stored under `key`, created (or made a
/// mapping) when it is absent or holds anything else.
fn object_entry<'v>(
    entries: &'v mut Vec<(String, FastValue)>,
    key: &str,
) -> &'v mut Vec<(String, FastValue)> {
    let index = match entries.iter().position(|(name, _)| name == key) {
        Some(index) => index,
        None => {
            entries.push((key.to_owned(), FastValue::Object(Vec::new())));
            entries.len() - 1
        }
    };
    if !matches!(entries[index].1, FastValue::Object(_)) {
        entries[index].1 = FastValue::Object(Vec::new());
    }
    match &mut entries[index].1 {
        FastValue::Object(members) => members,
        _ => unreachable!("made a mapping above"),
    }
}
