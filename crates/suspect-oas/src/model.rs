use suspect_low::NodeRef;

use crate::SchemaView;
use crate::session::{CycleGuard, Session};

/// Common view plumbing: session + home doc + raw node.
macro_rules! view {
    ($(#[doc = $doc:expr])? $name:ident) => {
        // `session`/`get` are part of the uniform view plumbing; non-ref
        // views never touch them, hence the allow.
        #[allow(dead_code)]
        #[derive(Debug, Clone, Copy)]
        $(#[doc = $doc])?
        pub struct $name<'s> {
            pub(crate) session: &'s Session,
            pub(crate) node: NodeRef<'s>,
        }

        #[allow(dead_code)]
        impl<'s> $name<'s> {
            pub(crate) fn new(session: &'s Session, node: NodeRef<'s>) -> Self {
                Self { session, node }
            }

            /// The raw node backing this view.
            #[must_use]
            pub fn node(&self) -> NodeRef<'s> {
                self.node
            }

            fn get(&self, key: &str) -> Option<NodeRef<'s>> {
                self.node.get(key)
            }
        }
    };
}

/// Adds `$ref` resolution to a view whose target is another instance of itself.
macro_rules! ref_view {
    ($name:ident) => {
        impl<'s> $name<'s> {
            /// Follows this object's `$ref`, if any.
            #[must_use]
            pub fn resolved(&self) -> Self {
                match self.get("$ref") {
                    Some(ref_value) => match self.session.resolve(ref_value) {
                        Ok(node) => Self::new(self.session, node),
                        Err(CycleGuard) => *self,
                    },
                    None => *self,
                }
            }

            /// True when this object is a reference.
            #[must_use]
            pub fn is_ref(&self) -> bool {
                self.get("$ref").is_some()
            }

            /// The raw `$ref` string, when present.
            #[must_use]
            pub fn ref_text(&self) -> Option<&'s str> {
                self.get("$ref").and_then(|n| n.as_str())
            }
        }
    };
}

view!(
    /// The document's `info` object: API metadata.
    Info
);
view!(
    /// Contact information inside [`Info`].
    Contact
);
view!(
    /// License information inside [`Info`].
    License
);
view!(
    /// One deployment target (`servers` entry).
    Server
);
view!(
    /// A substitution variable inside a [`Server`] URL template.
    ServerVariable
);
view!(
    /// A logical grouping tag declared at the document root.
    Tag
);
view!(
    /// A reference to external documentation.
    ExternalDocumentation
);
view!(
    /// XML serialization metadata for a schema.
    Xml
);
view!(
    /// Polymorphism discriminator attached to a schema.
    Discriminator
);
view!(
    /// Per-property wire encoding inside a media type.
    Encoding
);
view!(
    /// A named or inline example.
    Example
);
ref_view!(Example);
view!(
    /// One media type entry of a request/response body.
    MediaType
);
view!(
    /// A header object (request/response header, not a parameter).
    Header
);
ref_view!(Header);
view!(
    /// An operation parameter (query/header/path/cookie).
    Parameter
);
ref_view!(Parameter);
view!(
    /// An operation request body.
    RequestBody
);
ref_view!(RequestBody);
view!(
    /// One response description.
    Response
);
ref_view!(Response);
view!(
    /// A link from a response to a follow-up operation.
    Link
);
ref_view!(Link);
view!(
    /// A callback: client-initiated requests keyed by runtime expressions.
    Callback
);
view!(
    /// A security scheme definition.
    SecurityScheme
);
ref_view!(SecurityScheme);
view!(
    /// The OAuth2 `flows` map of a security scheme.
    OauthFlows
);
view!(
    /// A single OAuth2 flow configuration.
    OauthFlow
);
view!(
    /// One entry of a `security` list: required schemes and their scopes.
    SecurityRequirement
);

