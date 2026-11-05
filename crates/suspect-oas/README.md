# suspect-oas

Typed OpenAPI 2.0/3.0/3.1/3.2 models as lazy views (`OpenApi`,
`Operation`, `SchemaView`, ...).

Views borrow a `Session` that pins every loaded document in an append-only
arena, so node references stay valid for the session borrow without `Rc` or
`RefCell`. `$ref`s resolve transparently through a `suspect-ref`
`Workspace`; cycles surface as `CycleGuard` markers rather than errors or
infinite loops.

Part of [suspect](https://github.com/LukasParke/suspect), an OpenAPI,
Arazzo and Overlay toolchain. Most users want the `suspect` binary rather
than this crate; see the repository README for install instructions.

Licensed under either of MIT or Apache-2.0 at your option
([LICENSE-MIT](https://github.com/LukasParke/suspect/blob/main/LICENSE-MIT),
[LICENSE-APACHE](https://github.com/LukasParke/suspect/blob/main/LICENSE-APACHE)).
