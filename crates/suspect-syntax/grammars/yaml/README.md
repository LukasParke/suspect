# Vendored YAML grammar

Source: [tree-sitter-grammars/tree-sitter-yaml v0.7.2](https://github.com/tree-sitter-grammars/tree-sitter-yaml/tree/v0.7.2),
under the included MIT [LICENSE](LICENSE) (copied unchanged from that tag).
`src/parser.c` is byte-identical to the upstream file:

```text
8a3baaab33fb63cf9a89f97ec61dbb3ab0d4ef69be9f0f229092c79d129617c9
```

`src/scanner.c` carries one local patch: the scanner's row and column
fields (marked `SUSPECT-PATCH`) and their serialize/deserialize code are
widened from `int16_t` to `int32_t`, because upstream corrupts parses beyond
32,767 lines
(`crates/suspect-low/tests/large_yaml.rs`).
