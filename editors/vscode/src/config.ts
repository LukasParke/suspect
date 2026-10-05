// Configuration management with a UI, instead of hand-editing YAML.
//
// The server already re-derives its severity floor when the client's
// settings change — the same path the regression test
// `a_configuration_change_refilters_diagnostics` pins — so these commands
// are live controls, not "restart and pray": pick a floor, the editor and
// the Problems panel re-filter within the session.

import * as path from 'path';
import * as vscode from 'vscode';
import { detectProjectFiles } from './overview';

const FLOORS: { label: string; detail: string; value: string }[] = [
	{ label: 'Error', detail: 'only errors — the publish gate', value: 'error' },
	{ label: 'Warning', detail: 'errors and warnings — the recommended default', value: 'warning' },
	{ label: 'Information', detail: 'everything but hints', value: 'information' },
	{ label: 'Hint', detail: 'show everything', value: 'hint' },
];

/** The configured floor, falling back to what a committed `.suspect.yaml` says. */
export async function currentFloor(): Promise<string> {
	const fromSettings = vscode.workspace.getConfiguration('suspect').get<string>('lint.minSeverity');
	if (fromSettings !== undefined && fromSettings !== '') {
		return fromSettings;
	}
	// The file's floor is the server's real default; read it so the pick
	// shows the truth instead of a stale label.
	const root = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
	if (root === undefined) return 'hint';
	try {
		const document = await vscode.workspace.openTextDocument(path.join(root, '.suspect.yaml'));
		const match = document.getText().match(/min_severity:\s*(\w+)/);
		return match ? match[1] : 'hint';
	} catch {
		return 'hint';
	}
}

export async function setSeverityFloor(): Promise<void> {
	const current = await currentFloor();
	const picked = await vscode.window.showQuickPick(
		FLOORS.map((f) => ({ label: f.label, description: f.detail, picked: f.value === current })),
		{ placeHolder: `Findings floor — currently ${current}` },
	);
	if (picked === undefined) return;
	const chosen = FLOORS.find((f) => f.label === picked.label);
	if (chosen === undefined) return;
	await vscode.workspace
		.getConfiguration('suspect')
		.update('lint.minSeverity', chosen.value, vscode.ConfigurationTarget.Workspace);
	void vscode.window.showInformationMessage(
		`Suspect floor set to ${chosen.label.toLowerCase()}. Findings re-filter in the editor and the Problems panel.`,
	);
}

/** Opens the detected config surface: the committed files, or the editor settings when there are none. */
export async function openSuspectConfig(): Promise<void> {
	const root = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
	if (root === undefined) {
		void vscode.window.showWarningMessage('Open a folder first — suspect configuration is per-project.');
		return;
	}
	const files = detectProjectFiles(root);
	const picks: { label: string; description: string; target: vscode.Uri | 'settings' }[] = [];
	for (const config of files.configs) {
		picks.push({
			label: config.path,
			description: config.name,
			target: vscode.Uri.file(path.join(root, config.path)),
		});
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
