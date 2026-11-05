//! Rule metadata for the builtin packs.

use std::sync::OnceLock;

use suspect_diag::{CheckId, RuleClass, RuleInfo};

use crate::{Category, Linter};

/// Every rule in [`Linter::spectral_default`], at its default severity.
/// The summary is the rule's description; the security pack is
/// [`RuleClass::Design`]. Builtin rules carry no how-to-fix text yet.
///
/// A code defined once per family (`no-$ref-siblings`: error on 3.0,
/// warning on 3.1+) is listed once, from its first definition.
#[must_use]
pub fn builtin_rules() -> &'static [RuleInfo] {
    static LINTER: OnceLock<Linter> = OnceLock::new();
    static RULES: OnceLock<Vec<RuleInfo>> = OnceLock::new();
    RULES.get_or_init(|| {
        let linter: &'static Linter = LINTER.get_or_init(Linter::spectral_default);
        let mut seen = std::collections::HashSet::new();
        linter
            .rules()
            .iter()
            .filter(|rule| seen.insert(&*rule.code))
            .filter_map(|rule| {
                Some(RuleInfo {
                    class: match rule.category {
                        Category::Document => RuleClass::Document,
                        Category::Design => RuleClass::Design,
                    },
                    ..RuleInfo::new(
                        &rule.code,
                        CheckId::Lint,
                        rule.severity.to_diag()?,
                        rule.description.as_deref().unwrap_or(""),
                    )
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_rule_is_listed_once_with_a_summary() {
        let rules = builtin_rules();
        let mut codes: Vec<&str> = rules.iter().map(|r| r.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), rules.len(), "duplicate builtin code");
        for rule in rules {
            assert!(!rule.summary.is_empty(), "{} has no summary", rule.code);
        }
        let design = rules
            .iter()
            .find(|r| r.code == "security-no-delete-without-id");
        assert_eq!(design.map(|r| r.class), Some(RuleClass::Design));
    }
}
