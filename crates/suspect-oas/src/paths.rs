use suspect_low::NodeRef;

use crate::model::{
    named_map, Callback, ExternalDocumentation, Parameter, RequestBody, Response,
    SecurityRequirement, Server,
};
use crate::openapi::security_list;
use crate::session::{CycleGuard, Session};

/// The `paths` object: `/path` keys to path items.
pub struct Paths<'s> {
    session: &'s Session,
    node: NodeRef<'s>,
}

impl<'s> Paths<'s> {
    pub(crate) fn new(session: &'s Session, node: NodeRef<'s>) -> Self {
        Self { session, node }
    }

    #[must_use]
    /// The raw `paths` node.
    pub fn node(&self) -> NodeRef<'s> {
        self.node
    }

    /// `(path, path-item)` pairs in document order; `$ref` path items resolve.
    #[must_use]
    pub fn iter(&self) -> Vec<(String, PathItem<'s>)> {
        self.node
            .entries()
            .into_iter()
            .filter(|e| e.key.starts_with('/'))
            .filter_map(|e| e.value.map(|v| (e.key.to_owned(), PathItem::new(self.session, v))))
            .collect()
    }

    #[must_use]
    /// One path item by literal path key (`/users/{id}`); templates are
    /// matched verbatim, not expanded.
    pub fn get(&self, path: &str) -> Option<PathItem<'s>> {
        self.node.get(path).map(|v| PathItem::new(self.session, v))
    }

    /// Path-level parameters applying to every operation.
    #[must_use]
    pub fn parameters(&self) -> Vec<Parameter<'s>> {
        self.node
            .get("parameters")
            .map(|n| n.items().into_iter().map(|i| Parameter::new(self.session, i)).collect())
            .unwrap_or_default()
    }

    #[must_use]
    /// Number of `/`-prefixed path entries.
    pub fn len(&self) -> usize {
        self.iter().len()
    }

    #[must_use]
    /// True when the document declares no paths.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One path item: an operation set plus shared metadata. May be a `$ref`.
#[derive(Clone, Copy)]
pub struct PathItem<'s> {
    session: &'s Session,
    node: NodeRef<'s>,
}

/// HTTP method keys on a path item, in canonical order.
pub const PATH_METHODS: [&str; 8] =
    ["get", "put", "post", "delete", "options", "head", "patch", "trace"];

impl<'s> PathItem<'s> {
    pub(crate) fn new(session: &'s Session, node: NodeRef<'s>) -> Self {
        Self { session, node }
    }

    #[must_use]
    /// The raw node backing this item (before [`PathItem::resolved`]).
    pub fn node(&self) -> NodeRef<'s> {
        self.node
    }

    /// Follows a path-item `$ref` (3.1 `components/pathItems` or external).
    #[must_use]
    pub fn resolved(&self) -> Self {
        match self.node.get("$ref") {
            Some(ref_value) => match self.session.resolve(ref_value) {
                    Ok(node) => Self::new(self.session, node),
                    Err(CycleGuard) => *self,
                },
            None => *self,
        }
    }

    fn get(&self, key: &str) -> Option<NodeRef<'s>> {
        self.node.get(key)
    }

    /// Operation for one HTTP method key (`"get"`, ...).
    #[must_use]
    pub fn operation(&self, method: &'static str) -> Option<Operation<'s>> {
        self.resolved()
            .get(method)
            .map(|n| Operation::new(self.session, n, method))
    }

    /// All operations present on this item, in canonical method order.
    #[must_use]
    pub fn operations(&self) -> Vec<Operation<'s>> {
        let r = self.resolved();
        PATH_METHODS
            .iter()
            .filter_map(|m| r.get(m).map(|n| Operation::new(self.session, n, m)))
            .collect()
    }

    #[must_use]
    /// Short summary intended for docs UIs.
    pub fn summary(&self) -> Option<&'s str> {
        self.resolved().get("summary").and_then(|n| n.as_str())
    }

    #[must_use]
    /// Longer prose describing this path.
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }

    /// Parameters inherited by every operation on this item.
    #[must_use]
    pub fn parameters(&self) -> Vec<Parameter<'s>> {
        self.resolved()
            .get("parameters")
            .map(|n| n.items().into_iter().map(|i| Parameter::new(self.session, i)).collect())
            .unwrap_or_default()
    }

    #[must_use]
    /// Item-level `servers`, overriding the document defaults.
    pub fn servers(&self) -> Vec<Server<'s>> {
        self.resolved()
            .get("servers")
            .map(|n| n.items().into_iter().map(|i| Server::new(self.session, i)).collect())
            .unwrap_or_default()
    }
}

/// One API operation.
#[derive(Clone, Copy)]
pub struct Operation<'s> {
    session: &'s Session,
    node: NodeRef<'s>,
    method: &'static str,
}

