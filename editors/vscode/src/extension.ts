import * as cp from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions, TransportKind } from 'vscode-languageclient/node';
import { parseArazzo } from './parse';
import { registerNotebook } from './notebook';
import { errorMessage, gatewayPort, spawnSuspectRun, suspectBinary } from './runner';
import { registerTestExplorer } from './testExplorer';
import { registerWorkflowsView } from './workflowsView';

interface GatewayState {
	child: cp.ChildProcess;
	port: number;
	specLabel: string;
}

let client: LanguageClient | undefined;
let gateway: GatewayState | undefined;
let gatewayStatus: vscode.StatusBarItem | undefined;

export function activate(context: vscode.ExtensionContext): void {
	context.subscriptions.push(
		vscode.commands.registerCommand('suspect.runWorkflow', (uri?: vscode.Uri, workflow?: string) =>
			runWorkflowCommand(uri, workflow),
		),
		vscode.commands.registerCommand('suspect.startGateway', () => startGateway()),
		vscode.commands.registerCommand('suspect.stopGateway', () => stopGateway()),
		vscode.commands.registerCommand('_suspect.toggleGateway', () => (gateway ? stopGateway() : void startGateway())),
		vscode.commands.registerCommand('suspect.genPreset', () => genPresetCommand()),
		vscode.commands.registerCommand('suspect.openNotebook', (uri?: vscode.Uri) => openNotebook(uri)),
	);

	gatewayStatus = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 90);
	gatewayStatus.command = '_suspect.toggleGateway';
	gatewayStatus.name = 'Suspect Gateway';
	context.subscriptions.push(gatewayStatus);

	registerTestExplorer(context);
	registerWorkflowsView(context);
	registerNotebook(context);

	startClient();
	context.subscriptions.push(
		vscode.workspace.onDidChangeConfiguration((event) => {
			if (event.affectsConfiguration('suspect.basePath')) {
				void restartClient();
			}
		}),
	);
}

export async function deactivate(): Promise<void> {
	const tasks: Promise<unknown>[] = [];
	if (client !== undefined) {
		tasks.push(Promise.resolve(client.stop()).catch(() => undefined));
	}
	stopGateway();
	await Promise.all(tasks);
}

function startClient(): void {
	const serverOptions: ServerOptions = {
		command: suspectBinary(),
		args: ['lsp'],
		transport: TransportKind.stdio,
	};
	const clientOptions: LanguageClientOptions = {
		documentSelector: [
			{ scheme: 'file', language: 'yaml' },
			{ scheme: 'file', language: 'json' },
		],
	};
	client = new LanguageClient('suspect', 'Suspect', serverOptions, clientOptions);
	void Promise.resolve(client.start()).catch((err: unknown) => {
		vscode.window.showWarningMessage(`Suspect language server failed to start: ${errorMessage(err)}`);
		client = undefined;
	});
}

async function restartClient(): Promise<void> {
	if (client !== undefined) {
		const current = client;
		client = undefined;
		await Promise.resolve(current.stop()).catch(() => undefined);
	}
	startClient();
}

async function pickArazzoDocument(hint?: vscode.Uri): Promise<vscode.Uri | undefined> {
	if (hint) {
		return hint;
	}
	const active = vscode.window.activeTextEditor?.document;
	if (active && /\.arazzo\.ya?ml$/i.test(active.fileName)) {
		return active.uri;
	}
	const uris = await vscode.workspace.findFiles('**/*.arazzo.{yaml,yml}', '**/node_modules/**');
	if (uris.length === 0) {
		vscode.window.showErrorMessage('No *.arazzo.yaml documents found in the workspace.');
		return undefined;
	}
	if (uris.length === 1) {
		return uris[0];
	}
	const pick = await vscode.window.showQuickPick(
		uris.map((uri) => ({ label: vscode.workspace.asRelativePath(uri), uri })),
		{ placeHolder: 'Select an Arazzo document' },
	);
	return pick?.uri;
}

