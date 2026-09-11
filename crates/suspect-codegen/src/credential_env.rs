//! Explicit runtime environment defaults bound to admitted source security schemes.
//! This module never reads an environment variable, a file, or a remote resource.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, de};
use suspect_ir::contract::{Contract, SourceId};

pub use crate::http_contract::HttpDiagnostic;
use crate::http_protocol::{CredentialHook, ProtocolPlan, Provenance, SourceLocation};

/// Closed version of the application credential-loading policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialEnvVersion {
    #[serde(rename = "v1")]
    V1,
}

/// Variable names selected explicitly by source Security Scheme name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialEnv {
    pub version: CredentialEnvVersion,
    pub schemes: BTreeMap<String, String>,
}

impl CredentialEnv {
    #[must_use]
    pub fn v1(schemes: BTreeMap<String, String>) -> Self {
        Self {
            version: CredentialEnvVersion::V1,
            schemes,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.schemes.is_empty() || self.schemes.len() > 64 {
            return Err("credential_env.schemes must contain 1..=64 source scheme mappings".into());
        }
        for (name, variable) in &self.schemes {
            if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return Err(
                    "credential_env source scheme names must be 1..=256 bytes without control characters"
                        .into(),
                );
            }
            let bytes = variable.as_bytes();
            let valid = bytes.len() <= 128
                && bytes
                    .first()
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
            if !valid {
                return Err(format!(
                    "credential_env.schemes[{name:?}] needs a portable 1..=128 byte environment variable name"
                ));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for CredentialEnv {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Configuration {
            version: CredentialEnvVersion,
            #[serde(deserialize_with = "unique_schemes")]
            schemes: BTreeMap<String, String>,
        }
        let input = Configuration::deserialize(deserializer)?;
        let config = Self {
            version: input.version,
            schemes: input.schemes,
        };
        config.validate().map_err(de::Error::custom)?;
        Ok(config)
    }
}

fn unique_schemes<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    struct Schemes;
    impl<'de> de::Visitor<'de> for Schemes {
        type Value = BTreeMap<String, String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .write_str("an object of unique source scheme names and environment variable names")
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
            let mut schemes = BTreeMap::new();
            while let Some((name, variable)) = input.next_entry::<String, String>()? {
                if schemes.insert(name, variable).is_some() {
                    return Err(de::Error::custom(
                        "duplicate credential_env source scheme mapping",
                    ));
                }
                if schemes.len() > 64 {
                    return Err(de::Error::custom(
                        "credential_env.schemes exceeds 64 mappings",
                    ));
                }
            }
            Ok(schemes)
        }
    }
    deserializer.deserialize_map(Schemes)
}

/// String-valued native attachment shapes supported by policy v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialEnvKind {
    Bearer,
    ApiKey,
}

/// A variable name and the actual declaration to which it is bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialEnvBinding {
    name: String,
    variable: String,
    kind: CredentialEnvKind,
    scheme: Provenance,
}

impl CredentialEnvBinding {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn variable(&self) -> &str {
        &self.variable
    }

    #[must_use]
    pub fn kind(&self) -> CredentialEnvKind {
        self.kind
    }

    #[must_use]
    pub fn scheme(&self) -> &Provenance {
        &self.scheme
    }
}

/// Typed client-default metadata, independent of physical source relocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialEnvDescriptor {
    pub version: CredentialEnvVersion,
    pub bindings: Vec<CredentialEnvDescriptorBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialEnvDescriptorBinding {
    pub name: String,
    pub variable: String,
    pub kind: CredentialEnvKind,
}

/// Immutable result consumed by native emitters and their compatibility capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialEnvPlan {
    version: CredentialEnvVersion,
    bindings: Vec<CredentialEnvBinding>,
}

impl CredentialEnvPlan {
    #[must_use]
    pub fn version(&self) -> CredentialEnvVersion {
        self.version
    }

    #[must_use]
    pub fn bindings(&self) -> &[CredentialEnvBinding] {
        &self.bindings
    }

    /// Capture typed values without making provenance part of interface equality.
    #[must_use]
    pub fn semantic_descriptor(&self) -> CredentialEnvDescriptor {
        CredentialEnvDescriptor {
            version: self.version,
            bindings: self
                .bindings
                .iter()
                .map(|binding| CredentialEnvDescriptorBinding {
                    name: binding.name.clone(),
                    variable: binding.variable.clone(),
                    kind: binding.kind,
                })
                .collect(),
        }
    }
}

fn diagnostic(
    contract: &Contract,
    location: Option<&SourceLocation>,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    let source = location.map_or_else(
        || SourceId::new(contract.entry().clone(), Default::default()),
        |location| location.source().clone(),
    );
    let at = location.map_or_else(
        || contract.source_span(&source).unwrap_or(0..0),
        SourceLocation::span,
    );
    HttpDiagnostic {
        source,
        at,
        code,
        message: message.into(),
    }
}

/// Bind configured variable names through an already admitted canonical protocol.
/// Repeated requirements for one declaration share one binding; names ambiguous
/// across declaration documents are refused instead of selecting a document.
///
/// # Errors
/// Unadmitted protocol plans, unbound/ambiguous source names, or unsupported
/// credential attachment kinds. No runtime credential values enter this interface.
pub fn plan(
    contract: &Contract,
    protocol: &ProtocolPlan,
    config: Option<&CredentialEnv>,
) -> Result<Option<CredentialEnvPlan>, Vec<HttpDiagnostic>> {
    let Some(config) = config else {
        return Ok(None);
    };
    if let Err(message) = config.validate() {
        return Err(vec![diagnostic(
            contract,
            None,
            "sdk-credential-env-config",
            message,
        )]);
    }
    if !protocol.is_admitted() {
        return Err(vec![diagnostic(
            contract,
            None,
            "sdk-credential-env-protocol",
            "credential_env requires an admitted canonical HTTP protocol plan",
        )]);
    }
    let mut bindings = Vec::new();
    let mut errors = Vec::new();
    for (name, variable) in &config.schemes {
        let declarations = protocol
            .operations()
            .iter()
            .flat_map(|operation| operation.security().alternatives())
            .flat_map(|alternative| alternative.requirements())
            .filter(|requirement| requirement.name() == name)
            .map(|requirement| (requirement.scheme().use_site().source(), requirement))
            .collect::<BTreeMap<_, _>>();
        if declarations.len() != 1 {
            errors.push(diagnostic(
                contract,
                declarations.values().next().map(|r| r.scheme().use_site()),
                if declarations.is_empty() {
                    "sdk-credential-env-unbound"
                } else {
                    "sdk-credential-env-ambiguous"
                },
                format!(
                    "credential_env.schemes[{name:?}] must identify exactly one used source Security Scheme declaration; found {}",
                    declarations.len()
                ),
            ));
            continue;
        }
        let requirement = declarations.values().next().expect("one declaration");
        let kind = match requirement.credential() {
            CredentialHook::Bearer { .. } => CredentialEnvKind::Bearer,
            CredentialHook::ApiKey { .. } => CredentialEnvKind::ApiKey,
            _ => {
                errors.push(diagnostic(
                    contract,
                    Some(requirement.scheme().terminal()),
                    "sdk-credential-env-kind",
                    format!(
                        "credential_env.schemes[{name:?}] requires an HTTP bearer or API-key string attachment in v1"
                    ),
                ));
                continue;
            }
        };
        bindings.push(CredentialEnvBinding {
            name: name.clone(),
            variable: variable.clone(),
            kind,
            scheme: requirement.scheme().clone(),
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Some(CredentialEnvPlan {
        version: config.version,
        bindings,
    }))
}
