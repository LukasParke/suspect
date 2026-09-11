## Configured runtime environment credentials

`{client}.fromEnv()` snapshots the explicitly mapped environment variables once at
client creation and uses source-default servers. It stores only usable values in
the existing immutable HTTP credential options. Missing, empty, oversized,
unavailable or attachment-invalid strings remain missing; anonymous operations
still work. A protected operation with unsatisfied security throws a secret-free
`SdkException` with `kind() == "missing-credential"` before HTTP is sent.

`{client}.fromEnv(HttpClient transport)` retains the supplied transport's caller
ownership, so it can also be used with controlled transports and source HTTPS
URLs. The companion
`{client}.fromEnv(HttpClient transport, Function<String,String> environment)`
snapshots an explicitly supplied runtime environment accessor. It is called once
per distinct mapped variable. A null accessor or an accessor exception means
unavailable environment access; neither falls back to the process environment.
The transport parameter must be non-null.

The existing `new {client}(HttpRuntime.Options)` constructor is fully explicit and
never consults environment credentials. An explicit empty options/credentials
set or missing member remains authoritative; it is not filled from environment.
Existing explicit empty/null credential validation also remains unchanged. Use
`RequestOptions` for per-call server, timeout and security-alternative choices.

Source OR/AND order, explicit alternatives and anonymous requirements retain
their normal behavior. This policy supports only source bearer/API-key strings;
it adds no acquisition, refresh, role inference, server rewrite or per-request
environment lookup. Generated policy/provenance files contain variable names
and source identities only, never environment values.
