# Own the YAML stack: vendored, patched tree-sitter grammars and no serde_yaml

Every YAML file suspect reads (specs, Arazzo, overlays, load profiles, config) goes through one stack: the tree-sitter grammars vendored in `crates/suspect-syntax/grammars/`, lowered into the `suspect-low` CST, plus the fast-subset reader (ADR-0002). No serde_yaml or other YAML crate is a dependency. This gives every file the same YAML 1.2 typing, byte-exact spans for diagnostics, and LSP support without extra work.

## Consequences

The grammars are vendored because upstream tree-sitter-yaml v0.7.2 corrupts parses beyond 32,767 lines. The local patch (`SUSPECT-PATCH`, widening the scanner's row from `int16_t` to `int32_t`) is guarded by `crates/suspect-low/tests/large_yaml.rs`. A grammar upgrade must carry that patch forward.
