/**
 * The server's own `suspect config --explain`: `suspect/explainConfig`
 * reports every effective setting with the layer it came from, editor
 * settings included (the CLI cannot see those). A status-bar item names
 * the settings file in effect and opens the report; the severity-floor
 * picker reads its floor from the same report.
 */
import * as path from 'path';
import * as vscode from 'vscode';
import type { LanguageClient } from 'vscode-languageclient/node';
import { requestVersion } from './protocol';

export const EXPLAIN_CONFIG_REQUEST = 'suspect/explainConfig';
export const EXPLAIN_CONFIG_COMMAND = 'suspect.explainConfig';

/** One row: a setting, its effective value, and every layer that set it, lowest first. */
export interface ExplainRow {
	key: string;
	value: unknown;
	/** `default`, `.suspect.yaml:3`, `env SUSPECT_...`, `editor suspect.lint.minSeverity`. */
	origin: string;
	/** `default`, `file`, `manifest`, `env`, `editor` or `flag`. */
	layer: string;
	trace: [string, unknown][];
}

/** The `suspect/explainConfig` result. */
export interface ExplainReport {
	root: string;
	config: string | null;
	/** The file a status bar names, or `null` for built-in defaults. */
	source: string | null;
	/** The first rejected value, or `null`. */
	error: string | null;
	files: string[];
	values: ExplainRow[];
	warnings: string[];
	errors: string[];
}

type ClientGetter = () => LanguageClient | undefined;

/** The report for `uri` (the workspace's without one); `undefined` when no server offers the request or it fails. */
export async function explainConfig(client: LanguageClient | undefined, uri?: vscode.Uri): Promise<ExplainReport | undefined> {
	if (!client || requestVersion(client, EXPLAIN_CONFIG_REQUEST) === undefined) return undefined;
	try {
		const params = uri?.scheme === 'file' ? { uri: uri.toString() } : {};
		const report = await client.sendRequest<ExplainReport>(EXPLAIN_CONFIG_REQUEST, params);
		return Array.isArray(report?.values) ? report : undefined;
	} catch {
		return undefined;
	}
}

/** The row for `key`. */
export function explainRow(report: ExplainReport | undefined, key: string): ExplainRow | undefined {
	return report?.values.find((row) => row.key === key);
}

/** Where a row's value came from, in the words the UI shows: `editor setting`, `.suspect.yaml`, `default`, ... Pure. */
export function layerLabel(row: Pick<ExplainRow, 'layer' | 'origin'>): string {
	switch (row.layer) {
		case 'editor':
			return 'editor setting';
		case 'file':
		case 'manifest':
			// `path/to/.suspect.yaml:3` -> `.suspect.yaml`
			return path.basename(row.origin.replace(/:\d+$/, ''));
		case 'env':
			return row.origin.replace(/^env /, '');
		default:
			return row.layer;
	}
}

/** Status-bar text and tooltip for a report. Pure. */
export function statusText(report: ExplainReport): { text: string; tooltip: string } {
	const source = report.source ?? 'defaults';
	if (report.error !== null) {
		return { text: `$(warning) suspect: ${source}`, tooltip: `Suspect configuration: ${report.error}. Click for every setting and where it came from.` };
	}
	const where = report.config ?? (report.source === null ? 'built-in defaults' : report.source);
	return { text: `suspect: ${source}`, tooltip: `Suspect configuration from ${where}. Click for every setting and where it came from.` };
}

const shown = (value: unknown): string => (value === null || value === undefined ? '(unset)' : typeof value === 'string' ? value : JSON.stringify(value));

/** The report as a Markdown document. Pure. */
export function renderReport(report: ExplainReport): string {
	const lines = [
		'# Suspect configuration',
		'',
		`Settings file: ${report.config ?? 'none (built-in defaults)'}`,
		`Root: ${report.root}`,
		'',
		'Precedence: default < manifest (deprecated) < settings file < environment < editor setting.',
		'',
		'| Key | Value | From | Layers (lowest first) |',
		'|---|---|---|---|',
		...report.values.map((row) => {
			const trace = row.trace.map(([origin, value]) => `${origin} = ${shown(value)}`).join('; ');
			return `| \`${row.key}\` | ${shown(row.value).replace(/\|/g, '\\|')} | ${layerLabel(row)} | ${trace.replace(/\|/g, '\\|')} |`;
		}),
	];
	for (const [title, list] of [['Rejected values', report.errors], ['Warnings', report.warnings]] as const) {
		if (list.length) lines.push('', `## ${title}`, '', ...list.map((item) => `- ${item}`));
	}
	return lines.join('\n') + '\n';
}

export interface ConfigStatus {
	disposables: vscode.Disposable[];
	/** Re-reads the report for the active document (after the client starts or settings change). */
	refresh(): Promise<void>;
}

/** The status-bar item and the `suspect.explainConfig` command. */
export function registerConfigStatus(getClient: ClientGetter, watcher: vscode.FileSystemWatcher | undefined): ConfigStatus {
	const item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 91);
	item.name = 'Suspect Configuration';
	item.command = EXPLAIN_CONFIG_COMMAND;
	let latest = 0;
	const activeUri = (): vscode.Uri | undefined => vscode.window.activeTextEditor?.document.uri;
	const refresh = async (): Promise<void> => {
		const ticket = ++latest;
		const report = await explainConfig(getClient(), activeUri());
		if (ticket !== latest) return;
		if (report === undefined) {
			item.hide();
			return;
		}
		const { text, tooltip } = statusText(report);
		item.text = text;
		item.tooltip = tooltip;
		item.show();
	};
	// The server reloads asynchronously after a change; ask once it has.
	let timer: ReturnType<typeof setTimeout> | undefined;
	const later = (): void => {
		if (timer) clearTimeout(timer);
		timer = setTimeout(() => void refresh(), 300);
	};
	const disposables: vscode.Disposable[] = [
		item,
		vscode.window.onDidChangeActiveTextEditor(() => void refresh()),
		vscode.workspace.onDidChangeConfiguration((event) => {
			if (event.affectsConfiguration('suspect')) later();
		}),
		vscode.commands.registerCommand(EXPLAIN_CONFIG_COMMAND, async () => {
			const report = await explainConfig(getClient(), activeUri());
			if (report === undefined) {
				void vscode.window.showWarningMessage('This suspect CLI cannot explain its configuration; update it, or run `suspect config --explain` in a terminal.');
				return;
			}
			const document = await vscode.workspace.openTextDocument({ content: renderReport(report), language: 'markdown' });
			await vscode.window.showTextDocument(document, { preview: true });
		}),
		{ dispose: () => timer && clearTimeout(timer) },
	];
	if (watcher) disposables.push(watcher.onDidChange(later), watcher.onDidCreate(later), watcher.onDidDelete(later));
	return { disposables, refresh };
}
