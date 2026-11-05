# suspect-syntax

Lossless tree-sitter concrete syntax trees for JSON and YAML, built from
vendored grammars (`grammars/json`, `grammars/yaml`, each with its upstream
licence). The YAML scanner carries a local patch that widens its row and
column tracking to 32 bits, so documents beyond 32,767 lines parse correctly.

Part of [suspect](https://github.com/LukasParke/suspect), an OpenAPI,
Arazzo and Overlay toolchain. Most users want the `suspect` binary rather
than this crate; see the repository README for install instructions.

Licensed under either of MIT or Apache-2.0 at your option
([LICENSE-MIT](https://github.com/LukasParke/suspect/blob/main/LICENSE-MIT),
[LICENSE-APACHE](https://github.com/LukasParke/suspect/blob/main/LICENSE-APACHE)).
