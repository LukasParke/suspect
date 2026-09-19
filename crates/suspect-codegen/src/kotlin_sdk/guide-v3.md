
## Indexed resources and dynamic validation

The checked V3 program uses
`suspect.validation.experimental.v3` /
`oas31-jsonschema202012-resources-dynamic`. Its resource and anchor table comes
from the source Contract. Physical retrieval-document/pointer identities remain
the source of diagnostics and ownership; logical canonical/base/alias URIs are
retained separately as metadata. The runtime performs no acquisition.

Every schema entry enters its indexed resource, including a selected nested
schema whose resource root is never evaluated. A dynamic reference searches only
actually entered resources, outermost first. Unentered candidates are inert and
the initial fallback is not entered before the lookup. Pointer/empty/static-anchor
references use their indexed fallback, and static `$ref` never becomes dynamic.

Each return and trial restores resource scope. Cycle identity includes node,
instance identity and exact ordered resource context; a new context is not an
old-context cycle. New distinct resource entries and each inspected resource or
binding consume the shared work allowance. Numeric/equality/work failures remain
noninvertible. Selected targets receive fresh annotation scope and contribute
only successful evaluated-location sets.

Dynamic input slots use schema-bound JSON carriers, not the static type of their
fallback target. Encode/decode and SDK operations run the complete resource-aware
codec. A model-only call must choose its source-bound codec: a shared native shape
can have several codecs with different resource/annotation constraints.

Ordinary closures continue using their established V1/V2 programs. Explicit
`plan_sdk_v3` and `validation::plan_validation_v3` select the resource profile.
Legacy recursive-reference dialects, custom vocabularies, unsupported regex and
format assertions, and unimplemented directional views have located refusals.
