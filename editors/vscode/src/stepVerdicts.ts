/**
 * Which steps of a workflow run failed, from the runner's event stream.
 *
 * A retry replaces the attempt it retries, so that attempt's failures do
 * not count. A failure from an earlier run of the step (before a goto ran
 * it again) does: a step failed if any run of it failed, which is how the
 * CLI's reports count it.
 */
export class StepVerdicts<T> {
	/** Steps with a run that ended failed. */
	private readonly failed = new Set<T>();
	/** Steps whose attempt in progress has a failed criterion. */
	private readonly failing = new Set<T>();

	/** A step started an attempt: the one before it, if failed, stands. */
	started(step: T): void {
		if (this.failing.delete(step)) this.failed.add(step);
	}

	/** A criterion of the step's attempt in progress failed. */
	criterionFailed(step: T): void {
		this.failing.add(step);
	}

	/** The step's attempt in progress is being retried. */
	retried(step: T): void {
		this.failing.delete(step);
	}

	hasFailed(step: T): boolean {
		return this.failed.has(step) || this.failing.has(step);
	}
}
