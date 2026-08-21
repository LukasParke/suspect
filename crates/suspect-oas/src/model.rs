use suspect_low::NodeRef;

use crate::session::{CycleGuard, Session};
use crate::SchemaView;

/// Common view plumbing: session + home doc + raw node.
macro_rules! view {
    ($name:ident) => {
        // `session`/`get` are part of the uniform view plumbing; non-ref
        // views never touch them, hence the allow.
        #[allow(dead_code)]
        #[derive(Debug, Clone, Copy)]
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

view!(Info);
view!(Contact);
view!(License);
view!(Server);
view!(ServerVariable);
view!(Tag);
view!(ExternalDocumentation);
view!(Xml);
view!(Discriminator);
view!(Encoding);
view!(Example);
ref_view!(Example);
view!(MediaType);
view!(Header);
ref_view!(Header);
view!(Parameter);
ref_view!(Parameter);
view!(RequestBody);
ref_view!(RequestBody);
view!(Response);
ref_view!(Response);
view!(Link);
ref_view!(Link);
view!(Callback);
view!(SecurityScheme);
ref_view!(SecurityScheme);
view!(OauthFlows);
view!(OauthFlow);
view!(SecurityRequirement);

impl<'s> Info<'s> {
    #[must_use]
    pub fn title(&self) -> Option<&'s str> {
        self.get("title").and_then(|n| n.as_str())
    }

    /// 3.2+ summary.
    #[must_use]
    pub fn summary(&self) -> Option<&'s str> {
        self.get("summary").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn version(&self) -> Option<&'s str> {
        self.get("version").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn terms_of_service(&self) -> Option<&'s str> {
        self.get("termsOfService").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn contact(&self) -> Option<Contact<'s>> {
        self.get("contact").map(|n| Contact::new(self.session, n))
    }

    #[must_use]
    pub fn license(&self) -> Option<License<'s>> {
        self.get("license").map(|n| License::new(self.session, n))
    }
}

impl<'s> Contact<'s> {
    #[must_use]
    pub fn name(&self) -> Option<&'s str> {
        self.get("name").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn url(&self) -> Option<&'s str> {
        self.get("url").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn email(&self) -> Option<&'s str> {
        self.get("email").and_then(|n| n.as_str())
    }
}

impl<'s> License<'s> {
    #[must_use]
    pub fn name(&self) -> Option<&'s str> {
        self.get("name").and_then(|n| n.as_str())
    }
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
    pub fn url(&self) -> Option<&'s str> {
        self.get("url").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn variables(&self) -> Vec<(&'s str, ServerVariable<'s>)> {
        self.get("variables")
            .map(|n| {
                n.entries()
                    .into_iter()
                    .filter_map(|e| e.value.map(|v| (e.key, ServerVariable::new(self.session, v))))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl<'s> ServerVariable<'s> {
    #[must_use]
    pub fn default_value(&self) -> Option<&'s str> {
        self.get("default").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn enum_values(&self) -> Vec<&'s str> {
        self.get("enum").map(|n| n.items().into_iter().filter_map(|i| i.as_str()).collect()).unwrap_or_default()
    }
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }
}

impl<'s> Tag<'s> {
    #[must_use]
    pub fn name(&self) -> Option<&'s str> {
        self.get("name").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn external_docs(&self) -> Option<ExternalDocumentation<'s>> {
        self.get("externalDocs").map(|n| ExternalDocumentation::new(self.session, n))
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
    pub fn description(&self) -> Option<&'s str> {
        self.get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn url(&self) -> Option<&'s str> {
        self.get("url").and_then(|n| n.as_str())
    }
}

impl<'s> Xml<'s> {
    #[must_use]
    pub fn name(&self) -> Option<&'s str> {
        self.get("name").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn namespace(&self) -> Option<&'s str> {
        self.get("namespace").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn prefix(&self) -> Option<&'s str> {
        self.get("prefix").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn attribute(&self) -> bool {
        self.get("attribute").and_then(|n| n.as_bool()).unwrap_or(false)
    }
    #[must_use]
    pub fn wrapped(&self) -> bool {
        self.get("wrapped").and_then(|n| n.as_bool()).unwrap_or(false)
    }
}