async function pickWorkflowId(uri: vscode.Uri): Promise<string | undefined> {
	let workflows;
	try {
		workflows = parseArazzo(await fs.promises.readFile(uri.fsPath, 'utf8'));
	} catch (err) {
		vscode.window.showErrorMessage(`Could not read ${vscode.workspace.asRelativePath(uri)}: ${errorMessage(err)}`);
		return undefined;
	}
	if (workflows.length === 0) {
		vscode.window.showErrorMessage(`No workflows found in ${vscode.workspace.asRelativePath(uri)}.`);
		return undefined;
	}
	if (workflows.length === 1) {
		return workflows[0].workflowId;
	}
	const pick = await vscode.window.showQuickPick(
		workflows.map((wf) => ({ label: wf.workflowId, description: `${wf.steps.length} step(s)` })),
		{ placeHolder: 'Select a workflow to run' },
	);
	return pick?.label;
}

async function runWorkflowCommand(uriHint?: vscode.Uri, workflowHint?: string): Promise<void> {
	const uri = await pickArazzoDocument(uriHint);
	if (!uri) {
		return;
	}
	const workflowId = workflowHint ?? (await pickWorkflowId(uri));
	if (!workflowId) {
		return;
	}
	let startedSteps = 0;
	let finishedSteps = 0;
	await vscode.window.withProgress(
		{
			location: vscode.ProgressLocation.Notification,
			title: `Suspect: running '${workflowId}'`,
			cancellable: true,
		},
		async (progress, token) => {
			progress.report({ message: 'starting…' });
			const handle = spawnSuspectRun(uri.fsPath, workflowId, (event) => {
				switch (event.event) {
					case 'step_started':
						startedSteps += 1;
						break;
					case 'response_got':
					case 'criterion_fail':
						finishedSteps += 1;
						break;
					default:
						break;
				}
				progress.report({ message: `${Math.max(startedSteps - finishedSteps, 0)} running · ${finishedSteps} finished` });
			});
			token.onCancellationRequested(() => handle.kill());
			try {
				const totals = await handle.done;
				vscode.window.showInformationMessage(
					`Suspect run '${workflowId}': ${totals.passed} passed, ${totals.failed} failed.`,
				);
			} catch (err) {
				vscode.window.showErrorMessage(`Suspect run failed: ${errorMessage(err)}`);
			}
		},
	);
}

async function pickOpenApiSpec(): Promise<vscode.Uri | undefined> {
	const candidates = new Map<string, vscode.Uri>();
	for (const pattern of ['**/*.openapi.{yaml,yml,json}', '**/openapi*.{yaml,yml,json}', '**/swagger*.{yaml,yml,json}']) {
		for (const uri of await vscode.workspace.findFiles(pattern, '**/node_modules/**')) {
			candidates.set(uri.fsPath, uri);
		}
	}
	const activeUri = vscode.window.activeTextEditor?.document.uri;
	if (activeUri && !/\.arazzo\.ya?ml$/i.test(activeUri.fsPath) && fs.existsSync(activeUri.fsPath)) {
		candidates.set(activeUri.fsPath, activeUri);
	}
	if (candidates.size === 0) {
		vscode.window.showErrorMessage('No OpenAPI spec (*.openapi.yaml / openapi.json / swagger.*) found.');
		return undefined;
	}
	if (candidates.size === 1) {
		return [...candidates.values()][0];
	}
	const pick = await vscode.window.showQuickPick(
		[...candidates.values()].map((uri) => ({ label: vscode.workspace.asRelativePath(uri), uri })),
		{ placeHolder: 'Select an OpenAPI spec' },
	);
	return pick?.uri;
}

async function startGateway(): Promise<void> {
	if (gateway !== undefined) {
		vscode.window.showInformationMessage(`Suspect gateway already running on port ${gateway.port}.`);
		return;
	}
	const spec = await pickOpenApiSpec();
	if (!spec) {
		return;
	}
	const port = gatewayPort();
	const specLabel = vscode.workspace.asRelativePath(spec);
	let child: cp.ChildProcess;
	try {
		child = cp.spawn(suspectBinary(), ['gateway', spec.fsPath, '--port', String(port), '--mode', 'mock'], {
			stdio: 'ignore',
		});
	} catch (err) {
		vscode.window.showErrorMessage(`Failed to launch suspect gateway: ${errorMessage(err)}`);
		return;
	}
	gateway = { child, port, specLabel };
	child.on('error', (err: Error) => {
		vscode.window.showErrorMessage(`Suspect gateway error: ${errorMessage(err)}`);
		if (gateway?.child === child) {
			gateway = undefined;
			updateGatewayStatus();
		}
	});
	child.on('exit', () => {
		if (gateway?.child === child) {
			gateway = undefined;
			updateGatewayStatus();
		}
	});
	updateGatewayStatus();
	vscode.window.showInformationMessage(`Suspect gateway mocking ${specLabel} on http://127.0.0.1:${port}.`);
}

