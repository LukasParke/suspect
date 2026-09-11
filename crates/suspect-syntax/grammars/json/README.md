# Vendored JSON grammar

Source: [tree-sitter/tree-sitter-json v0.24.8](https://github.com/tree-sitter/tree-sitter-json/tree/v0.24.8),
under the included MIT [LICENSE](LICENSE). The unmodified upstream `parser.c`
was byte-identical to Suspect's originally vendored parser:

```text
e8e1ff5df0d73e3b82574129724e68ef4fa0faf1b8c43dd3f5c1a84839f830ab
```

`grammar.js` is the maintained source. Two local numeric grammar corrections
follow [RFC 8259 section 6](https://www.rfc-editor.org/rfc/rfc8259#section-6):

- An exponent accepts either `+` or `-`, for example `1e+400` and `-1.25E-400`.
- A decimal point requires at least one following digit; `1.` and `1.e2` are
  rejected.

These changes concern number tokens. The upstream grammar's support for
comments, error recovery, and multiple top-level values is otherwise retained;
this parser alone is not a complete strict JSON document validator.

Generate artifacts using **tree-sitter CLI 0.24.7**, ABI 14, and Node.js. That
CLI regenerates the unmodified upstream parser byte for byte, which was checked
before applying the local corrections. From this directory:

```sh
tree-sitter generate --abi 14
```

Keep the generated `src/parser.c`, `src/grammar.json`, `src/node-types.json`, and
`src/tree_sitter/*.h` together. Do not edit generated tables. The CLI may be
provisioned separately from its [pinned release](https://github.com/tree-sitter/tree-sitter/releases/tag/v0.24.7);
ordinary Cargo builds compile these checked-in artifacts and do not download or
run a grammar generator.

Regression checks, from the workspace root:

```sh
cargo test -p suspect-syntax --test json_numbers --locked
cargo test -p suspect-low --test numbers --locked
```

The first test exercises `SourceDoc::with_format`; the second exercises numeric
classification and bounded conversion through `LowDoc::with_format`. Neither
uses floating-point materialization as an oracle for numeric validity.
