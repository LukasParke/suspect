// The Testing view: one controller, "Suspect".
//
// VS Code groups the Testing view by controller, so two controllers with
// overlapping content ("Suspect Workflows" scanning file names, "Suspect
// Contract Tests" reading the manifest) read as two products. There is
// one question the view should answer — is this project green? — so
// there is one tree:
//
//   <project>                           suspect.project.json
//   ├ Gate                              suspect ci --stage …
//   │  ├ Validate · Lint · Breaking
//   └ Contract tests                    tests.arazzo, manifest base URL / cassette
//      └ <suite> └ <workflow> └ <step>  live ndjson step results
//   Other workflows                     Arazzo documents no manifest declares
//      └ <document> └ <workflow> └ <step>   editor base URL
//
// Arazzo documents are found by content, so a suite named `health.yaml`
// is as visible as `health.arazzo.yaml`.

import * as cp from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { affectsProject, findArazzoDocuments, findProjects } from './discover';
import { parseArazzo } from './parse';
import { testId, undeclaredDocuments } from './project';
import { errorMessage, SuspectEvent, spawnSuspectRunWith, testBaseUrl } from './runner';

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

/** What one `suspect test` invocation covers. */
export interface DocumentJob {
	file: string;
	cwd: string;
	baseUrl: string;
	cassette?: string;
	/** `undefined` runs the whole document; otherwise one `--filter` per id. */
	workflows: string[] | undefined;
}

export interface Selection {
	file: string;
	cwd: string;
	baseUrl: string;
	cassette?: string;
	workflowId?: string;
}

/**
 * Groups a selection into the fewest `suspect test` runs: selecting a
 * document (or the group above it) runs it whole; selecting workflows
 * runs each by id. Pure: testable.
 */
export function groupJobs(selection: Selection[]): DocumentJob[] {
	const jobs = new Map<string, DocumentJob>();
	for (const pick of selection) {
		let job = jobs.get(pick.file);
		if (job === undefined) {
			job = { file: pick.file, cwd: pick.cwd, baseUrl: pick.baseUrl, cassette: pick.cassette, workflows: [] };
			jobs.set(pick.file, job);
		}
		if (pick.workflowId === undefined) {
			job.workflows = undefined;
		} else if (job.workflows !== undefined && !job.workflows.includes(pick.workflowId)) {
			job.workflows.push(pick.workflowId);
		}
	}
	return [...jobs.values()];
}

type Node =
	| { kind: 'project'; dir: string }
	| { kind: 'gate'; dir: string }
	| { kind: 'stage'; dir: string; stage: string }
	| { kind: 'group'; file?: undefined }
	| { kind: 'document'; file: string; cwd: string; baseUrl: string; cassette?: string }
	| { kind: 'workflow'; file: string; cwd: string; baseUrl: string; cassette?: string; workflowId: string }
	| { kind: 'step'; file: string; cwd: string; baseUrl: string; cassette?: string; workflowId: string; stepId: string };

const nodeDataByItem = new WeakMap<vscode.TestItem, Node>();
const nodeData = (item: vscode.TestItem): Node | undefined => nodeDataByItem.get(item);

const STAGES: { id: string; label: string }[] = [
	{ id: 'validate', label: 'Validate' },
	{ id: 'lint', label: 'Lint' },
	{ id: 'breaking', label: 'Breaking' },
];

const UNDECLARED_ID = 'other-workflows';

interface Counts {
	projects: number;
	documents: number;
	workflows: number;
	steps: number;
}

function ciBaseline(): string {
	return vscode.workspace.getConfiguration('suspect').get<string>('ci.baseline', '').trim();
}

/** Runs one CI stage and returns its result. */
function runStage(binary: string, dir: string, stage: string): Promise<StageResult> {
	return new Promise((resolve, reject) => {
		let out = '';
		const args = ['ci', '--stage', stage, '--format', 'json'];
		const baseline = ciBaseline();
		if (baseline) args.push('--baseline', baseline);
		args.push('.');
		const child = cp.spawn(binary, args, { cwd: dir });
		child.stdout?.on('data', (c: Buffer) => { out += c.toString(); });
		child.stderr?.on('data', () => { /* the JSON is the result */ });
		child.on('error', reject);
		child.on('exit', () => {
			const mine = parseCiStages(out).find((s) => s.stage === stage);
			resolve(mine ?? { stage, passed: false, errors: 0, warnings: 0, summary: 'no result from suspect ci' });
		});
	});
}

