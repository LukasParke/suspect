import * as vscode from 'vscode';

/**
 * Minimal dependency-free Arazzo structure parser.
 *
 * Deliberately regex-based (per editors contract): we only need workflow
 * ordering and their step ids for the tree view, Testing API provider, and
 * notebook projection — not a full YAML model.
 */
export interface ParsedStep {
	/** `stepId` value, quotes stripped, comments removed. */
	stepId: string;
	/** Zero-based line number in the source document. */
	line: number;
}

export interface ParsedWorkflow {
	/** `workflowId` value, quotes stripped, comments removed. */
	workflowId: string;
	/** Zero-based line number of the `workflowId:` key. */
	line: number;
	steps: ParsedStep[];
}

// Arazzo nests workflow entries under a sequence dash; accept the key either
// bare at indent <= 2 or directly after a "- " bullet.
const WORKFLOW_RE = /^[ \t]{0,2}(?:-[ \t]+)?workflowId:(.*)$/;
const STEP_RE = /^[ \t]+(?:-[ \t]+)?stepId:(.*)$/;

function cleanValue(raw: string): string {
	let value = raw.trim();
	const comment = value.indexOf(' #');
	if (comment >= 0) {
		value = value.slice(0, comment).trim();
	}
	if (value.length >= 2) {
		const first = value[0];
		if ((first === '"' || first === "'") && value[value.length - 1] === first) {
			return value.slice(1, -1);
		}
	}
	return value;
}

/** Parse every `workflowId:` at indent <= 2 with its nested `stepId:` lines. */
export function parseArazzo(source: string): ParsedWorkflow[] {
	const workflows: ParsedWorkflow[] = [];
	let current: ParsedWorkflow | undefined;
	source.split(/\r?\n/).forEach((rawLine, index) => {
		const wfMatch = WORKFLOW_RE.exec(rawLine);
		if (wfMatch) {
			current = { workflowId: cleanValue(wfMatch[1]), line: index, steps: [] };
			workflows.push(current);
			return;
		}
		if (!current) {
			return;
		}
		const stepMatch = STEP_RE.exec(rawLine);
		if (stepMatch) {
			current.steps.push({ stepId: cleanValue(stepMatch[1]), line: index });
		}
	});
	return workflows;
}