impl<'s> Operation<'s> {
    pub(crate) fn new(
        session: &'s Session,
        node: NodeRef<'s>,
        method: &'static str,
    ) -> Self {
        Self { session, node, method }
    }

    #[must_use]
    /// The raw node backing this operation.
    pub fn node(&self) -> NodeRef<'s> {
        self.node
    }

    /// HTTP method as a lowercase static string.
    #[must_use]
    pub const fn method(&self) -> &'static str {
        self.method
    }

    fn get(&self, key: &str) -> Option<NodeRef<'s>> {
        self.node.get(key)
    }

    #[must_use]
    /// Unique identifier referenced by [`Link::operation_id`].
    pub fn operation_id(&self) -> Option<&'s str> {
        self.get("operationId").and_then(|n| n.as_str())
    }

    #[must_use]
    /// Short summary intended for docs UIs.
    pub fn summary(&self) -> Option<&'s str> {
        self.get("summary").and_then(|n| n.as_str())
    }

    #[must_use]
    /// Longer prose describing the operation's behavior.
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }

    #[must_use]
    /// True when the operation is explicitly marked deprecated.
    pub fn deprecated(&self) -> bool {
        self.get("deprecated").and_then(|n| n.as_bool()).unwrap_or(false)
    }

    /// Operation parameters; combine with path-item parameters upstream.
    #[must_use]
    pub fn parameters(&self) -> Vec<Parameter<'s>> {
        self.get("parameters")
            .map(|n| n.items().into_iter().map(|i| Parameter::new(self.session, i)).collect())
            .unwrap_or_default()
    }

    #[must_use]
    /// Expected request body; `None` for body-less operations.
    pub fn request_body(&self) -> Option<RequestBody<'s>> {
        self.get("requestBody").map(|n| RequestBody::new(self.session, n))
    }

    #[must_use]
    /// Declared responses; per spec effectively required, but views return
    /// `None` rather than assuming validity.
    pub fn responses(&self) -> Option<Responses<'s>> {
        self.get("responses").map(|n| Responses::new(self.session, n))
    }

    #[must_use]
    /// Client-initiated callbacks, `(expression, callback)` in document order.
    pub fn callbacks(&self) -> Vec<(&'s str, Callback<'s>)> {
        named_map(self.session, self.get("callbacks"), Callback::new)
    }

    #[must_use]
    /// Operation-level security requirements, overriding the document ones.
    pub fn security(&self) -> Vec<SecurityRequirement<'s>> {
        security_list(self.session, self.get("security"))
    }

    #[must_use]
    /// Operation-level `servers`, overriding enclosing scopes.
    pub fn servers(&self) -> Vec<Server<'s>> {
        self.get("servers")
            .map(|n| n.items().into_iter().map(|i| Server::new(self.session, i)).collect())
            .unwrap_or_default()
    }

    #[must_use]
    /// Tag names grouping this operation into categories.
    pub fn tags(&self) -> Vec<&'s str> {
        self.get("tags")
            .map(|n| n.items().into_iter().filter_map(|i| i.as_str()).collect())
            .unwrap_or_default()
    }

    #[must_use]
    /// External documentation reference for this operation.
    pub fn external_docs(&self) -> Option<ExternalDocumentation<'s>> {
        self.get("externalDocs").map(|n| ExternalDocumentation::new(self.session, n))
    }
}

/// The responses map: status codes plus `default`.
pub struct Responses<'s> {
    session: &'s Session,
    node: NodeRef<'s>,
}

impl<'s> Responses<'s> {
    pub(crate) fn new(session: &'s Session, node: NodeRef<'s>) -> Self {
        Self { session, node }
    }

    #[must_use]
    /// The raw responses node.
    pub fn node(&self) -> NodeRef<'s> {
        self.node
    }

    /// `(status-key, response)` in document order; keys are codes like `"200"`
    /// or ranges like `"2XX"` / `"default"`.
    #[must_use]
    pub fn iter(&self) -> Vec<(&'s str, Response<'s>)> {
        self.node
            .entries()
            .into_iter()
            .filter_map(|e| e.value.map(|v| (e.key, Response::new(self.session, v))))
            .collect()
    }

    #[must_use]
    /// One entry by status key (`"200"`, `"4XX"`, ...); see
    /// [`Responses::iter`] for key semantics.
    pub fn get(&self, status: &str) -> Option<Response<'s>> {
        self.node.get(status).map(|v| Response::new(self.session, v))
    }

    #[must_use]
    /// The catch-all `default` response.
    pub fn default(&self) -> Option<Response<'s>> {
        self.get("default")
    }

    #[must_use]
    /// Number of response entries including `default`.
    pub fn len(&self) -> usize {
        self.node.entries().len()
    }

    #[must_use]
    /// True when no response entries exist.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// re-exported types referenced by Operation accessors above
#[allow(unused_imports)]
use crate::model::{Encoding as _Encoding, Example as _Example};