impl<'s> Info<'s> {
    #[must_use]
    /// Human-readable title of the API.
    pub fn title(&self) -> Option<&'s str> {
        self.get("title").and_then(|n| n.as_str())
    }

    /// 3.2+ summary.
    #[must_use]
    pub fn summary(&self) -> Option<&'s str> {
        self.get("summary").and_then(|n| n.as_str())
    }

    #[must_use]
    /// Version string of the API document itself (free-form, not the spec version).
    pub fn version(&self) -> Option<&'s str> {
        self.get("version").and_then(|n| n.as_str())
    }

    #[must_use]
    /// Longer prose description of the API.
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }

    #[must_use]
    /// URL to the terms of service.
    pub fn terms_of_service(&self) -> Option<&'s str> {
        self.get("termsOfService").and_then(|n| n.as_str())
    }

    #[must_use]
    /// Contact information for the API.
    pub fn contact(&self) -> Option<Contact<'s>> {
        self.get("contact").map(|n| Contact::new(self.session, n))
    }

    #[must_use]
    /// License under which the API is published.
    pub fn license(&self) -> Option<License<'s>> {
        self.get("license").map(|n| License::new(self.session, n))
    }
}

impl<'s> Contact<'s> {
    #[must_use]
    /// Identifying name of the contact person or organization.
    pub fn name(&self) -> Option<&'s str> {
        self.get("name").and_then(|n| n.as_str())
    }
    #[must_use]
    /// URL pointing to the contact information.
    pub fn url(&self) -> Option<&'s str> {
        self.get("url").and_then(|n| n.as_str())
    }
    #[must_use]
    /// Email address of the contact person or organization.
    pub fn email(&self) -> Option<&'s str> {
        self.get("email").and_then(|n| n.as_str())
    }
}

impl<'s> License<'s> {
    #[must_use]
    /// License name as used on the API.
    pub fn name(&self) -> Option<&'s str> {
        self.get("name").and_then(|n| n.as_str())
    }
    /// URL pointing to the full license text. Mutually exclusive with
    /// [`License::identifier`] per the spec.
    #[must_use]
    pub fn url(&self) -> Option<&'s str> {
        self.get("url").and_then(|n| n.as_str())
    }
    /// 3.1+ SPDX identifier.
    #[must_use]
    pub fn identifier(&self) -> Option<&'s str> {
        self.get("identifier").and_then(|n| n.as_str())
    }
}

impl<'s> Server<'s> {
    #[must_use]
    /// Base URL of the server; may contain [`Server::variables`] templates.
    pub fn url(&self) -> Option<&'s str> {
        self.get("url").and_then(|n| n.as_str())
    }
    #[must_use]
    /// Optional prose describing this deployment target.
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    /// Reusable template variables for [`Server::url`], in document order.
    pub fn variables(&self) -> Vec<(&'s str, ServerVariable<'s>)> {
        self.get("variables")
            .map(|n| {
                n.entries()
                    .into_iter()
                    .filter_map(|e| {
                        e.value
                            .map(|v| (e.key, ServerVariable::new(self.session, v)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl<'s> ServerVariable<'s> {
    #[must_use]
    /// Default value used when the client supplies none.
    pub fn default_value(&self) -> Option<&'s str> {
        self.get("default").and_then(|n| n.as_str())
    }
    #[must_use]
    /// Allowed substitutions; empty when unrestricted.
    pub fn enum_values(&self) -> Vec<&'s str> {
        self.get("enum")
            .map(|n| n.items().into_iter().filter_map(|i| i.as_str()).collect())
            .unwrap_or_default()
    }
    #[must_use]
    /// Optional prose describing this variable.
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }
}

impl<'s> Tag<'s> {
    #[must_use]
    /// Tag name; unique within the document's `tags` array.
    pub fn name(&self) -> Option<&'s str> {
        self.get("name").and_then(|n| n.as_str())
    }
    #[must_use]
    /// Optional prose explaining the tag's purpose.
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    /// External documentation reference for this tag.
    pub fn external_docs(&self) -> Option<ExternalDocumentation<'s>> {
        self.get("externalDocs")
            .map(|n| ExternalDocumentation::new(self.session, n))
    }
    /// 3.2+ parent tag.
    #[must_use]
    pub fn parent(&self) -> Option<&'s str> {
        self.get("parent").and_then(|n| n.as_str())
    }
    /// 3.2+ kind.
    #[must_use]
    pub fn kind(&self) -> Option<&'s str> {
        self.get("kind").and_then(|n| n.as_str())
    }
}

