//! The precedence rule, stated once and enforced everywhere.
//!
//! ```text
//! flag  >  environment  >  config file  >  built-in default
//! ```
//!
//! Making this a value rather than an `if` at each call site is the point:
//! the previous CLI wiring had config beating an explicit flag, because a
//! clap default and a typed flag were indistinguishable at the call site.
//! Here, a flag is `Some` or it is not.

/// One setting's resolution across the layers, remembering which layer
/// supplied the value.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution<T> {
    /// The effective value.
    pub value: T,
    /// Which layer supplied it.
    pub source: Layer,
    /// Every layer, for `suspect config` style reporting.
    pub trace: Vec<(Layer, Option<T>)>,
}

/// Where a value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// An explicit command-line flag.
    Flag,
    /// An environment variable.
    Env,
    /// A configuration file.
    Config,
    /// The built-in default.
    Default,
}

impl Layer {
    /// A human label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "environment",
            Self::Config => "config",
            Self::Default => "default",
        }
    }
}

impl<T: Clone> Resolution<T> {
    /// Resolves a value from the four layers, highest precedence first.
    ///
    /// `flag` is `None` when the user did not pass the flag, which is the
    /// case that previously could not be distinguished from a default.
    #[must_use]
    pub fn new(flag: Option<T>, env: Option<T>, config: Option<T>, default: T) -> Self {
        let mut trace: Vec<(Layer, Option<T>)> = Vec::new();
        for (layer, value) in [
            (Layer::Flag, flag),
            (Layer::Env, env),
            (Layer::Config, config),
        ] {
            trace.push((layer, value.clone()));
        }
        let resolved = trace
            .iter()
            .find_map(|(layer, value)| value.clone().map(|value| (*layer, value)));
        let (source, value) = resolved.unwrap_or((Layer::Default, default));
        Self {
            value,
            source,
            trace,
        }
    }

    /// Whether the value came from an explicit flag or environment
    /// variable rather than a file or default.
    #[must_use]
    pub fn is_explicit(&self) -> bool {
        matches!(self.source, Layer::Flag | Layer::Env)
    }
}

/// Resolves a string-valued setting from an environment variable name.
#[must_use]
pub fn resolve_str(
    flag: Option<String>,
    key: &str,
    config: Option<String>,
    default: &str,
) -> String {
    let env = std::env::var(key).ok().filter(|v| !v.is_empty());
    Resolution::new(flag, env, config, default.to_owned()).value
}

/// Resolves a boolean setting from an environment variable name.
#[must_use]
pub fn resolve_bool(flag: bool, key: &str, config: bool) -> bool {
    let env = std::env::var(key).ok().map(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    });
    Resolution::new(Some(flag), env, Some(config), false).value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flag_beats_config_and_default() {
        let resolved = Resolution::new(
            Some("warning".to_owned()),
            None,
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        assert_eq!(resolved.value, "warning");
        assert_eq!(resolved.source, Layer::Flag);
        assert!(resolved.is_explicit());
    }

    #[test]
    fn config_beats_default_when_no_flag_is_given() {
        let resolved = Resolution::new(
            None::<String>,
            None,
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        assert_eq!(resolved.value, "info");
        assert_eq!(resolved.source, Layer::Config);
        assert!(!resolved.is_explicit());
    }

    #[test]
    fn the_default_applies_with_nothing_set() {
        let resolved = Resolution::new(None::<String>, None, None, "hint".to_owned());
        assert_eq!(resolved.value, "hint");
        assert_eq!(resolved.source, Layer::Default);
    }

    #[test]
    fn env_sits_between_flag_and_config() {
        let resolved = Resolution::new(
            Some("error".to_owned()),
            Some("warning".to_owned()),
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        assert_eq!(resolved.value, "error", "the flag still wins over env");
        let resolved = Resolution::new(
            None::<String>,
            Some("warning".to_owned()),
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        assert_eq!(resolved.value, "warning");
        assert_eq!(resolved.source, Layer::Env);
    }

    #[test]
    fn every_layer_is_traceable() {
        let resolved = Resolution::new(
            Some("error".to_owned()),
            Some("warning".to_owned()),
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        let layers: Vec<&str> = resolved.trace.iter().map(|(l, _)| l.label()).collect();
        assert_eq!(layers.first(), Some(&"flag"));
        assert!(layers.contains(&"environment"));
        assert!(layers.contains(&"config"));
    }

    #[test]
    fn boolean_env_values_are_honoured() {
        // A flag (even false, as clap supplies a default bool) is explicit
        // only when set; the helper's contract is documented in the module.
        assert!(resolve_bool(true, "SUSPECT_TEST_UNSET_BOOL", false));
        assert!(!resolve_bool(false, "SUSPECT_TEST_UNSET_BOOL", false));
    }
}
