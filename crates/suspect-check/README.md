# suspect-check

The one check pass for the suspect toolchain. Today it holds the rule
registry: every code the validate, Swagger, lint, Arazzo, Overlay,
load-profile and SDK admission checks emit, with its owner, default
severity, class and guidance. The CLI and the language server will both run
their checks through this crate (docs/proposals/P1-finding-model.md).

Part of [suspect](https://github.com/LukasParke/suspect), an OpenAPI,
Arazzo and Overlay toolchain. Most users want the `suspect` binary rather
than this crate; see the repository README for install instructions.

Licensed under either of MIT or Apache-2.0 at your option
([LICENSE-MIT](https://github.com/LukasParke/suspect/blob/main/LICENSE-MIT),
[LICENSE-APACHE](https://github.com/LukasParke/suspect/blob/main/LICENSE-APACHE)).