impl<'s> ExternalDocumentation<'s> {
    #[must_use]
    /// Short prose describing the target documentation.
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    /// URL of the target documentation.
    pub fn url(&self) -> Option<&'s str> {
        self.get("url").and_then(|n| n.as_str())
    }
}

impl<'s> Xml<'s> {
    #[must_use]
    /// Element/attribute name used when serializing this schema as XML.
    pub fn name(&self) -> Option<&'s str> {
        self.get("name").and_then(|n| n.as_str())
    }
    #[must_use]
    /// XML namespace URI accompanying [`Xml::prefix`].
    pub fn namespace(&self) -> Option<&'s str> {
        self.get("namespace").and_then(|n| n.as_str())
    }
    #[must_use]
    /// Namespace prefix used together with [`Xml::namespace`].
    pub fn prefix(&self) -> Option<&'s str> {
        self.get("prefix").and_then(|n| n.as_str())
    }
    #[must_use]
    /// True when the property serializes as an XML attribute rather than an element.
    pub fn attribute(&self) -> bool {
        self.get("attribute")
            .and_then(|n| n.as_bool())
            .unwrap_or(false)
    }
    #[must_use]
    /// True when array items wrap in a container element named after the property.
    pub fn wrapped(&self) -> bool {
        self.get("wrapped")
            .and_then(|n| n.as_bool())
            .unwrap_or(false)
    }
}

impl<'s> Discriminator<'s> {
    #[must_use]
    /// Name of the property whose value selects the referenced subschema.
    pub fn property_name(&self) -> Option<&'s str> {
        self.get("propertyName").and_then(|n| n.as_str())
    }
    /// `(value, ref-string)` mapping entries.
    #[must_use]
    pub fn mapping(&self) -> Vec<(&'s str, &'s str)> {
        self.get("mapping")
            .map(|n| {
                n.entries()
                    .into_iter()
                    .filter_map(|e| e.value.and_then(|v| v.as_str()).map(|s| (e.key, s)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl<'s> Encoding<'s> {
    #[must_use]
    /// Content type this encoding applies to (e.g. `image/png`, `text/plain`).
    pub fn content_type(&self) -> Option<&'s str> {
        self.get("contentType").and_then(|n| n.as_str())
    }
    #[must_use]
    /// Per-property headers sent alongside this media type.
    pub fn headers(&self) -> Vec<(&'s str, Header<'s>)> {
        self.get("headers")
            .map(|n| {
                n.entries()
                    .into_iter()
                    .filter_map(|e| e.value.map(|v| (e.key, Header::new(self.session, v))))
                    .collect()
            })
            .unwrap_or_default()
    }
    #[must_use]
    /// Serialization style for this property; defaults to `form`.
    pub fn style(&self) -> Option<ParameterStyle> {
        self.get("style")
            .and_then(|n| n.as_str())
            .and_then(ParameterStyle::parse)
    }
    #[must_use]
    /// Whether arrays/objects generate separate parameters per item;
    /// defaults to the style's own default (`true` for `form`).
    pub fn explode(&self) -> Option<bool> {
        self.get("explode").and_then(|n| n.as_bool())
    }
    #[must_use]
    /// True when `Reserved` characters in values are left unescaped.
    pub fn allow_reserved(&self) -> bool {
        self.get("allowReserved")
            .and_then(|n| n.as_bool())
            .unwrap_or(false)
    }
}

impl<'s> Example<'s> {
    /// Short summary of the example.
    #[must_use]
    pub fn summary(&self) -> Option<&'s str> {
        self.resolved().get("summary").and_then(|n| n.as_str())
    }
    /// Longer prose describing the example.
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
    /// Embedded example value; mutually exclusive with
    /// [`Example::external_value`] per the spec.
    #[must_use]
    pub fn value(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("value")
    }
    /// URI pointing to an external example payload.
    #[must_use]
    pub fn external_value(&self) -> Option<&'s str> {
        self.resolved()
            .get("externalValue")
            .and_then(|n| n.as_str())
    }
}

impl<'s> MediaType<'s> {
    /// Schema the payload conforms to. `None` when only raw examples are given.
    #[must_use]
    pub fn schema(&self) -> Option<SchemaView<'s>> {
        self.get("schema").map(|v| SchemaView::new(self.session, v))
    }

    /// Inline example payload, overriding any schema-generated example.
    #[must_use]
    pub fn example(&self) -> Option<NodeRef<'s>> {
        self.get("example")
    }

    /// Named examples keyed by media type, in document order.
    #[must_use]
    pub fn examples(&self) -> Vec<(&'s str, Example<'s>)> {
        named_map(self.session, self.get("examples"), Example::new)
    }

    /// Per-property encoding rules, in document order.
    #[must_use]
    pub fn encoding(&self) -> Vec<(&'s str, Encoding<'s>)> {
        named_map(self.session, self.get("encoding"), Encoding::new)
    }
}

