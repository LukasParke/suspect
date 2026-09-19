//! `ua/v1` attribution: the programmatically built User-Agent that identifies
//! every generated package as suspect-generated and separates raw SDK usage
//! from identified application adoption in API traffic.
//!
//! Constant parts are compiled at generation time; the language version is
//! discovered at client construction by each native runtime. This module never
//! reads an environment variable, a file, or a remote resource.

use serde::{Deserialize, Serialize};

/// Closed version of the attribution grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttributionTemplateVersion {
    #[serde(rename = "v1")]
    V1,
}

/// RFC 9110 `tchar` set: the characters allowed in a User-Agent token.
const TOKEN_EXTRA: &[u8] = b"!#$%&'*+-.^_`|~";

/// True when every byte is an RFC 9110 token character.
#[must_use]
pub fn is_token(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || TOKEN_EXTRA.contains(byte))
}

/// Replace disallowed characters so a package identity stays one token.
#[must_use]
fn sanitize_token(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_dash = false;
    for character in value.chars() {
        let keep = character.is_ascii_alphanumeric() || TOKEN_EXTRA.contains(&(character as u8));
        if keep {
            out.push(character);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Validate one RFC 9110 token with the bounded length policy.
///
/// # Errors
/// Empty values, values above 128 bytes, or values with non-token characters.
pub fn validate_token(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > 128 {
        return Err(format!("{kind} must be at most 128 bytes"));
    }
    if !is_token(value) {
        return Err(format!(
            "{kind} must be an RFC 9110 token (ASCII letters, digits, and !#$%&'*+-.^_`|~)"
        ));
    }
    Ok(())
}

/// One client-supplied application identifier in `<name>[/<version>]` form.
/// It replaces the SDK identity token so adopted traffic is separable while
/// every request still carries the suspect generator token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationIdentity {
    /// The full identity segment, for example `acme-chatbot` or `acme-chatbot/7`.
    pub token: String,
}

impl ApplicationIdentity {
    /// Validate the identifier against the RFC 9110 product grammar: at most
    /// one `/` separating two non-empty tokens.
    ///
    /// # Errors
    /// Empty values, values above 128 bytes, more than one `/`, or non-token
    /// characters. An empty value means unset and must be represented by
    /// `None`, never by `""`.
    pub fn validate(&self) -> Result<(), String> {
        if self.token.is_empty() {
            return Err("application identifier must not be empty".into());
        }
        if self.token.len() > 128 {
            return Err("application identifier must be at most 128 bytes".into());
        }
        let mut parts = self.token.split('/');
        let name = parts.next().unwrap_or_default();
        let version = parts.next();
        if parts.next().is_some() {
            return Err(format!(
                "application identifier {:?} must be <name> or <name>/<version>",
                self.token
            ));
        }
        validate_token("application identifier name", name).map_err(|error| {
            format!("{error}; use <name> or <name>/<version>, e.g. \"acme-chatbot/7\"")
        })?;
        if let Some(version) = version {
            if version.is_empty() {
                return Err(format!(
                    "application identifier {:?} needs a version after '/' or none at all",
                    self.token
                ));
            }
            validate_token("application identifier version", version)?;
        }
        Ok(())
    }
}

/// Compiled attribution values for one generated package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributionDescriptor {
    pub template_version: AttributionTemplateVersion,
    /// Suspect generator version captured at generation time.
    pub suspect_version: String,
    pub sdk_name: String,
    pub sdk_version: String,
    /// Source document's declared OpenAPI or Swagger version.
    pub spec_version: String,
    /// Per-backend language tag (`python`, `typescript`, `go`, ...).
    pub language: String,
}

impl AttributionDescriptor {
    /// Compile the descriptor for one target. Package identities that are not
    /// valid RFC 9110 tokens (Go module paths, scoped npm names) are sanitized
    /// deterministically; the sanitized form is what the grammar emits.
    #[must_use]
    pub fn plan(
        suspect_version: &str,
        sdk_name: &str,
        sdk_version: &str,
        spec_version: &str,
        language: &str,
    ) -> Self {
        let mut sdk_name = sanitize_token(sdk_name);
        if sdk_name.is_empty() {
            sdk_name = "sdk".into();
        }
        Self {
            template_version: AttributionTemplateVersion::V1,
            suspect_version: suspect_version.into(),
            sdk_name,
            sdk_version: sdk_version.into(),
            spec_version: spec_version.into(),
            language: language.into(),
        }
    }

    /// `<identity>/<identity-version>`: the SDK package identity, or the
    /// client-supplied application identifier when one is configured.
    #[must_use]
    pub fn identity_token(&self, application_id: Option<&ApplicationIdentity>) -> String {
        match application_id {
            Some(identity) => identity.token.clone(),
            None => format!("{}/{}", self.sdk_name, self.sdk_version),
        }
    }

    /// The constant prefix through the identity token, before the runtime
    /// language-version comment.
    #[must_use]
    pub fn user_agent_base(&self, application_id: Option<&ApplicationIdentity>) -> String {
        format!(
            "suspect/{} {}",
            self.suspect_version,
            self.identity_token(application_id)
        )
    }

    /// The constant document-version segment inside the trailing comment.
    #[must_use]
    pub fn user_agent_tail(&self) -> String {
        format!("openapi/{}", self.spec_version)
    }

    /// Assemble the complete default User-Agent for a runtime-discovered
    /// language version. Native runtimes reproduce this assembly with their
    /// own idiom; this function is the normative fixture.
    #[must_use]
    pub fn user_agent(
        &self,
        application_id: Option<&ApplicationIdentity>,
        language_version: &str,
    ) -> String {
        format!(
            "{} ({}/{}; {})",
            self.user_agent_base(application_id),
            self.language,
            language_version,
            self.user_agent_tail(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> AttributionDescriptor {
        AttributionDescriptor::plan("0.42.1", "openrouter-python", "1.2.0", "3.1.0", "python")
    }

    #[test]
    fn default_header_identifies_suspect_and_sdk() {
        let value = descriptor().user_agent(None, "3.12.1");
        assert_eq!(
            value,
            "suspect/0.42.1 openrouter-python/1.2.0 (python/3.12.1; openapi/3.1.0)"
        );
    }

    #[test]
    fn application_identity_replaces_the_sdk_identity_token() {
        let identity = ApplicationIdentity {
            token: "acme-chatbot/7".into(),
        };
        let value = descriptor().user_agent(Some(&identity), "3.12.1");
        assert_eq!(
            value,
            "suspect/0.42.1 acme-chatbot/7 (python/3.12.1; openapi/3.1.0)"
        );
    }

    #[test]
    fn go_module_paths_are_sanitized_into_tokens() {
        let descriptor = AttributionDescriptor::plan(
            "0.42.1",
            "github.com/org/repo/sdk",
            "1.0.0",
            "3.0.0",
            "go",
        );
        assert_eq!(descriptor.sdk_name, "github.com-org-repo-sdk");
        assert_eq!(
            descriptor.user_agent(None, "go1.22"),
            "suspect/0.42.1 github.com-org-repo-sdk/1.0.0 (go/go1.22; openapi/3.0.0)"
        );
    }

    #[test]
    fn unknown_language_version_degrades_without_omission() {
        let value = descriptor().user_agent(None, "unknown");
        assert_eq!(
            value,
            "suspect/0.42.1 openrouter-python/1.2.0 (python/unknown; openapi/3.1.0)"
        );
    }

    #[test]
    fn application_identifiers_reject_non_tokens() {
        for invalid in ["", "acme chatbot", "café/1", "a\tb"] {
            let identity = ApplicationIdentity {
                token: invalid.into(),
            };
            assert!(identity.validate().is_err(), "{invalid:?} must be refused");
        }
        for valid in ["acme-chatbot/7", "acme_chatbot.v2"] {
            let identity = ApplicationIdentity {
                token: valid.into(),
            };
            assert!(identity.validate().is_ok(), "{valid:?} must be accepted");
        }
    }

    #[test]
    fn swagger_versions_are_carried_verbatim() {
        let descriptor =
            AttributionDescriptor::plan("0.42.1", "legacy-swift", "2.0.0", "2.0", "swift");
        assert_eq!(
            descriptor.user_agent(None, "5.9"),
            "suspect/0.42.1 legacy-swift/2.0.0 (swift/5.9; openapi/2.0)"
        );
    }
}
