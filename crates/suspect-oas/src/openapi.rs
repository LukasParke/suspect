use suspect_ref::DocId;
use suspect_low::{NodeRef, ValueKind};

use crate::model::{Info, SecurityRequirement, Server, Tag, ExternalDocumentation};
use crate::paths::{Operation, Paths};
use crate::components::Components;
use crate::{OasVersion, OpenApi, Session};

impl<'s> OpenApi<'s> {
    /// The sniffed [`OasVersion`] for this document.
    #[must_use]
    pub const fn version(&self) -> OasVersion {
        self.version
    }

    /// The session this view borrows; use it to load further entry
    /// documents sharing the same workspace.
    #[must_use]
    pub fn session(&self) -> &'s Session {
        self.session
    }

    /// Root node of the document.
    #[must_use]
    pub fn root(&self) -> NodeRef<'s> {
        self.root
    }

    #[allow(dead_code)] // part of the view API surface
    pub(crate) fn doc_id(&self) -> DocId {
        self.doc
    }

    fn get(&self, key: &str) -> Option<NodeRef<'s>> {
        self.root.get(key)
    }

    /// The `info` object. `None` when absent (invalid per spec, but
    /// views never assume validity).
    #[must_use]
    pub fn info(&self) -> Option<Info<'s>> {
        self.get("info").map(|n| Info::new(self.session, n))
    }

    /// The `paths` object. Absent in pure-webhook documents.
    #[must_use]
    pub fn paths(&self) -> Option<Paths<'s>> {
        self.get("paths").map(|n| Paths::new(self.session, n))
    }

    /// `webhooks` (3.1+). `None` on 3.0.
    #[must_use]
    pub fn webhooks(&self) -> Option<WebhookViews<'s>> {
        if self.version < OasVersion::V31 {
            return None;
        }
        self.get("webhooks").map(|n| WebhookViews { session: self.session, node: n })
    }

    /// The `components` object. `None` when the document declares none.
    #[must_use]
    pub fn components(&self) -> Option<Components<'s>> {
        self.get("components").map(|n| Components::new(self.session, n))
    }

    /// Root-level `servers`; empty when absent.
    #[must_use]
    pub fn servers(&self) -> Vec<Server<'s>> {
        self.get("servers")
            .map(|n| n.items().into_iter().map(|i| Server::new(self.session, i)).collect())
            .unwrap_or_default()
    }

    /// Security requirements at the root level.
    #[must_use]
    pub fn security(&self) -> Vec<SecurityRequirement<'s>> {
        security_list(self.session, self.get("security"))
    }

    /// Root-level `tags`; empty when absent.
    #[must_use]
    pub fn tags(&self) -> Vec<Tag<'s>> {
        self.get("tags")
            .map(|n| n.items().into_iter().map(|i| Tag::new(self.session, i)).collect())
            .unwrap_or_default()
    }

    /// Root-level `externalDocs`. `None` when absent.
    #[must_use]
    pub fn external_docs(&self) -> Option<ExternalDocumentation<'s>> {
        self.get("externalDocs").map(|n| ExternalDocumentation::new(self.session, n))
    }

    /// `jsonSchemaDialect` (3.1+).
    #[must_use]
    pub fn json_schema_dialect(&self) -> Option<&'s str> {
        if self.version < OasVersion::V31 {
            return None;
        }
        self.get("jsonSchemaDialect").and_then(|n| n.as_str())
    }

    /// `openapi` / `swagger` version string as written.
    #[must_use]
    pub fn version_string(&self) -> Option<&'s str> {
        let key = match self.version {
            OasVersion::V30 | OasVersion::V31 | OasVersion::V32 => "openapi",
        };
        self.get(key).and_then(|n| n.as_str())
    }

    /// Flat iterator over every operation: paths first, then webhooks.
    #[must_use]
    pub fn operations(&self) -> Vec<Operation<'s>> {
        let mut out = Vec::new();
        if let Some(paths) = self.paths() {
            for (_, item) in paths.iter() {
                out.extend(item.operations());
            }
        }
        if let Some(webhooks) = self.webhooks() {
            for (_, item) in webhooks.iter() {
                out.extend(item.operations());
            }
        }
        out
    }

    /// Vendor extension value (`x-*`).
    #[must_use]
    pub fn extension(&self, name: &str) -> Option<NodeRef<'s>> {
        debug_assert!(name.starts_with("x-"));
        self.get(name)
    }
}

/// Iteration over webhook path items (`webhooks` map).
///
/// Like every view, borrows the [`Session`] for its lifetime; `$ref` values
/// resolve transparently through [`PathItem::resolved`].
pub struct WebhookViews<'s> {
    session: &'s Session,
    node: NodeRef<'s>,
}

impl<'s> WebhookViews<'s> {
    /// Yields `(webhook-key, path-item)` in document order.
    #[must_use]
    pub fn iter(&self) -> Vec<(String, crate::paths::PathItem<'s>)> {
        self.node
            .entries()
            .into_iter()
            .filter_map(|e| e.value.map(|v| (e.key.to_owned(), crate::paths::PathItem::new(self.session, v))))
            .collect()
    }

    /// Number of webhook entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.node.entries().len()
    }

    /// True when the document declares no webhooks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub(crate) fn security_list<'s>(
    session: &'s Session,
    node: Option<NodeRef<'s>>,
) -> Vec<SecurityRequirement<'s>> {
    node.map(|n| {
        n.items().into_iter().map(|i| SecurityRequirement::new(session, i)).collect()
    })
    .unwrap_or_default()
}

/// Validates that a root node looks like an OpenAPI document (diagnostics).
#[must_use]
#[allow(dead_code)] // diagnostic helper retained for tooling
pub fn looks_like_openapi(root: NodeRef<'_>) -> bool {
    root.kind() == ValueKind::Object && root.get("openapi").is_some()
}

