
## Configured environment credentials

This package has an explicit version-v1 environment policy. The generated policy
contains variable names only. OpenAPI determines bearer or API-key attachment.

`Client()` and `Client(transport = ..., options = ...)` snapshot the configured
process variables when the client is created. `Client.fromEnv(transport = ...,
options = ..., environment = CredentialEnvironment.system)` is the named factory.
Its optional reader supports application-specific environment access through
`CredentialEnvironment { name -> ... }`; values are read once per mapped variable
for that client, never at module load or on each request.

`Client(credentials = Credentials(...), ...)` uses that entire explicit argument.
An empty credentials object, null member, empty string or missing member is never
filled from the environment. Kotlin rejects a null whole `Credentials` argument
at compile time. Use the explicit constructor to disable environment defaults.

Missing, empty, unavailable or attachment-invalid environment values remain
unavailable. Environment-reader exceptions become unavailable values; coroutine
cancellation is preserved. The existing operation security policy resolves
OR/AND/explicit alternatives and anonymous operations. A missing protected
credential fails before transport with `FailureKind.AUTHENTICATION`, without
credential values in the diagnostic. Anonymous operations remain usable.

Environment defaults do not rewrite servers. Omitted server overrides use the
source server, including its declared HTTPS URL. This policy performs no token
acquisition, refresh or file loading. See `docs/credential-env.json` for the bound
physical source declarations and variable-name metadata.
