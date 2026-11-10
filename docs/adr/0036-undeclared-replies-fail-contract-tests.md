# A reply the description does not declare fails the contract test

Contract checks are strict by default: a status the operation does not declare (no exact code, no `4XX` range, no `default`), a content type the response does not declare, a missing required response header and a JSON body that fails the schema of its media type each fail the step, whether or not the step's own criteria pass. `--conformance schema` (or `test.conformance`) keeps only the body schema of declared responses, and `off` keeps only the criteria. The level is one setting shared by `test run`, `test stateful`, `test fuzz` and the editor.

## Considered Options

- Check only what is declared and pass everything else, as the runner first did: rejected. An undeclared `202`, an HTML error page behind a `200` and a body under `application/problem+json` all passed, and the report still said the response matched the contract.
- Strict with no way out: rejected. A description that documents only the success path would fail on its first error-path workflow, and the only alternative would be no checking at all.

## Consequences

A suite that passed before can fail after upgrading, on descriptions that leave statuses or media types out; the failure names what was undeclared. `✓ response matches contract` is reported only when a body was actually compared with a schema.