impl<'s> Parameter<'s> {
    /// Parameter name as used in `in` (e.g. the query-key or header name).
    #[must_use]
    pub fn name(&self) -> Option<&'s str> {
        self.resolved().get("name").and_then(|n| n.as_str())
    }

    /// Where the parameter appears ([`ParameterIn::Path`], query, ...).
    /// `None` for a value outside the four spec locations.
    #[must_use]
    pub fn location(&self) -> Option<ParameterIn> {
        self.resolved()
            .get("in")
            .and_then(|n| n.as_str())
            .and_then(ParameterIn::parse)
    }

    /// Whether the parameter must be supplied. Path parameters default to
    /// required per the spec; everything else defaults to optional.
    #[must_use]
    pub fn required(&self) -> bool {
        let r = self.resolved();
        let in_path = r.get("in").and_then(|n| n.as_str()) == Some("path");
        r.get("required")
            .and_then(|n| n.as_bool())
            .unwrap_or(in_path)
    }

    /// True when the parameter is explicitly marked deprecated.
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.resolved()
            .get("deprecated")
            .and_then(|n| n.as_bool())
            .unwrap_or(false)
    }

    /// Prose describing how the parameter is used.
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }

    /// Schema validating the parameter value; exclusive with
    /// [`Parameter::content`] per the spec.
    #[must_use]
    pub fn schema(&self) -> Option<SchemaView<'s>> {
        self.resolved()
            .get("schema")
            .map(|v| SchemaView::new(self.session, v))
    }

    /// Content-style parameter payload: `(media-type, media-type-object)`.
    #[must_use]
    pub fn content(&self) -> Vec<(&'s str, MediaType<'s>)> {
        named_map(self.session, self.resolved().get("content"), |s, n| {
            MediaType::new(s, n)
        })
    }

    /// Inline example value for the parameter.
    #[must_use]
    pub fn example(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("example")
    }

    /// Named [`Example`] objects for this parameter, in document order.
    #[must_use]
    pub fn examples(&self) -> Vec<(&'s str, Example<'s>)> {
        named_map(self.session, self.resolved().get("examples"), Example::new)
    }

    /// Serialization style; see the OpenAPI `style` keyword.
    #[must_use]
    pub fn style(&self) -> Option<ParameterStyle> {
        self.resolved()
            .get("style")
            .and_then(|n| n.as_str())
            .and_then(ParameterStyle::parse)
    }

    /// Explicit `explode` setting; `None` when left to the style default.
    #[must_use]
    pub fn explode(&self) -> Option<bool> {
        self.resolved().get("explode").and_then(|n| n.as_bool())
    }

    /// True when empty values (`?flag=`) are permitted; query-only per spec.
    #[must_use]
    pub fn allow_empty_value(&self) -> bool {
        self.resolved()
            .get("allowEmptyValue")
            .and_then(|n| n.as_bool())
            .unwrap_or(false)
    }
}

