// The contract suite, in the native Testing view.
//
// The workflow explorer runs any *.arazzo.yaml it finds, live, against the
// editor's base URL. Contract testing is what the project declares: the
// manifest's `tests` section names the Arazzo suites, the base URL they
// run against, and — the part a live run can never be — the recorded
// cassette to replay offline. This controller surfaces exactly that, and
// the CI gate stages beside it, so the Testing view answers the question
// CI answers: is this project green?
//
//   Suspect Contract Tests
//   └ <project>                       (suspect.project.json)
//     ├ validate / lint / breaking    (suspect ci --stage …, per-stage)
//     └ contract tests                (tests.arazzo, cassette honored)
//       └ <suite> └ <workflow> └ <steps>   (live ndjson step results)

import * as cp from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { errorMessage, SuspectEvent, spawnSuspectRunWith } from './runner';
import { parseArazzo } from './parse';

/** The `tests` section of a suspect project manifest. */
export interface ManifestTests {
	arazzo: string[];
	base_url: string;
	cassette?: string;
}

/** Reads one project manifest's contract-test declaration. Pure: testable. */
export function readManifestTests(text: string): ManifestTests | undefined {
	let parsed: unknown;
	try {
		parsed = JSON.parse(text);
	} catch {
		return undefined;
	}
	if (typeof parsed !== 'object' || parsed === null) {
		return undefined;
	}
	const tests = (parsed as { tests?: unknown }).tests;
	if (typeof tests !== 'object' || tests === null) {
		return undefined;
	}
	const raw = tests as { arazzo?: unknown; base_url?: unknown; cassette?: unknown };
	if (!Array.isArray(raw.arazzo)) {
		return undefined;
	}
	const arazzo = raw.arazzo.filter((f): f is string => typeof f === 'string');
	const base_url = typeof raw.base_url === 'string' ? raw.base_url : 'http://localhost:8080';
	const cassette = typeof raw.cassette === 'string' ? raw.cassette : undefined;
	return { arazzo, base_url, cassette };
}

/** One stage result from `suspect ci --format json`. Pure: testable. */
export interface StageResult {
	stage: string;
	passed: boolean;
	errors: number;
	warnings: number;
	summary: string;
}

/** Extracts the first project's stages from a `ci --format json` document. Pure. */
export function parseCiStages(text: string): StageResult[] {
	let parsed: unknown;
	try {
		parsed = JSON.parse(text);
	} catch {
		return [];
	}
	if (typeof parsed !== 'object' || parsed === null) {
		return [];
	}
	const projects = (parsed as { projects?: unknown }).projects;
	if (!Array.isArray(projects) || projects.length === 0) {
		return [];
	}
	const stages = (projects[0] as { stages?: unknown }).stages;
	if (!Array.isArray(stages)) {
		return [];
	}
	return stages
		.filter((s): s is Record<string, unknown> => typeof s === 'object' && s !== null)
		.map((s) => ({
			stage: String(s.stage ?? ''),
			passed: s.passed === true,
			errors: Number(s.errors ?? 0),
			warnings: Number(s.warnings ?? 0),
			summary: String(s.summary ?? ''),
		}));
}

type Node =
	| { kind: 'project'; dir: string; manifest: string }
	| { kind: 'stage'; dir: string; stage: string }
	| { kind: 'suite'; dir: string; file: string; baseUrl: string; cassette?: string }
	| { kind: 'workflow'; dir: string; file: string; baseUrl: string; cassette?: string; workflowId: string }
	| { kind: 'step'; dir: string; file: string; baseUrl: string; cassette?: string; workflowId: string; stepId: string };

const nodeDataByItem = new WeakMap<vscode.TestItem, Node>();
const nodeData = (item: vscode.TestItem): Node | undefined => nodeDataByItem.get(item);

const STAGES: { id: string; label: string }[] = [
	{ id: 'validate', label: 'Validate' },
	{ id: 'lint', label: 'Lint' },
	{ id: 'breaking', label: 'Breaking' },
];