impl<'s> Discriminator<'s> {
    #[must_use]
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
    pub fn content_type(&self) -> Option<&'s str> {
        self.get("contentType").and_then(|n| n.as_str())
    }
    #[must_use]
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
    pub fn style(&self) -> Option<ParameterStyle> {
        self.get("style").and_then(|n| n.as_str()).and_then(ParameterStyle::parse)
    }
    #[must_use]
    pub fn explode(&self) -> Option<bool> {
        self.get("explode").and_then(|n| n.as_bool())
    }
    #[must_use]
    pub fn allow_reserved(&self) -> bool {
        self.get("allowReserved").and_then(|n| n.as_bool()).unwrap_or(false)
    }
}

impl<'s> Example<'s> {
    #[must_use]
    pub fn summary(&self) -> Option<&'s str> {
        self.resolved().get("summary").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn value(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("value")
    }
    #[must_use]
    pub fn external_value(&self) -> Option<&'s str> {
        self.resolved().get("externalValue").and_then(|n| n.as_str())
    }
}

impl<'s> MediaType<'s> {

    #[must_use]
    pub fn schema(&self) -> Option<SchemaView<'s>> {
        self.get("schema").map(|v| SchemaView::new(self.session, v))
    }

    #[must_use]
    pub fn example(&self) -> Option<NodeRef<'s>> {
        self.get("example")
    }

    #[must_use]
    pub fn examples(&self) -> Vec<(&'s str, Example<'s>)> {
        named_map(self.session, self.get("examples"), Example::new)
    }

    #[must_use]
    pub fn encoding(&self) -> Vec<(&'s str, Encoding<'s>)> {
        named_map(self.session, self.get("encoding"), Encoding::new)
    }
}

impl<'s> Parameter<'s> {
    #[must_use]
    pub fn name(&self) -> Option<&'s str> {
        self.resolved().get("name").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn location(&self) -> Option<ParameterIn> {
        self.resolved().get("in").and_then(|n| n.as_str()).and_then(ParameterIn::parse)
    }

    #[must_use]
    pub fn required(&self) -> bool {
        let r = self.resolved();
        let in_path = r.get("in").and_then(|n| n.as_str()) == Some("path");
        r.get("required").and_then(|n| n.as_bool()).unwrap_or(in_path)
    }

    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.resolved().get("deprecated").and_then(|n| n.as_bool()).unwrap_or(false)
    }

    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn schema(&self) -> Option<SchemaView<'s>> {
        self.resolved().get("schema").map(|v| SchemaView::new(self.session, v))
    }

    /// Content-style parameter payload: `(media-type, media-type-object)`.
    #[must_use]
    pub fn content(&self) -> Vec<(&'s str, MediaType<'s>)> {
        named_map(self.session, self.resolved().get("content"), |s, n| {
            MediaType::new(s, n)
        })
    }

    #[must_use]
    pub fn example(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("example")
    }

    #[must_use]
    pub fn examples(&self) -> Vec<(&'s str, Example<'s>)> {
        named_map(self.session, self.resolved().get("examples"), Example::new)
    }

    #[must_use]
    pub fn style(&self) -> Option<ParameterStyle> {
        self.resolved().get("style").and_then(|n| n.as_str()).and_then(ParameterStyle::parse)
    }

    #[must_use]
    pub fn explode(&self) -> Option<bool> {
        self.resolved().get("explode").and_then(|n| n.as_bool())
    }

    #[must_use]
    pub fn allow_empty_value(&self) -> bool {
        self.resolved().get("allowEmptyValue").and_then(|n| n.as_bool()).unwrap_or(false)
    }
}

impl<'s> RequestBody<'s> {
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn required(&self) -> bool {
        self.resolved().get("required").and_then(|n| n.as_bool()).unwrap_or(false)
    }
    #[must_use]
    pub fn content(&self) -> Vec<(&'s str, MediaType<'s>)> {
        named_map(self.session, self.resolved().get("content"), |s, n| {
            MediaType::new(s, n)
        })
    }
}

