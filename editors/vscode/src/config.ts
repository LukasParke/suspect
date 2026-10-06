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

const FLOORS: { label: string; detail: string; value: string }[] = [
	{ label: 'Error', detail: 'only errors — the publish gate', value: 'error' },
	{ label: 'Warning', detail: 'errors and warnings — the recommended default', value: 'warning' },
	{ label: 'Information', detail: 'everything but hints', value: 'information' },
	{ label: 'Hint', detail: 'show everything', value: 'hint' },
];

export interface FloorResolution {
	value: string;
	/** Where the value comes from, in the words the UI shows. */
	source: 'editor setting' | '.suspect.yaml' | 'suspect.project.json' | 'default';
}

/**
 * The floor in effect, by precedence: the editor setting, the committed
 * `.suspect.yaml`, the project manifest, then the server default. Pure.
 */
export function resolveFloor(input: { setting?: string; workspaceYaml?: string; manifest?: string }): FloorResolution {
	if (input.setting) return { value: input.setting, source: 'editor setting' };
	const fromYaml = input.workspaceYaml?.match(/^\s*min_severity:\s*(\w+)/m)?.[1];
	if (fromYaml) return { value: fromYaml, source: '.suspect.yaml' };
	if (input.manifest) return { value: input.manifest, source: 'suspect.project.json' };
	return { value: 'hint', source: 'default' };
}

/** `resolveFloor` over the live workspace. */
export async function effectiveFloor(root: string | undefined, manifestFloor?: string): Promise<FloorResolution> {
	const setting = vscode.workspace.getConfiguration('suspect').get<string>('lint.minSeverity');
	let workspaceYaml: string | undefined;
	if (root !== undefined) {
		try {
			workspaceYaml = await fs.promises.readFile(path.join(root, '.suspect.yaml'), 'utf8');
		} catch {
			workspaceYaml = undefined;
		}
	}
	return resolveFloor({ setting, workspaceYaml, manifest: manifestFloor });
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