export function registerTesting(context: vscode.ExtensionContext, binaryOf: () => string): void {
	const controller = vscode.tests.createTestController('suspect', 'Suspect');
	const log = vscode.window.createOutputChannel('Suspect Tests');

	controller.resolveHandler = async (item) => {
		if (!item) await safeRefresh();
	};
	controller.refreshHandler = () => safeRefresh();
	controller.createRunProfile('Run', vscode.TestRunProfileKind.Run, runHandler, true);

	context.subscriptions.push(
		controller,
		log,
		vscode.workspace.onDidChangeWorkspaceFolders(() => void safeRefresh()),
		vscode.workspace.onDidSaveTextDocument((doc) => {
			if (affectsProject(doc)) void safeRefresh();
		}),
	);
	// Discover eagerly: the tree is small, and open Arazzo documents get
	// their gutter run controls without the Testing view being opened first.
	void safeRefresh();

	async function safeRefresh(): Promise<void> {
		try {
			const built = await refresh();
			log.appendLine(`${new Date().toISOString()} discovered ${built.projects} project(s), ${built.documents} Arazzo document(s), ${built.workflows} workflow(s), ${built.steps} step(s)`);
		} catch (err) {
			// A throwing resolve handler empties the Testing view silently;
			// say what happened where the user can find it.
			const message = `Suspect: could not build the test tree: ${errorMessage(err)}`;
			log.appendLine(message);
			console.error(message);
			void vscode.window.showErrorMessage(message, 'Show log').then((pick) => pick && log.show());
		}
	}

	function addDocument(
		parent: vscode.TestItem,
		idPrefix: string,
		label: string,
		uri: vscode.Uri,
		cwd: string,
		baseUrl: string,
		cassette: string | undefined,
		counts: Counts,
	): Promise<void> {
		return fs.promises.readFile(uri.fsPath, 'utf8').then(
			(text) => {
				counts.documents += 1;
				const docItem = controller.createTestItem(testId(idPrefix, uri.fsPath), label, uri);
				nodeDataByItem.set(docItem, { kind: 'document', file: uri.fsPath, cwd, baseUrl, cassette });
				docItem.description = cassette !== undefined ? 'offline · cassette' : baseUrl;
				for (const wf of parseArazzo(text)) {
					counts.workflows += 1;
					const wfItem = controller.createTestItem(testId(idPrefix, uri.fsPath, wf.workflowId), wf.workflowId, uri);
					nodeDataByItem.set(wfItem, { kind: 'workflow', file: uri.fsPath, cwd, baseUrl, cassette, workflowId: wf.workflowId });
					wfItem.range = new vscode.Range(wf.line, 0, wf.line, 0);
					for (const step of wf.steps) {
						counts.steps += 1;
						const stepItem = controller.createTestItem(testId(idPrefix, uri.fsPath, wf.workflowId, step.stepId), step.stepId, uri);
						nodeDataByItem.set(stepItem, {
							kind: 'step', file: uri.fsPath, cwd, baseUrl, cassette, workflowId: wf.workflowId, stepId: step.stepId,
						});
						stepItem.range = new vscode.Range(step.line, 0, step.line, 0);
						wfItem.children.add(stepItem);
					}
					docItem.children.add(wfItem);
				}
				parent.children.add(docItem);
			},
			(err: unknown) => {
				const broken = controller.createTestItem(testId(idPrefix, uri.fsPath), label, uri);
				broken.description = 'unreadable';
				broken.error = errorMessage(err);
				parent.children.add(broken);
			},
		);
	}

	async function refresh(): Promise<Counts> {
		const counts: Counts = { projects: 0, documents: 0, workflows: 0, steps: 0 };
		const items: vscode.TestItem[] = [];
		const declared: string[] = [];
		const projects = await findProjects();
		counts.projects = projects.length;

		for (const project of projects) {
			const { manifestUri, dir, manifest } = project;
			const projectItem = controller.createTestItem(testId('project', manifestUri.fsPath), manifest.name ?? path.basename(dir), manifestUri);
			nodeDataByItem.set(projectItem, { kind: 'project', dir });
			projectItem.description = vscode.workspace.asRelativePath(manifestUri);

			const gate = controller.createTestItem(testId('gate', manifestUri.fsPath), 'Gate', manifestUri);
			nodeDataByItem.set(gate, { kind: 'gate', dir });
			gate.description = 'suspect ci';
			gate.sortText = '0';
			for (const stage of STAGES) {
				const stageItem = controller.createTestItem(testId('stage', manifestUri.fsPath, stage.id), stage.label, manifestUri);
				nodeDataByItem.set(stageItem, { kind: 'stage', dir, stage: stage.id });
				gate.children.add(stageItem);
			}
			projectItem.children.add(gate);

			if (manifest.tests !== undefined) {
				const suites = controller.createTestItem(testId('suites', manifestUri.fsPath), 'Contract tests', manifestUri);
				nodeDataByItem.set(suites, { kind: 'group' });
				suites.description = manifest.tests.cassette !== undefined ? 'offline · cassette' : manifest.tests.base_url;
				suites.sortText = '1';
				const cassette = manifest.tests.cassette !== undefined ? path.resolve(dir, manifest.tests.cassette) : undefined;
				for (const rel of manifest.tests.arazzo) {
					const file = path.resolve(dir, rel);
					declared.push(file);
					await addDocument(suites, testId('suite', manifestUri.fsPath), rel, vscode.Uri.file(file), dir, manifest.tests.base_url, cassette, counts);
				}
				projectItem.children.add(suites);
			}
			items.push(projectItem);
		}

		const found = (await findArazzoDocuments()).map((uri) => uri.fsPath);
		const others = undeclaredDocuments(found, declared);
		if (others.length > 0) {
			const group = controller.createTestItem(UNDECLARED_ID, projects.length > 0 ? 'Other workflows' : 'Workflows');
			nodeDataByItem.set(group, { kind: 'group' });
			group.description = projects.length > 0 ? 'not declared by a manifest' : testBaseUrl();
			group.sortText = 'z';
			for (const file of others) {
				const uri = vscode.Uri.file(file);
				const folder = vscode.workspace.getWorkspaceFolder(uri);
				await addDocument(group, UNDECLARED_ID, vscode.workspace.asRelativePath(uri), uri, folder?.uri.fsPath ?? path.dirname(file), testBaseUrl(), undefined, counts);
			}
			items.push(group);
		}

		controller.items.replace(items);
		return counts;
	}

	async function runHandler(request: vscode.TestRunRequest, token: vscode.CancellationToken): Promise<void> {
		const run = controller.createTestRun(request);
		const binary = binaryOf();

		const stageItems: vscode.TestItem[] = [];
		const selection: Selection[] = [];
		const docItems = new Map<string, vscode.TestItem>();
		const collect = (item: vscode.TestItem) => {
			const data = nodeData(item);
			if (!data) return;
			switch (data.kind) {
				case 'project':
				case 'gate':
				case 'group':
					item.children.forEach(collect);
					return;
				case 'stage':
					stageItems.push(item);
					return;
				case 'document':
					docItems.set(data.file, item);
					selection.push({ file: data.file, cwd: data.cwd, baseUrl: data.baseUrl, cassette: data.cassette });
					return;
				case 'workflow':
					docItems.set(data.file, item.parent!);
					selection.push({ file: data.file, cwd: data.cwd, baseUrl: data.baseUrl, cassette: data.cassette, workflowId: data.workflowId });
					return;
				case 'step':
					docItems.set(data.file, item.parent!.parent!);
					selection.push({ file: data.file, cwd: data.cwd, baseUrl: data.baseUrl, cassette: data.cassette, workflowId: data.workflowId });
					return;
			}
		};
		if (request.include && request.include.length > 0) {
			request.include.forEach(collect);
		} else {
			controller.items.forEach(collect);
		}

		try {
			for (const item of stageItems) {
				const data = nodeData(item);
				if (!data || data.kind !== 'stage' || token.isCancellationRequested) continue;
				run.started(item);
				const started = Date.now();
				const result = await runStage(binary, data.dir, data.stage);
				const duration = Date.now() - started;
				const message = `${result.errors} error(s), ${result.warnings} warning(s) — ${result.summary}`;
				run.appendOutput(`${data.stage}: ${message}\r\n`, undefined, item);
				if (result.passed) {
					run.passed(item, duration);
				} else {
					run.failed(item, new vscode.TestMessage(message), duration);
				}
			}

			for (const job of groupJobs(selection)) {
				if (token.isCancellationRequested) break;
				const docItem = docItems.get(job.file);
				if (docItem === undefined) continue;
				if (job.workflows === undefined) {
					await executeDocument(run, docItem, job, undefined, token, binary);
				} else {
					for (const workflowId of job.workflows) {
						if (token.isCancellationRequested) break;
						await executeDocument(run, docItem, job, workflowId, token, binary);
					}
				}
			}
		} finally {
			run.end();
		}
	}

	async function executeDocument(
		run: vscode.TestRun,
		docItem: vscode.TestItem,
		job: DocumentJob,
		filter: string | undefined,
		token: vscode.CancellationToken,
		binary: string,
	): Promise<void> {
		const wfItems = new Map<string, vscode.TestItem>();
		const stepItems = new Map<string, vscode.TestItem>();
		docItem.children.forEach((wf) => {
			if (filter !== undefined && wf.label !== filter) return;
			wfItems.set(wf.label, wf);
			wf.children.forEach((step) => stepItems.set(`${wf.label}/${step.label}`, step));
		});
		for (const item of wfItems.values()) run.enqueued(item);
		const started = new Set<vscode.TestItem>();
		const failed = new Set<vscode.TestItem>();

		const onEvent = (ev: SuspectEvent) => {
			switch (ev.event) {
				case 'step_started': {
					const step = stepItems.get(`${ev.wf}/${ev.step}`);
					if (step) {
						started.add(step);
						run.started(step);
					}
					return;
				}
				case 'request_sent':
					run.appendOutput(`${ev.wf}/${ev.step}: ${ev.method} ${ev.url}\r\n`, undefined, stepItems.get(`${ev.wf}/${ev.step}`));
					return;
				case 'response_got': {
					const step = stepItems.get(`${ev.wf}/${ev.step}`);
					run.appendOutput(`${ev.wf}/${ev.step}: ${ev.status} in ${ev.duration_ms} ms\r\n`, undefined, step);
					return;
				}
				case 'criterion_fail': {
					const step = stepItems.get(`${ev.wf}/${ev.step}`);
					if (step) {
						const message = new vscode.TestMessage(`${ev.crit} — expected: ${ev.expected}, actual: ${ev.actual}`);
						message.expectedOutput = ev.expected;
						message.actualOutput = ev.actual;
						failed.add(step);
						run.failed(step, message);
					}
					return;
				}
				case 'wf_done': {
					const wf = wfItems.get(ev.wf);
					if (!wf) return;
					// A step that ran without a failing criterion passed; one
					// the runner never reached (it stops at the first failure)
					// is skipped, not silently left pending.
					wf.children.forEach((step) => {
						if (failed.has(step)) return;
						if (started.has(step) || ev.passed) run.passed(step);
						else run.skipped(step);
					});
					if (ev.passed) {
						run.passed(wf);
					} else {
						run.failed(wf, new vscode.TestMessage(`workflow ${ev.wf} finished with failures`));
					}
					return;
				}
			}
		};

		const handle = spawnSuspectRunWith(
			job.file,
			{ baseUrl: job.baseUrl, cassette: job.cassette, filter, cwd: job.cwd },
			onEvent,
			binary,
		);
		const cancelSub = token.onCancellationRequested(() => handle.kill());
		try {
			await handle.done;
		} catch (err) {
			const message = new vscode.TestMessage(errorMessage(err));
			for (const wf of wfItems.values()) run.errored(wf, message);
		} finally {
			cancelSub.dispose();
		}
	}
}
