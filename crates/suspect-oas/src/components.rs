use suspect_low::NodeRef;

use crate::model::{
    Callback, Example, Header, Link, Parameter, RequestBody, Response, SecurityScheme, named_map,
};
use crate::paths::PathItem;
use crate::schema::SchemaView;
use crate::session::Session;

/// The `components` object: reusable named objects.
pub struct Components<'s> {
    session: &'s Session,
    node: NodeRef<'s>,
}

impl<'s> Components<'s> {
    pub(crate) fn new(session: &'s Session, node: NodeRef<'s>) -> Self {
        Self { session, node }
    }

    #[must_use]
    /// The raw `components` node.
    pub fn node(&self) -> NodeRef<'s> {
        self.node
    }

    fn section(&self, key: &str) -> Option<NodeRef<'s>> {
        self.node.get(key)
    }

    /// `(name, schema)` pairs from `components/schemas`.
    #[must_use]
    pub fn schemas(&self) -> Vec<(&'s str, SchemaView<'s>)> {
        self.section("schemas")
            .map(|n| {
                n.entries()
                    .into_iter()
                    .filter_map(|e| e.value.map(|v| (e.key, SchemaView::new(self.session, v))))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[must_use]
    /// One reusable schema by name; `None` when the section or entry is absent.
    pub fn schema(&self, name: &str) -> Option<SchemaView<'s>> {
        self.section("schemas")?
            .get(name)
            .map(|v| SchemaView::new(self.session, v))
    }

    #[must_use]
    /// `components/responses`, in document order.
    pub fn responses(&self) -> Vec<(&'s str, Response<'s>)> {
        named_map(self.session, self.section("responses"), Response::new)
    }

    #[must_use]
    /// `components/parameters`, in document order.
    pub fn parameters(&self) -> Vec<(&'s str, Parameter<'s>)> {
        named_map(self.session, self.section("parameters"), Parameter::new)
    }

    #[must_use]
    /// `components/requestBodies`, in document order.
    pub fn request_bodies(&self) -> Vec<(&'s str, RequestBody<'s>)> {
        named_map(
            self.session,
            self.section("requestBodies"),
            RequestBody::new,
        )
    }

    #[must_use]
    /// `components/examples`, in document order.
    pub fn examples(&self) -> Vec<(&'s str, Example<'s>)> {
        named_map(self.session, self.section("examples"), Example::new)
    }

    #[must_use]
    /// `components/headers`, in document order.
    pub fn headers(&self) -> Vec<(&'s str, Header<'s>)> {
        named_map(self.session, self.section("headers"), Header::new)
    }

    #[must_use]
    /// `components/securitySchemes`, in document order.
    pub fn security_schemes(&self) -> Vec<(&'s str, SecurityScheme<'s>)> {
        named_map(self.session, self.section("securitySchemes"), |s, n| {
            SecurityScheme::new(s, n)
        })
    }

    #[must_use]
    /// `components/links`, in document order.
    pub fn links(&self) -> Vec<(&'s str, Link<'s>)> {
        named_map(self.session, self.section("links"), Link::new)
    }

    #[must_use]
    /// `components/callbacks`, in document order.
    pub fn callbacks(&self) -> Vec<(&'s str, Callback<'s>)> {
        named_map(self.session, self.section("callbacks"), Callback::new)
    }

    /// `pathItems` (3.1+).
    #[must_use]
    pub fn path_items(&self) -> Vec<(&'s str, PathItem<'s>)> {
        self.section("pathItems")
            .map(|n| {
                n.entries()
                    .into_iter()
                    .filter_map(|e| e.value.map(|v| (e.key, PathItem::new(self.session, v))))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Count of entries in a given component section (diagnostics).
    #[must_use]
    pub fn section_len(&self, key: &str) -> usize {
        self.section(key).map_or(0, |n| n.entries().len())
    }
}