function stopGateway(): void {
	if (gateway === undefined) {
		vscode.window.showInformationMessage('Suspect gateway is not running.');
		return;
	}
	const state = gateway;
	gateway = undefined;
	state.child.kill('SIGTERM');
	updateGatewayStatus();
}

function updateGatewayStatus(): void {
	if (gatewayStatus === undefined) {
		return;
	}
	if (gateway === undefined) {
		gatewayStatus.text = '$(circle-slash) suspect';
		gatewayStatus.tooltip = 'Suspect: start mock gateway';
		gatewayStatus.hide();
		return;
	}
	gatewayStatus.text = `$(broadcast) suspect:${gateway.port}`;
	gatewayStatus.tooltip = `Suspect gateway mocking ${gateway.specLabel} on http://127.0.0.1:${gateway.port} — click to stop`;
	gatewayStatus.show();
}

async function genPresetCommand(): Promise<void> {
	const presetPick = await vscode.window.showQuickPick(['docs-md', 'ts-sdk', 'rust-sdk'], {
		placeHolder: 'Suspect: choose a generation preset',
	});
	if (!presetPick) {
		return;
	}
	const spec = await pickOpenApiSpec();
	if (!spec) {
		return;
	}
	const folder =
		vscode.workspace.getWorkspaceFolder(spec) ??
		vscode.workspace.workspaceFolders?.[0];
	if (!folder) {
		vscode.window.showErrorMessage('No workspace folder is open.');
		return;
	}
	const outDir = path.join(folder.uri.fsPath, 'gen-out', presetPick);
	try {
		await vscode.window.withProgress(
			{ location: vscode.ProgressLocation.Notification, title: `Suspect gen: ${presetPick}` },
			() =>
				new Promise<void>((resolve, reject) => {
					const child: cp.ChildProcess = cp.spawn(
						suspectBinary(),
						['gen', spec.fsPath, '--preset', presetPick, '--out', outDir],
						{ stdio: ['ignore', 'ignore', 'pipe'] },
					);
					let stderrTail = '';
					child.stderr?.on('data', (chunk: Buffer) => {
						stderrTail = (stderrTail + chunk.toString()).slice(-2000);
					});
					child.on('error', (err: Error) => reject(err));
					child.on('exit', (code) => {
						if (code === 0) {
							resolve();
							return;
						}
						reject(new Error(`suspect gen exited with code ${code}${stderrTail.trim() ? `: ${stderrTail.trim()}` : ''}`));
					});
				}),
		);
	} catch (err) {
		vscode.window.showErrorMessage(`Suspect gen failed: ${errorMessage(err)}`);
		return;
	}
	const produced = await firstFileRecursive(outDir);
	if (!produced) {
		vscode.window.showWarningMessage(`suspect gen produced no files under gen-out/${presetPick}.`);
		return;
	}
	const doc = await vscode.workspace.openTextDocument(produced);
	await vscode.window.showTextDocument(doc);
}

async function firstFileRecursive(dir: string): Promise<string | undefined> {
	let entries: fs.Dirent[];
	try {
		entries = await fs.promises.readdir(dir, { withFileTypes: true });
	} catch {
		return undefined;
	}
	entries.sort((a, b) => a.name.localeCompare(b.name));
	for (const entry of entries) {
		const full = path.join(dir, entry.name);
		if (entry.isFile()) {
			return full;
		}
		if (entry.isDirectory()) {
			const nested = await firstFileRecursive(full);
			if (nested) {
				return nested;
			}
		}
	}
	return undefined;
}

async function openNotebook(uriHint?: vscode.Uri): Promise<void> {
	const uri = await pickArazzoDocument(uriHint);
	if (!uri) {
		return;
	}
	await vscode.commands.executeCommand('vscode.openWith', uri, 'suspect.notebook');
}
