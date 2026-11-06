# Validation programs are versioned, additive and frozen

There are three program envelopes, each a separate version/profile pair:

- v1 `oas31-jsonschema202012-static-subset`, produced by `compile`
- v2 `static-applicators` (`unevaluated*`, `if`, `contains`, …), produced by `compile_v2`
- v3 `resources-dynamic` (`$id` resources, `$dynamicRef`), produced by `compile_v3`

A newer compiler emits a newer envelope only when the selected closure needs one of its instructions (`requires_v2()`/`requires_v3()`). Otherwise it emits the older envelope byte for byte. Existing operands and fields never change. Each native runtime adopts new instructions on its own schedule, so it checks the declared pair and refuses anything newer instead of misexecuting it, and `OwnedProgram::check()` rejects instructions newer than their envelope. Adding schema features therefore never changes the bytes or behavior of SDKs whose schemas do not use them.
