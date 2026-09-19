# Resource and dynamic validation

This selection uses `suspect.validation.experimental.v3` with the checked
`oas31-jsonschema202012-resources-dynamic` profile. Its original physical schema
sources and separate logical resource names are in `doc/validation-program.json`.

Each node enters its indexed resource, including validation starting at a nested
schema. Entering a resource does not evaluate its root. A dynamic plain-name
reference selects the outermost actually entered matching binding. Unentered
candidate resources remain inert; the fallback resource is entered only after
lookup selects its target. Pointer, empty-fragment and static-anchor fallbacks
are not dynamic overrides. All target indices and bindings come from the checked
compiler; runtime validation does not resolve URIs, fetch schemas or load files.

Every return, failed branch, trial and evaluation failure restores the resource
scope. Cycle identity includes node, instance identity and the exact ordered
resource context. Work is shared: each new distinct entered resource, scanned
resource and inspected binding adds a visit. Dynamic targets start fresh
annotation scopes and propagate only successful evaluated locations. Numeric,
equality, work, depth and nonproductive-recursion failures remain noninvertible.

Native fields affected by dynamic binding use `JsonValue` carriers. A fallback's
string or object shape is not a safe static field type when an outer resource
can replace it. Construct the native enclosing object with exact JSON child
values, then use its complete enclosing codec or SDK operation; each encode
revalidates the current object under that root's resource scope. A standalone
field codec has its own indexed entry scope and can legitimately select a
different binding. Optional fields still distinguish `Absent()` and `Present`;
JSON-null carriers are explicit `JsonNull` values.

Static/native fields retain their source types and exact numeric tokens. Physical
retrieval SourceIds remain the owners of findings, even when references use a
logical `$id`, `$self`, anchor or redirect alias. Source examples use the bounded
shared v3 planner and the same complete native codecs. Earlier static-only
closures stay on their established v1/v2 programs and executors.
