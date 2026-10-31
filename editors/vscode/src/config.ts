// Configuration management with a UI, instead of hand-editing YAML.
//
// The server already re-derives its severity floor when the client's
// settings change — the same path the regression test
// `a_configuration_change_refilters_diagnostics` pins — so these commands
// are live controls, not "restart and pray": pick a floor, the editor and
// the Problems panel re-filter within the session.

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import type { LanguageClient } from 'vscode-languageclient/node';
import { ExplainReport, explainConfig, explainRow, layerLabel } from './explainConfig';
import { lintMinSeverity } from './settings.generated';

const FLOORS: { label: string; detail: string; value: string }[] = [
	{ label: 'Error', detail: 'only errors — the publish gate', value: 'error' },
	{ label: 'Warning', detail: 'errors and warnings — the recommended default', value: 'warning' },
	{ label: 'Information', detail: 'everything but hints', value: 'info' },
	{ label: 'Hint', detail: 'show everything', value: 'hint' },
];

export interface FloorResolution {
	value: string;
	/** Where the value comes from, in the words the UI shows: `editor setting`, `.suspect.yaml`, `default`, ... */
	source: string;
}

/**
 * The floor in effect. The server's `suspect/explainConfig` report is the
 * answer (it resolves the files, the environment and the editor settings
 * exactly as diagnostics do); without one (no server, or one that predates
 * the request) only the editor setting and the manifest's deprecated floor
 * are known. Pure.
 */
export function resolveFloor(input: { setting?: string | null; report?: ExplainReport; manifest?: string }): FloorResolution {
	const row = explainRow(input.report, 'lint.min_severity');
	if (row && typeof row.value === 'string') return { value: row.value, source: layerLabel(row) };
	if (input.setting) return { value: input.setting, source: 'editor setting' };
	if (input.manifest) return { value: input.manifest, source: 'suspect.project.json' };
	return { value: 'hint', source: 'default' };
}

let clientOf: () => LanguageClient | undefined = () => undefined;

/** The running language client the floor is read from. */
export function useClient(getClient: () => LanguageClient | undefined): void {
	clientOf = getClient;
}

/** `resolveFloor` over the live workspace: the configuration of a document in `root`. */
export async function effectiveFloor(root: string | undefined, manifestFloor?: string): Promise<FloorResolution> {
	const at = root === undefined ? undefined : vscode.Uri.file(path.join(root, 'suspect.project.json'));
	const report = await explainConfig(clientOf(), at);
	return resolveFloor({ setting: lintMinSeverity(), report, manifest: manifestFloor });
}

function workspaceRoot(): string | undefined {
	return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

export async function setSeverityFloor(): Promise<void> {
	const current = await effectiveFloor(workspaceRoot());
	const picked = await vscode.window.showQuickPick(
		FLOORS.map((f) => ({ label: f.label, description: f.detail, picked: f.value === current.value })),
		{ placeHolder: `Minimum severity of reported findings — currently ${current.value} (${current.source})` },
	);
	if (picked === undefined) return;
	const chosen = FLOORS.find((f) => f.label === picked.label);
	if (chosen === undefined) return;
	await vscode.workspace
		.getConfiguration('suspect')
		.update('lint.minSeverity', chosen.value, vscode.ConfigurationTarget.Workspace);
	void vscode.window.showInformationMessage(
		`Suspect now reports ${chosen.label.toLowerCase()} and above. Findings re-filter in the editor and the Problems panel.`,
	);
}

/** Opens the configuration surface: the committed files, or the editor settings. */
export async function openSuspectConfig(): Promise<void> {
	const root = workspaceRoot();
	if (root === undefined) {
		void vscode.window.showWarningMessage('Open a folder first — suspect configuration is per-project.');
		return;
	}
	const picks: { label: string; description: string; target: vscode.Uri | 'settings' }[] = [];
	for (const [file, description] of [['.suspect.yaml', 'workspace policy'], ['suspect.project.json', 'project manifest']] as const) {
		const full = path.join(root, file);
		if (fs.existsSync(full)) picks.push({ label: file, description, target: vscode.Uri.file(full) });
	}
	picks.push({ label: 'Editor settings', description: 'suspect.* in settings.json', target: 'settings' });
	const picked = await vscode.window.showQuickPick(picks, { placeHolder: 'Open suspect configuration' });
	if (picked === undefined) return;
	if (picked.target === 'settings') {
		void vscode.commands.executeCommand('workbench.action.openWorkspaceSettings', 'suspect');
		return;
	}
	void vscode.window.showTextDocument(picked.target);
}
