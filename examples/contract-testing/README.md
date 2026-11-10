# Contract-testing example

A description, a suite and nothing else: enough to run every command in the
[contract-testing guide](../../docs/guide/contract-testing.md).

| File | What it is |
|---|---|
| [`petstore.openapi.yaml`](petstore.openapi.yaml) | The contract: an OpenAPI 3.1 description with an inline response schema, a shared error response, a `default` response and a nullable property |
| [`flows.arazzo.yaml`](flows.arazzo.yaml) | The suite: two Arazzo workflows (`browse`, `lifecycle`) with compound conditions, a JSONPath criterion, step outputs and a retry |

Run it from the repository root. The mock gateway stands in for the
service; `--enforce` makes it reject requests the description does not
allow, as a real service would:

```console
$ suspect traffic gateway examples/contract-testing/petstore.openapi.yaml -p 8080 --mode mock --enforce &
$ suspect test run examples/contract-testing/flows.arazzo.yaml --base-url http://127.0.0.1:8080
...
6 passed, 0 failed, 0 skipped in 3 ms
```

Record that run, then repeat it offline from the recording:

```console
$ suspect traffic gateway examples/contract-testing/petstore.openapi.yaml -p 9000 \
    --mode record --upstream http://127.0.0.1:8080 --cassette rec.scj &
$ suspect test run examples/contract-testing/flows.arazzo.yaml --base-url http://127.0.0.1:9000
$ suspect test run examples/contract-testing/flows.arazzo.yaml --cassette rec.scj
```

Generated sequences and mutated requests against the same description:

```console
$ suspect test stateful examples/contract-testing/petstore.openapi.yaml --base-url http://127.0.0.1:8080
$ suspect test fuzz examples/contract-testing/petstore.openapi.yaml --base-url http://127.0.0.1:8080
```

To see a failure, change `id` in the `Pet` schema to `type: integer` and run
the suite again without restarting the gateway. The gateway still answers
with the string ids of the description it loaded, so every `Pet` now breaks
the edited contract:

```text
FAIL response-schema: expected a body matching the declared schema, got /id: value has type string, expected `integer`
```

`cargo xtask ci platform-smoke` runs the first two blocks on every push, and
the workspace tests run all of them, so this example cannot go stale.
