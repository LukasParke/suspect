// The verdict the Testing view gives each step of a workflow run, from the
// runner's event stream.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { StepVerdicts } = require('../dist/stepVerdicts.js');

test('a step with a failed criterion has failed', () => {
	const verdicts = new StepVerdicts();
	verdicts.started('work');
	verdicts.criterionFailed('work');
	assert.equal(verdicts.hasFailed('work'), true);
	assert.equal(verdicts.hasFailed('other'), false);
});

test('a retry replaces the attempt it retries', () => {
	const verdicts = new StepVerdicts();
	verdicts.started('work');
	verdicts.criterionFailed('work');
	verdicts.retried('work');
	verdicts.started('work');
	assert.equal(verdicts.hasFailed('work'), false);
});

test('a retry does not erase what an earlier run of the step failed', () => {
	const verdicts = new StepVerdicts();
	// First run: 500, and a goto runs the workflow again.
	verdicts.started('work');
	verdicts.criterionFailed('work');
	// Second run: 503, retried, then accepted.
	verdicts.started('work');
	verdicts.criterionFailed('work');
	verdicts.retried('work');
	verdicts.started('work');
	assert.equal(verdicts.hasFailed('work'), true);
});