/** Runs one CI stage and returns its result. */
function runStage(binary: string, dir: string, stage: string): Promise<StageResult> {
	return new Promise((resolve, reject) => {
		let out = '';
		const child = cp.spawn(binary, ['ci', '--stage', stage, '--format', 'json', '.'], { cwd: dir });
		child.stdout?.on('data', (c: Buffer) => { out += c.toString(); });
		child.stderr?.on('data', () => { /* the JSON is the result */ });
		child.on('error', reject);
		child.on('exit', () => {
			const stages = parseCiStages(out);
			const mine = stages.find((s) => s.stage === stage);
			resolve(mine ?? { stage, passed: false, errors: 0, warnings: 0, summary: 'no result' });
		});
	});
}

export function registerContractSuite(context: vscode.ExtensionContext, binaryOf: () => string): void {
	const controller = vscode.tests.createTestController('suspect.contract', 'Suspect Contract Tests');

	controller.resolveHandler = async (item) => {
		if (!item) {
			await refresh();
		}
	};

	controller.createRunProfile('Run', vscode.TestRunProfileKind.Run, runHandler, true);

	context.subscriptions.push(
		controller,
		vscode.commands.registerCommand('suspect.contract.refresh', () => void refresh()),
		vscode.workspace.onDidSaveTextDocument((doc) => {
			if (doc.uri.fsPath.endsWith('suspect.project.json') || /\.arazzo\.ya?ml$/i.test(doc.uri.fsPath)) {
				void refresh();
			}
		}),
	);

	async function refresh(): Promise<void> {
		const items: vscode.TestItem[] = [];
		const manifests = await vscode.workspace.findFiles('**/suspect.project.json', '**/node_modules/**');
		for (const manifest of manifests.sort((a, b) => a.fsPath.localeCompare(b.fsPath))) {
			const dir = path.dirname(manifest.fsPath);
			const projectItem = controller.createTestItem(manifest.toString(), path.basename(dir), manifest);
			nodeDataByItem.set(projectItem, { kind: 'project', dir, manifest: manifest.fsPath });
			projectItem.canResolveChildren = true;

			for (const stage of STAGES) {
				const stageItem = controller.createTestItem(`${manifest.toString()}#stage/${stage.id}`, stage.label, manifest);
				nodeDataByItem.set(stageItem, { kind: 'stage', dir, stage: stage.id });
				projectItem.children.add(stageItem);
			}

			let tests: ManifestTests | undefined;
			try {
				tests = readManifestTests(await fs.promises.readFile(manifest.fsPath, 'utf8'));
			} catch {
				tests = undefined;
			}
			if (tests !== undefined) {
				for (const rel of tests.arazzo) {
					const suiteUri = vscode.Uri.file(path.join(dir, rel));
					let parsed;
					try {
						parsed = parseArazzo(await fs.promises.readFile(suiteUri.fsPath, 'utf8'));
					} catch {
						const broken = controller.createTestItem(`${manifest}#suite/${rel}`, rel, suiteUri);
						broken.description = 'unreadable';
						projectItem.children.add(broken);
						continue;
					}
					const suiteItem = controller.createTestItem(`${manifest}#suite/${rel}`, rel, suiteUri);
					nodeDataByItem.set(suiteItem, { kind: 'suite', dir, file: suiteUri.fsPath, baseUrl: tests.base_url, cassette: tests.cassette });
					suiteItem.description = tests.cassette !== undefined ? 'offline (cassette)' : tests.base_url;
					for (const wf of parsed) {
						const wfItem = controller.createTestItem(`${manifest}#suite/${rel}#${wf.workflowId}`, wf.workflowId, suiteUri);
						nodeDataByItem.set(wfItem, { kind: 'workflow', dir, file: suiteUri.fsPath, baseUrl: tests.base_url, cassette: tests.cassette, workflowId: wf.workflowId });
						wfItem.range = new vscode.Range(wf.line, 0, wf.line, 0);
						for (const step of wf.steps) {
							const stepItem = controller.createTestItem(
								`${manifest}#suite/${rel}#${wf.workflowId}\u0000${step.stepId}`,
								step.stepId,
								suiteUri,
							);
							nodeDataByItem.set(stepItem, {
								kind: 'step',
								dir,
								file: suiteUri.fsPath,
								baseUrl: tests.base_url,
								cassette: tests.cassette,
								workflowId: wf.workflowId,
								stepId: step.stepId,
							});
							stepItem.range = new vscode.Range(step.line, 0, step.line, 0);
							wfItem.children.add(stepItem);
						}
						suiteItem.children.add(wfItem);
					}
					projectItem.children.add(suiteItem);
				}
			}
			items.push(projectItem);
		}
		controller.items.replace(items);
	}

	async function runHandler(request: vscode.TestRunRequest, token: vscode.CancellationToken): Promise<void> {
		const run = controller.createTestRun(request);
		const binary = binaryOf();

		const stageItems: vscode.TestItem[] = [];
		const suites = new Map<vscode.TestItem, Node & { kind: 'suite' }>();
		const collect = (item: vscode.TestItem) => {
			const data = nodeData(item);
			if (!data) return;
			switch (data.kind) {
				case 'project':
					item.children.forEach(collect);
					return;
				case 'stage':
					stageItems.push(item);
					return;
				case 'suite':
					suites.set(item, data);
					return;
				case 'workflow':
					suites.set(item.parent!, { ...data, kind: 'suite' });
					return;
				case 'step':
					suites.set(item.parent!.parent!, { ...(nodeData(item.parent!) as Node & { kind: 'workflow' }), kind: 'suite' });
					return;
			}
		};
		if (request.include && request.include.length > 0) {
			request.include.forEach(collect);
		} else {
			controller.items.forEach(collect);
		}

		for (const item of stageItems) {
			const data = nodeData(item);
			if (!data || data.kind !== 'stage' || token.isCancellationRequested) continue;
			run.started(item);
			const result = await runStage(binary, data.dir, data.stage);
			const message = `${result.errors} error(s), ${result.warnings} warning(s) — ${result.summary}`;
			if (result.passed) {
				// Duration keeps the passing stage visible rather than
				// vanishing; the message carries the warning count.
				run.passed(item, 1);
			} else {
				run.failed(item, new vscode.TestMessage(message));
			}
		}

		for (const [suiteItem, data] of suites) {
			if (token.isCancellationRequested) break;
			await executeSuite(run, suiteItem, data, token, binary);
		}
		run.end();
	}

	async function executeSuite(
		run: vscode.TestRun,
		suiteItem: vscode.TestItem,
		data: Node & { kind: 'suite' },
		token: vscode.CancellationToken,
		binary: string,
	): Promise<void> {
		const filter = data.kind === 'suite' ? undefined : (data as { workflowId?: string }).workflowId;
		const wfItems: vscode.TestItem[] = [];
		const stepItems = new Map<string, vscode.TestItem>();
		suiteItem.children.forEach((wf) => {
			if (filter !== undefined && wf.label !== filter) return;
			wfItems.push(wf);
			wf.children.forEach((step) => stepItems.set(`${wf.label}\u0000${step.label}`, step));
		});
		for (const item of wfItems) run.enqueued(item);

		const onEvent = (ev: SuspectEvent) => {
			switch (ev.event) {
				case 'step_started': {
					const step = stepItems.get(`${ev.wf}\u0000${ev.step}`);
					if (step) run.started(step);
					return;
				}
				case 'criterion_fail': {
					const step = stepItems.get(`${ev.wf}\u0000${ev.step}`);
					if (step) {
						run.failed(step, new vscode.TestMessage(`${ev.crit} — expected: ${ev.expected}, actual: ${ev.actual}`));
					}
					return;
				}
				case 'wf_done': {
					const wf = wfItems.find((w) => w.label === ev.wf);
					if (wf) {
						if (ev.passed) {
							run.passed(wf);
						} else {
							run.failed(wf, new vscode.TestMessage(`workflow ${ev.wf} finished with failures`));
						}
					}
					// Steps that never reported stay in whatever state the
					// criteria left them; the workflow item carries the verdict.
					return;
				}
			}
		};

		const handle = spawnSuspectRunWith(
			data.file,
			{ baseUrl: data.baseUrl, cassette: data.cassette, filter: (data as { workflowId?: string }).workflowId },
			onEvent,
			binary,
		);
		const cancelSub = token.onCancellationRequested(() => handle.kill());
		try {
			await handle.done;
			if (filter === undefined) run.passed(suiteItem);
		} catch (err) {
			run.errored(suiteItem, new vscode.TestMessage(errorMessage(err)));
		} finally {
			cancelSub.dispose();
		}
		void filter;
	}
}