impl<'s> RequestBody<'s> {
    /// Prose describing what the request body carries.
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
    /// True when the request body must be present.
    #[must_use]
    pub fn required(&self) -> bool {
        self.resolved()
            .get("required")
            .and_then(|n| n.as_bool())
            .unwrap_or(false)
    }
    /// `(media-type, media-type-object)` payloads, in document order.
    #[must_use]
    pub fn content(&self) -> Vec<(&'s str, MediaType<'s>)> {
        named_map(self.session, self.resolved().get("content"), |s, n| {
            MediaType::new(s, n)
        })
    }
}

impl<'s> Response<'s> {
    /// Prose describing what this response returns.
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
    /// Headers returned with this response (never `Content-Type`, which
    /// lives in the media type).
    #[must_use]
    pub fn headers(&self) -> Vec<(&'s str, Header<'s>)> {
        named_map(self.session, self.resolved().get("headers"), Header::new)
    }
    /// Possible payload shapes keyed by media type.
    #[must_use]
    pub fn content(&self) -> Vec<(&'s str, MediaType<'s>)> {
        named_map(self.session, self.resolved().get("content"), |s, n| {
            MediaType::new(s, n)
        })
    }
    /// Operation links triggered by this response, in document order.
    #[must_use]
    pub fn links(&self) -> Vec<(&'s str, Link<'s>)> {
        named_map(self.session, self.resolved().get("links"), Link::new)
    }
}

impl<'s> Header<'s> {
    /// Prose describing the header's meaning.
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
    /// True when the header must be present in every response/request.
    #[must_use]
    pub fn required(&self) -> bool {
        self.resolved()
            .get("required")
            .and_then(|n| n.as_bool())
            .unwrap_or(false)
    }
    /// True when the header is explicitly marked deprecated.
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.resolved()
            .get("deprecated")
            .and_then(|n| n.as_bool())
            .unwrap_or(false)
    }
    /// Schema of the header value; headers ignore `in`-dependent styles.
    #[must_use]
    pub fn schema(&self) -> Option<SchemaView<'s>> {
        self.resolved()
            .get("schema")
            .map(|v| SchemaView::new(self.session, v))
    }
    /// Serialization style; see the OpenAPI `style` keyword.
    #[must_use]
    pub fn style(&self) -> Option<ParameterStyle> {
        self.resolved()
            .get("style")
            .and_then(|n| n.as_str())
            .and_then(ParameterStyle::parse)
    }
    /// Explicit `explode` setting; `None` when left to the style default.
    #[must_use]
    pub fn explode(&self) -> Option<bool> {
        self.resolved().get("explode").and_then(|n| n.as_bool())
    }
    /// Inline example value for the header.
    #[must_use]
    pub fn example(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("example")
    }
    /// Named [`Example`] objects for the header, in document order.
    #[must_use]
    pub fn examples(&self) -> Vec<(&'s str, Example<'s>)> {
        named_map(self.session, self.resolved().get("examples"), Example::new)
    }
}

impl<'s> Link<'s> {
    /// Runtime expression resolving to an operation (exclusive with
    /// [`Link::operation_id`] per the spec).
    #[must_use]
    pub fn operation_ref(&self) -> Option<&'s str> {
        self.resolved().get("operationRef").and_then(|n| n.as_str())
    }
    /// `operationId` of the linked operation (exclusive with
    /// [`Link::operation_ref`] per the spec).
    #[must_use]
    pub fn operation_id(&self) -> Option<&'s str> {
        self.resolved().get("operationId").and_then(|n| n.as_str())
    }
    /// Prose describing how the link relates to the current request.
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
    /// Literal or runtime-expression payload for the linked operation's
    /// request body.
    #[must_use]
    pub fn body(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("requestBody")
    }
}

