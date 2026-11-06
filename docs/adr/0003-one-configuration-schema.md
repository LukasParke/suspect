# One configuration schema with a fixed precedence; the project manifest is not configuration

The CLI and the language server read settings through one crate, `suspect-config`, with one precedence rule: flag > environment > config file > built-in default. Before this, three loaders drifted apart, and twice a config value silently overrode an explicit flag. The project manifest deliberately stays separate: it says what to build (entry document, overlays, targets, tests), not how the tools behave, so editing a build file can never change editor diagnostics.
