// The findings/status surface: one status-bar item summarising the
// specification's health, one place to jump from it.
//
// The counts come from the language client's published diagnostics, so
// the item is live: it moves as findings are fixed, and clicking it opens
// the Overview view where each severity is navigable.

import * as vscode from 'vscode';

export interface SeverityCount {
	error: number;
	warning: number;
	information: number;
	hint: number;
}

export const SEVERITY_ORDER: { key: keyof SeverityCount; label: string; icon: string }[] = [
	{ key: 'error', label: 'Errors', icon: '$(error)' },
	{ key: 'warning', label: 'Warnings', icon: '$(warning)' },
	{ key: 'information', label: 'Information', icon: '$(info)' },
	{ key: 'hint', label: 'Hints', icon: '$(lightbulb)' },
];

/** Counts a diagnostic list by severity. Pure: unit-tested without VS Code. */
export function countSeverities(diagnostics: readonly { severity: number }[]): SeverityCount {
	const out: SeverityCount = { error: 0, warning: 0, information: 0, hint: 0 };
	for (const diagnostic of diagnostics) {
		switch (diagnostic.severity) {
			case 0: out.error += 1; break;
			case 1: out.warning += 1; break;
			case 2: out.information += 1; break;
			case 3: out.hint += 1; break;
			default: break;
		}
	}
	return out;
}

/** The status-bar text for a count: leading with what blocks work first. */
export function statusText(counts: SeverityCount): string {
	const parts: string[] = [];
	if (counts.error > 0) parts.push(`$(error) ${counts.error}`);
	if (counts.warning > 0) parts.push(`$(warning) ${counts.warning}`);
	if (parts.length === 0) return `$(check) 0 problems`;
	return `Suspect ${parts.join('  ')}`;
}

let item: vscode.StatusBarItem | undefined;

export function registerStatus(context: vscode.ExtensionContext): void {
	item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 95);
	item.name = 'Suspect Findings';
	item.command = 'suspect.showOverview';
	item.tooltip = 'Suspect findings — click to open the Overview';
	context.subscriptions.push(item);
	item.show();

	const refresh = () => updateStatus();
	context.subscriptions.push(vscode.languages.onDidChangeDiagnostics(refresh));
	refresh();
}

export function updateStatus(): SeverityCount {
	const all = vscode.languages.getDiagnostics();
	let flat: { severity: number }[] = [];
	for (const [, list] of all) {
		for (const d of list) {
			// Only what the suspect server published: markers carry the
			// source we set on every finding.
			if (d.source === 'suspect' || d.source === 'suspect-lint') {
				flat.push(d);
			}
		}
	}
	const counts = countSeverities(flat);
	if (item !== undefined) {
		item.text = statusText(counts);
		item.tooltip = new vscode.MarkdownString(
			`**Suspect** — ${counts.error} errors, ${counts.warning} warnings, ${counts.information} info, ${counts.hint} hints\n\nClick to open the Overview`,
		);
	}
	return counts;
}