impl<'s> Callback<'s> {
    /// `(expression, path-item)` pairs; expressions are arbitrary keys like
    /// `{$request.body#/url}`.
    #[must_use]
    pub fn expressions(&self) -> Vec<(&'s str, crate::paths::PathItem<'s>)> {
        self.node
            .entries()
            .into_iter()
            .filter_map(|e| {
                e.value
                    .map(|v| (e.key, crate::paths::PathItem::new(self.session, v)))
            })
            .collect()
    }
}

impl<'s> SecurityScheme<'s> {
    /// Typed view of the scheme's `type` plus its type-specific fields.
    /// `None` when `type` is missing or outside the four known kinds.
    #[must_use]
    pub fn scheme_type(&self) -> Option<SecuritySchemeType<'s>> {
        let r = self.resolved();
        let t = r.get("type").and_then(|n| n.as_str())?;
        Some(match t {
            "apiKey" => SecuritySchemeType::ApiKey {
                name: r.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                location: r.get("in").and_then(|n| n.as_str()).unwrap_or(""),
            },
            "http" => SecuritySchemeType::Http {
                scheme: r.get("scheme").and_then(|n| n.as_str()).unwrap_or(""),
                bearer_format: r.get("bearerFormat").and_then(|n| n.as_str()),
            },
            "openIdConnect" => SecuritySchemeType::OpenIdConnect {
                open_id_connect_url: r
                    .get("openIdConnectUrl")
                    .and_then(|n| n.as_str())
                    .unwrap_or(""),
            },
            "oauth2" => SecuritySchemeType::Oauth2 {
                flows: r.get("flows").map(|n| OauthFlows::new(self.session, n)),
            },
            _ => return None,
        })
    }
    /// Prose describing the security scheme.
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
}

impl<'s> OauthFlows<'s> {
    fn flow(&self, key: &str) -> Option<OauthFlow<'s>> {
        self.get(key).map(|n| OauthFlow::new(self.session, n))
    }
    /// OAuth implicit flow, when declared.
    #[must_use]
    pub fn implicit(&self) -> Option<OauthFlow<'s>> {
        self.flow("implicit")
    }
    /// OAuth resource-owner-password flow, when declared.
    #[must_use]
    pub fn password(&self) -> Option<OauthFlow<'s>> {
        self.flow("password")
    }
    /// OAuth client-credentials flow, when declared.
    #[must_use]
    pub fn client_credentials(&self) -> Option<OauthFlow<'s>> {
        self.flow("clientCredentials")
    }
    /// OAuth authorization-code flow, when declared.
    #[must_use]
    pub fn authorization_code(&self) -> Option<OauthFlow<'s>> {
        self.flow("authorizationCode")
    }
}

