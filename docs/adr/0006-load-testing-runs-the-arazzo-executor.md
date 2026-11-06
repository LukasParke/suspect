# Load testing is an execution mode of the Arazzo executor, not an Artillery export

`suspect load` runs the same Arazzo workflows as `suspect test`, through the same executor, as weighted virtual users. It reports contract failures (`successCriteria`) separately from transport errors. The Arazzo document says what a user does. A separate `suspectLoad` profile (`*.load.yaml`) says how many users, how fast, with what data and under which SLOs, and references workflows by id, so a workflow stays usable unchanged as a contract test. We rejected exporting to (or importing from) Artillery: that loses contract-resolved requests, per-response contract checks and Rust throughput.

## Consequences

Arrivals are open-loop with Artillery semantics: at `maxVusers` an arrival is skipped, not queued, so a slow server shows up as more concurrent users rather than less load. `suspect test` ignores think time.
