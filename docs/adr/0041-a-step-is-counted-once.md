# A workflow step is counted once, and failed if any run of it failed

`passed + failed + skipped` of a run is the number of steps. A step a `goto` ran again is still one step: it is failed if any run of it failed, passed if it ran and none did, skipped if it never ran. A retry replaces only the attempt it retries, so a failure from an earlier run of the same step stays. The run summary, the console report, JUnit, the JSON report and the editor all count this way.

## Considered Options

- Count every run of a step, as the runner first did: rejected. Two steps in a `goto` loop reported `1 passed, 17 failed`, and the totals disagreed with the cases of the same report.
- Let the last run decide: rejected. A workflow that fails, jumps back and then passes exits `1` (a step did fail), and a report showing no failed step would not say why.

## Consequences

A suite whose totals were read by a script sees smaller numbers for workflows that loop. A step that failed once and then passed is reported failed, with the checks of the run that failed.
