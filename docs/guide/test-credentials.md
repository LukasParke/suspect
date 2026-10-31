# Test credentials

`suspect test` (and every stage that runs workflows: `suspect ci --stage test`,
`suspect project build`) injects credentials into every request whose
operation declares a matching security requirement. Explicit step
parameters win; injection never overwrites a value the workflow set.

## The credentials file

`.suspect/credentials.json`, discovered by walking up from the Arazzo
document — the same convention as `.suspect.yaml`. Three ways to point at
a different file, in precedence order:

1. `--credentials FILE` on `suspect test` / `suspect auth check`
2. the `SUSPECT_CREDENTIALS` environment variable
3. `tests.credentials` in `suspect.project.json` (the project and CI paths)

Keep the file out of version control (`.suspect/` is already the ignored
build directory) and mode `0600` — a loose file draws a warning telling you
to `chmod 600`. Secrets are never echoed: not in `auth check` output, not
in run events, and never in recorded cassettes, which redact
`authorization`, `cookie`, `set-cookie`, `proxy-authorization` and
`x-api-key` headers (plus `password`/`token`/`secret`/`api_key` body keys)
before anything reaches a sink.

## The format

An `auth.schemes` map from OpenAPI security scheme name to a credential
strategy. Every string field accepts `${VAR}` references, resolved from the
environment at load time — the committed file names variables, the
environment holds values, so a file with references can be committed and a
CI job just exports the vars. A missing variable fails the run with an
error that names the variable, never any value.

Field names are snake_case; the scheme names must match the
`components.securitySchemes` keys of the API under test.

### Static bearer

```json
{
  "auth": {
    "schemes": {
      "tokenAuth": {"kind": "bearer", "token": "${MY_TOKEN}"}
    }
  }
}
```

`header` (optional) overrides `Authorization`.

### API key

```json
{
  "auth": {
    "schemes": {
      "apiKeyAuth": {"kind": "apiKey", "name": "X-Api-Key", "value": "${MY_KEY}"}
    }
  }
}
```

### OAuth 2.0 client credentials

Machine-to-machine: the runner acquires an access token from the token
endpoint, caches it until 30s before `expires_in`, and re-uses it across
every workflow in the run. The grant goes through the same HTTP transport
as the steps, so cassettes replay it deterministically.

```json
{
  "auth": {
    "schemes": {
      "oauth2": {
        "kind": "clientCredentials",
        "token_url": "https://sso.example.com/oauth2/token",
        "client_id": "${CLIENT_ID}",
        "client_secret": "${CLIENT_SECRET}",
        "scope": "api:read"
      }
    }
  }
}
```

### OAuth 2.0 refresh token

User-context tokens: acquire the refresh token out of band (a device-code
flow, a manual login) and let the runner exchange it for access tokens,
refreshing on the same schedule. The refresh token is never placed on a
request — it goes to the token endpoint only; treating a long-lived
credential as an API key would put it on every call.

```json
{
  "auth": {
    "schemes": {
      "oauth2": {
        "kind": "refreshToken",
        "token_url": "https://sso.example.com/oauth2/token",
        "client_id": "${CLIENT_ID}",
        "refresh_token": "${REFRESH_TOKEN}"
      }
    }
  }
}
```

Both grants percent-encode the form body field by field: a client secret
containing `&`, `=` or `%` is a value, not structure.

## Verifying without a workflow

`suspect auth check` resolves every configured scheme the way a run would —
OAuth schemes acquire a real token from their endpoint — and reports each
scheme's wire placement, exiting non-zero on any failure. It is the one
command to run when credentials stop working; its output contains no secret
values.

## What the editor inherits

The VS Code Testing view invokes the same `suspect test` binary, so suites
run from the editor discover `.suspect/credentials.json` exactly like a
terminal run from the project root.
