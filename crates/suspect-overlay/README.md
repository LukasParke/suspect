# suspect-overlay

OpenAPI Overlay 1.0/1.1 models, apply engine and overlay synthesis.

Actions apply in order to an owned value tree materialized from the target
document, and the result is re-emitted canonically as JSON or YAML. Action
targets are RFC 9535 JSONPath queries evaluated through `suspect-jsonpath`.

Part of [suspect](https://github.com/LukasParke/suspect), an OpenAPI,
Arazzo and Overlay toolchain. Most users want the `suspect` binary rather
than this crate; see the repository README for install instructions.

Licensed under either of MIT or Apache-2.0 at your option
([LICENSE-MIT](https://github.com/LukasParke/suspect/blob/main/LICENSE-MIT),
[LICENSE-APACHE](https://github.com/LukasParke/suspect/blob/main/LICENSE-APACHE)).