impl<'s> OauthFlow<'s> {
    /// URL used to obtain end-user authorization (implicit/code flows).
    #[must_use]
    pub fn authorization_url(&self) -> Option<&'s str> {
        self.get("authorizationUrl").and_then(|n| n.as_str())
    }
    /// Token endpoint URL (password/client-credentials/code flows).
    #[must_use]
    pub fn token_url(&self) -> Option<&'s str> {
        self.get("tokenUrl").and_then(|n| n.as_str())
    }
    /// Refresh-token endpoint URL shared by all flows that issue tokens.
    #[must_use]
    pub fn refresh_url(&self) -> Option<&'s str> {
        self.get("refreshUrl").and_then(|n| n.as_str())
    }
    /// `(scope-name, description)` pairs.
    #[must_use]
    pub fn scopes(&self) -> Vec<(&'s str, &'s str)> {
        self.get("scopes")
            .map(|n| {
                n.entries()
                    .into_iter()
                    .filter_map(|e| e.value.and_then(|v| v.as_str()).map(|s| (e.key, s)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl<'s> SecurityRequirement<'s> {
    /// `(scheme-name, scopes)` pairs.
    #[must_use]
    pub fn requirements(&self) -> Vec<(&'s str, Vec<&'s str>)> {
        self.node
            .entries()
            .into_iter()
            .map(|e| {
                (
                    e.key,
                    e.value
                        .map(|v| v.items().into_iter().filter_map(|i| i.as_str()).collect())
                        .unwrap_or_default(),
                )
            })
            .collect()
    }
}

/// Builds `(key, View)` pairs from a named-map node.
pub(crate) fn named_map<'s, T, F>(
    session: &'s Session,
    node: Option<NodeRef<'s>>,
    make: F,
) -> Vec<(&'s str, T)>
where
    F: Fn(&'s Session, NodeRef<'s>) -> T,
{
    node.map(|n| {
        n.entries()
            .into_iter()
            .filter_map(|e| e.value.map(|v| (e.key, make(session, v))))
            .collect()
    })
    .unwrap_or_default()
}

/// Where a parameter applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterIn {
    /// Query-string parameter (`?name=value`).
    Query,
    /// Request or response header.
    Header,
    /// Path-template segment (`/users/{id}`).
    Path,
    /// Cookie value.
    Cookie,
}

impl ParameterIn {
    /// Parses the `in` keyword spelling; `None` outside the four locations.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "query" => Some(Self::Query),
            "header" => Some(Self::Header),
            "path" => Some(Self::Path),
            "cookie" => Some(Self::Cookie),
            _ => None,
        }
    }

    /// The canonical lowercase keyword for this location.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Header => "header",
            Self::Path => "path",
            Self::Cookie => "cookie",
        }
    }
}

/// Serialization style for parameters/headers/encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterStyle {
    /// `form`: `id=1&id=2` style, default for query/cookie.
    Form,
    /// `simple`: comma-separated, default for path/header.
    Simple,
    /// `matrix`: semicolon-prefixed path parameters (`;id=1`).
    Matrix,
    /// `label`: dot-prefixed path parameters (`.id`).
    Label,
    /// `spaceDelimited`: space-separated array values.
    SpaceDelimited,
    /// `pipeDelimited`: pipe-separated array values.
    PipeDelimited,
    /// `deepObject`: nested query keys (`id[prop]=x`).
    DeepObject,
}

impl ParameterStyle {
    /// Parses the `style` keyword spelling; `None` for unknown styles.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "form" => Some(Self::Form),
            "simple" => Some(Self::Simple),
            "matrix" => Some(Self::Matrix),
            "label" => Some(Self::Label),
            "spaceDelimited" => Some(Self::SpaceDelimited),
            "pipeDelimited" => Some(Self::PipeDelimited),
            "deepObject" => Some(Self::DeepObject),
            _ => None,
        }
    }
}

/// Shape of a [`SecurityScheme`], pairing the `type` keyword with the
/// fields specific to that type. Field strings default to `""` when the
/// required sibling keyword is missing.
#[derive(Debug, Clone, Copy)]
pub enum SecuritySchemeType<'a> {
    /// `apiKey`: named credential in a request location.
    ApiKey {
        /// Parameter/header/cookie name carrying the key.
        name: &'a str,
        /// Where the key travels (`query`/`header`/`cookie`).
        location: &'a str,
    },
    /// `http`: standard `Authorization` header scheme.
    Http {
        /// RFC 7235 scheme (e.g. `basic`, `bearer`).
        scheme: &'a str,
        /// Hint for `bearer` token formats (e.g. `jwt`).
        bearer_format: Option<&'a str>,
    },
    /// `oauth2`: OAuth2 flows configuration.
    Oauth2 {
        /// The `flows` object; `None` when absent.
        flows: Option<OauthFlows<'a>>,
    },
    /// `openIdConnect`: OpenID Connect discovery.
    OpenIdConnect {
        /// URL of the OpenID Connect discovery document.
        open_id_connect_url: &'a str,
    },
}
