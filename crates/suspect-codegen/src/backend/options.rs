//! Explicit source interpretation and client defaults, independent of package identity.
use crate::http_protocol::{Capabilities, CompatibilityProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Explicit source interpretation and application-selected runtime defaults.
/// Empty options preserve existing behavior. Neither interpretation profiles nor
/// environment policy infer API behavior or enable unwitnessed native support.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationOptions {
    #[serde(default)]
    pub compatibility_profiles: BTreeSet<CompatibilityProfile>,
    /// Source scheme -> runtime variable names; values are never read by generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    /// Golden SDK behavior defaults; omitted fields resolve to the documented
    /// defaults. Participates in generation fingerprints like every other option.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_defaults: Option<crate::sdk_defaults::SdkDefaults>,
}

impl GenerationOptions {
    /// Add only interpretation choices to the adapter's own capability fence.
    #[must_use]
    pub fn apply_to(&self, capabilities: Capabilities) -> Capabilities {
        self.compatibility_profiles
            .iter()
            .fold(capabilities, |capabilities, profile| {
                capabilities.with_profile(*profile)
            })
    }

    #[must_use]
    pub(crate) fn legacy_binary_strings(&self) -> bool {
        self.compatibility_profiles
            .contains(&CompatibilityProfile::LegacyBinaryStringV1)
    }
}

pub(crate) fn typescript_options(
    options: &GenerationOptions,
    attribution: Option<&crate::attribution::AttributionDescriptor>,
) -> crate::typescript::http::HttpConfig {
    crate::typescript::http::HttpConfig {
        legacy_binary_string: options.legacy_binary_strings(),
        #[cfg(feature = "http-protocol")]
        compatibility_profiles: options.compatibility_profiles.iter().copied().collect(),
        credential_env: options.credential_env.clone(),
        sdk_defaults: options.sdk_defaults.clone(),
        attribution: attribution.cloned(),
        ..crate::typescript::http::HttpConfig::expanded()
    }
}

pub(crate) fn rust_options(
    options: &GenerationOptions,
    attribution: Option<&crate::attribution::AttributionDescriptor>,
) -> crate::rust_http::HttpConfig {
    crate::rust_http::HttpConfig {
        compatibility_profiles: options.compatibility_profiles.iter().copied().collect(),
        credential_env: options.credential_env.clone(),
        sdk_defaults: options.sdk_defaults.clone(),
        attribution: attribution.cloned(),
        ..Default::default()
    }
}

pub(crate) fn python_options(
    options: &GenerationOptions,
    attribution: Option<&crate::attribution::AttributionDescriptor>,
) -> crate::python_http::HttpConfig {
    crate::python_http::HttpConfig {
        capabilities: options.apply_to(crate::python_http::capabilities()),
        credential_env: options.credential_env.clone(),
        sdk_defaults: options.sdk_defaults.clone(),
        attribution: attribution.cloned(),
        ..Default::default()
    }
}

pub(crate) fn go_options(
    options: &GenerationOptions,
    attribution: Option<&crate::attribution::AttributionDescriptor>,
) -> crate::go_http::HttpConfig {
    crate::go_http::HttpConfig {
        compatibility_profiles: options.compatibility_profiles.clone(),
        credential_env: options.credential_env.clone(),
        sdk_defaults: options.sdk_defaults.clone(),
        attribution: attribution.cloned(),
        ..Default::default()
    }
}

pub(crate) fn swift_options(
    options: &GenerationOptions,
    attribution: Option<&crate::attribution::AttributionDescriptor>,
) -> crate::swift_sdk::SwiftConfig {
    crate::swift_sdk::SwiftConfig {
        compatibility_profiles: options.compatibility_profiles.clone(),
        credential_env: options.credential_env.clone(),
        sdk_defaults: options.sdk_defaults.clone(),
        attribution: attribution.cloned(),
        ..Default::default()
    }
}

#[cfg(feature = "ruby-sdk")]
pub(crate) fn ruby_options(
    options: &GenerationOptions,
    attribution: Option<&crate::attribution::AttributionDescriptor>,
) -> crate::ruby_sdk::RubyConfig {
    crate::ruby_sdk::RubyConfig {
        legacy_binary_strings: options.legacy_binary_strings(),
        credential_env: options.credential_env.clone(),
        sdk_defaults: options.sdk_defaults.clone(),
        attribution: attribution.cloned(),
        ..Default::default()
    }
}

#[cfg(feature = "csharp-sdk")]
pub(crate) fn csharp_options(
    options: &GenerationOptions,
    attribution: Option<&crate::attribution::AttributionDescriptor>,
) -> crate::csharp_sdk::protocol::ProtocolOptions {
    crate::csharp_sdk::protocol::ProtocolOptions {
        compatibility_profiles: options.compatibility_profiles.iter().copied().collect(),
        credential_env: options.credential_env.clone(),
        sdk_defaults: options.sdk_defaults.clone(),
        attribution: attribution.cloned(),
    }
}

#[cfg(feature = "java-sdk")]
pub(crate) fn java_options(
    options: &GenerationOptions,
    attribution: Option<&crate::attribution::AttributionDescriptor>,
) -> crate::java_sdk::ProtocolConfig {
    crate::java_sdk::ProtocolConfig {
        compatibility_profiles: options.compatibility_profiles.clone(),
        credential_env: options.credential_env.clone(),
        sdk_defaults: options.sdk_defaults.clone(),
        attribution: attribution.cloned(),
        ..Default::default()
    }
}
