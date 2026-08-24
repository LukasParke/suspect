import * as fs from 'fs';
import * as vscode from 'vscode';
import { parseArazzo } from './parse';
import { errorMessage, spawnSuspectRun, SuspectEvent } from './runner';

type NodeData =
	| { kind: 'file'; file: vscode.Uri }
	| { kind: 'workflow'; file: vscode.Uri; workflowId: string }
	| { kind: 'step'; file: vscode.Uri; workflowId: string; stepId: string };

interface StepEntry {
	item: vscode.TestItem;
	ended: boolean;
}

interface WorkflowJob {
	file: vscode.Uri;
	workflowId: string;
	wfItem?: vscode.TestItem;
	steps: Map<string, StepEntry>;
}

const nodeDataByItem = new WeakMap<vscode.TestItem, NodeData>();

function nodeData(item: vscode.TestItem): NodeData | undefined {
	return nodeDataByItem.get(item);
}

export function registerTestExplorer(context: vscode.ExtensionContext): void {
	const controller = vscode.tests.createTestController('suspect.tests', 'Suspect Workflows');

	controller.resolveHandler = async (item) => {
		if (!item) {
			await refresh(controller);
		}
	};

	controller.createRunProfile('Suspect Run', vscode.TestRunProfileKind.Run, runHandler.bind(undefined, controller), true);

	context.subscriptions.push(
		controller,
		vscode.workspace.onDidChangeWorkspaceFolders(() => void refresh(controller)),
		vscode.workspace.onDidSaveTextDocument((doc) => {
			if (/\.arazzo\.ya?ml$/i.test(doc.uri.fsPath)) {
				void refresh(controller);
			}
		}),
		vscode.commands.registerCommand('suspect.tests.refresh', () => void refresh(controller)),
	);

	async function refresh(controller: vscode.TestController): Promise<void> {
		const items: vscode.TestItem[] = [];
		const uris = await vscode.workspace.findFiles('**/*.arazzo.{yaml,yml}', '**/node_modules/**');
		for (const uri of uris.sort((a, b) => a.fsPath.localeCompare(b.fsPath))) {
			let parsed;
			try {
				parsed = parseArazzo(await fs.promises.readFile(uri.fsPath, 'utf8'));
			} catch {
				continue; // unreadable/deleted between findFiles and read
			}
			const fileItem = controller.createTestItem(uri.toString(), vscode.workspace.asRelativePath(uri), uri);
			nodeDataByItem.set(fileItem, { kind: 'file', file: uri });
			fileItem.canResolveChildren = false;
			for (const wf of parsed) {
				const wfItem = controller.createTestItem(
					`${uri.toString()}\u0000${wf.workflowId}`,
					wf.workflowId,
					uri,
				);
				nodeDataByItem.set(wfItem, { kind: 'workflow', file: uri, workflowId: wf.workflowId });
				wfItem.range = new vscode.Range(wf.line, 0, wf.line, 0);
				for (const step of wf.steps) {
					const stepItem = controller.createTestItem(
						`${uri.toString()}\u0000${wf.workflowId}\u0000${step.stepId}`,
						step.stepId,
						uri,
					);
					nodeDataByItem.set(stepItem, {
						kind: 'step',
						file: uri,
						workflowId: wf.workflowId,
						stepId: step.stepId,
					});
					stepItem.range = new vscode.Range(step.line, 0, step.line, 0);
				}
				fileItem.children.add(wfItem);
			}
			items.push(fileItem);
		}
		controller.items.replace(items);
	}

	async function runHandler(controller: vscode.TestController, request: vscode.TestRunRequest, token: vscode.CancellationToken): Promise<void> {
		const run = controller.createTestRun(request);

		const jobs = new Map<string, WorkflowJob>();
		const visit = (item: vscode.TestItem, wfItem?: vscode.TestItem, wf?: Extract<NodeData, { kind: 'workflow' }>) => {
			const data = nodeData(item);
			if (!data) {
				return;
			}
			switch (data.kind) {
				case 'file':
					item.children.forEach((child) => visit(child));
					return;
				case 'workflow':
					item.children.forEach((child) => visit(child, item, data));
					return;
				case 'step': {
					const key = `${data.file.fsPath}\u0000${data.workflowId}`;
					let job = jobs.get(key);
					if (!job) {
						job = { file: data.file, workflowId: data.workflowId, wfItem, steps: new Map() };
						jobs.set(key, job);
					}
					job.steps.set(data.stepId, { item, ended: false });
					run.enqueued(item);
					return;
				}
			}
		};
		if (request.include && request.include.length > 0) {
			request.include.forEach((item) => visit(item));
		} else {
			controller.items.forEach((item) => visit(item));
		}

		try {
			for (const job of jobs.values()) {
				if (token.isCancellationRequested) {
					break;
				}
				await executeWorkflow(run, job, token);
			}
		} finally {
			run.end();
		}
	}

	function executeWorkflow(run: vscode.TestRun, job: WorkflowJob, token: vscode.CancellationToken): Promise<void> {
		if (job.wfItem) {
			run.started(job.wfItem);
		}
		const finishStep = (stepId: string, outcome: 'passed' | 'failed' | 'errored', message?: vscode.TestMessage) => {
			const entry = job.steps.get(stepId);
			if (!entry || entry.ended) {
				return;
			}
			switch (outcome) {
				case 'passed':
					run.passed(entry.item);
					return;
				case 'failed':
					run.failed(entry.item, message ?? new vscode.TestMessage(`step ${stepId} failed`));
					return;
				case 'errored':
					run.errored(entry.item, message ?? new vscode.TestMessage(`step ${stepId} could not run`));
					return;
			}
		};
		const onEvent = (ev: SuspectEvent) => {
			switch (ev.event) {
				case 'step_started': {
					const entry = job.steps.get(ev.step);
					if (entry && !entry.ended) {
						run.started(entry.item);
					}
					return;
				}
				case 'criterion_fail':
					finishStep(ev.step, 'failed', new vscode.TestMessage(`${ev.crit} — expected: ${ev.expected}, actual: ${ev.actual}`));
					return;
				case 'wf_done':
					for (const stepId of job.steps.keys()) {
						finishStep(stepId, ev.passed ? 'passed' : 'failed');
					}
					if (job.wfItem) {
						if (ev.passed) {
							run.passed(job.wfItem);
						} else {
							run.failed(job.wfItem, new vscode.TestMessage(`workflow ${job.workflowId} finished with failures`));
						}
					}
					return;
			}
		};
		const handle = spawnSuspectRun(job.file.fsPath, job.workflowId, onEvent);
		const cancelSub = token.onCancellationRequested(() => handle.kill());
		return handle.done
			.catch(async (err: unknown) => {
				const msg = new vscode.TestMessage(errorMessage(err));
				if (job.wfItem) {
					run.errored(job.wfItem, msg);
				}
				for (const [stepId, entry] of job.steps) {
					if (!entry.ended) {
						run.errored(entry.item, msg);
						entry.ended = true;
					}
				}
				await vscode.window.showErrorMessage(`Suspect could not run ${job.workflowId}: ${errorMessage(err)}`);
			})
			.finally(() => cancelSub.dispose())
			.then(() => undefined);
	}
}