impl<'s> Response<'s> {
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn headers(&self) -> Vec<(&'s str, Header<'s>)> {
        named_map(self.session, self.resolved().get("headers"), Header::new)
    }
    #[must_use]
    pub fn content(&self) -> Vec<(&'s str, MediaType<'s>)> {
        named_map(self.session, self.resolved().get("content"), |s, n| {
            MediaType::new(s, n)
        })
    }
    #[must_use]
    pub fn links(&self) -> Vec<(&'s str, Link<'s>)> {
        named_map(self.session, self.resolved().get("links"), Link::new)
    }
}

impl<'s> Header<'s> {
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn required(&self) -> bool {
        self.resolved().get("required").and_then(|n| n.as_bool()).unwrap_or(false)
    }
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.resolved().get("deprecated").and_then(|n| n.as_bool()).unwrap_or(false)
    }
    #[must_use]
    pub fn schema(&self) -> Option<SchemaView<'s>> {
        self.resolved().get("schema").map(|v| SchemaView::new(self.session, v))
    }
    #[must_use]
    pub fn style(&self) -> Option<ParameterStyle> {
        self.resolved().get("style").and_then(|n| n.as_str()).and_then(ParameterStyle::parse)
    }
    #[must_use]
    pub fn explode(&self) -> Option<bool> {
        self.resolved().get("explode").and_then(|n| n.as_bool())
    }
    #[must_use]
    pub fn example(&self) -> Option<NodeRef<'s>> {
        self.resolved().get("example")
    }
    #[must_use]
    pub fn examples(&self) -> Vec<(&'s str, Example<'s>)> {
        named_map(self.session, self.resolved().get("examples"), Example::new)
    }
}

impl<'s> Link<'s> {
    #[must_use]
    pub fn operation_ref(&self) -> Option<&'s str> {
        self.resolved().get("operationRef").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn operation_id(&self) -> Option<&'s str> {
        self.resolved().get("operationId").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
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
            .filter_map(|e| e.value.map(|v| (e.key, crate::paths::PathItem::new(self.session, v))))
            .collect()
    }
}

impl<'s> SecurityScheme<'s> {
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
                open_id_connect_url: r.get("openIdConnectUrl").and_then(|n| n.as_str()).unwrap_or(""),
            },
            "oauth2" => SecuritySchemeType::Oauth2 { flows: r.get("flows").map(|n| OauthFlows::new(self.session, n)) },
            _ => return None,
        })
    }
    #[must_use]
    pub fn description(&self) -> Option<&'s str> {
        self.resolved().get("description").and_then(|n| n.as_str())
    }
}

impl<'s> OauthFlows<'s> {
    fn flow(&self, key: &str) -> Option<OauthFlow<'s>> {
        self.get(key).map(|n| OauthFlow::new(self.session, n))
    }
    #[must_use]
    pub fn implicit(&self) -> Option<OauthFlow<'s>> {
        self.flow("implicit")
    }
    #[must_use]
    pub fn password(&self) -> Option<OauthFlow<'s>> {
        self.flow("password")
    }
    #[must_use]
    pub fn client_credentials(&self) -> Option<OauthFlow<'s>> {
        self.flow("clientCredentials")
    }
    #[must_use]
    pub fn authorization_code(&self) -> Option<OauthFlow<'s>> {
        self.flow("authorizationCode")
    }
}

impl<'s> OauthFlow<'s> {
    #[must_use]
    pub fn authorization_url(&self) -> Option<&'s str> {
        self.get("authorizationUrl").and_then(|n| n.as_str())
    }
    #[must_use]
    pub fn token_url(&self) -> Option<&'s str> {
        self.get("tokenUrl").and_then(|n| n.as_str())
    }
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
                    e.value.map(|v| v.items().into_iter().filter_map(|i| i.as_str()).collect()).unwrap_or_default(),
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
    Query,
    Header,
    Path,
    Cookie,
}

impl ParameterIn {
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
    Form,
    Simple,
    Matrix,
    Label,
    SpaceDelimited,
    PipeDelimited,
    DeepObject,
}

impl ParameterStyle {
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

#[derive(Debug, Clone, Copy)]
pub enum SecuritySchemeType<'a> {
    ApiKey { name: &'a str, location: &'a str },
    Http { scheme: &'a str, bearer_format: Option<&'a str> },
    Oauth2 { flows: Option<OauthFlows<'a>> },
    OpenIdConnect { open_id_connect_url: &'a str },
}

