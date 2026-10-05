// The Suspect Overview: the project's spec tooling in one tree.
//
// The activity-bar container used to hold only the workflows list. The
// Overview adds what the improved LSP makes possible — findings by
// severity from the published diagnostics, the detected configuration
// surface, and the runtime state (gateway, SDK session) — so the
// extension's own functionality is manageable from a native view instead
// of scattered commands.

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { SEVERITY_ORDER, SeverityCount, updateStatus } from './status';

type Node =
	| { kind: 'section'; id: string; label: string; icon: string; state: string }
	| { kind: 'finding'; severity: keyof SeverityCount; label: string }
	| { kind: 'file'; uri: vscode.Uri; label: string; detail?: string }
	| { kind: 'runtime'; id: string; label: string; detail: string; command?: string }
	| { kind: 'server'; label: string; detail: string };

/** The configuration surface a suspect project can carry. Pure: testable. */
export interface ProjectFiles {
	entry: string | undefined;
	configs: { name: string; path: string }[];
	overlays: string[];
	workflows: string[];
}

export function detectProjectFiles(root: string): ProjectFiles {
	const exists = (rel: string) => fs.existsSync(path.join(root, rel));
	const readDir = (rel: string) => {
		try {
			return fs.readdirSync(path.join(root, rel))
				.filter((f) => /\.(yaml|yml|json)$/.test(f))
				.map((f) => `${rel}/${f}`);
		} catch {
			return [];
		}
	};
	const configs: { name: string; path: string }[] = [];
	if (exists('.suspect.yaml')) configs.push({ name: 'Workspace settings', path: '.suspect.yaml' });
	if (exists('suspect.project.json')) configs.push({ name: 'Project manifest', path: 'suspect.project.json' });
	return {
		entry: ['openapi.yaml', 'spec.yaml', 'main.yaml'].find(exists),
		configs,
		overlays: readDir('overlays'),
		workflows: readDir('workflows'),
	};
}

export class OverviewProvider implements vscode.TreeDataProvider<Node> {
	private readonly emitter = new vscode.EventEmitter<Node | undefined>();
	readonly onDidChangeTreeData = this.emitter.event;

	refresh(): void {
		this.emitter.fire(undefined);
	}

	getChildren(node?: Node): ProviderResult<Node[]> {
		if (node === undefined) {
			return this.sections();
		}
		switch (node.kind) {
			case 'section': {
				switch (node.id) {
					case 'findings': return this.findings();
					case 'project': return this.project();
					case 'runtime': return this.runtime();
					default: return [];
				}
			}
			default:
				return [];
		}
	}

	getTreeItem(node: Node): vscode.TreeItem {
		switch (node.kind) {
			case 'section': {
				const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.Expanded);
				item.iconPath = new vscode.ThemeIcon(node.icon);
				item.description = node.state;
				item.contextValue = `section:${node.id}`;
				return item;
			}
			case 'finding': {
				const order = SEVERITY_ORDER.find((s) => s.key === node.severity);
				const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.None);
				item.iconPath = new vscode.ThemeIcon(order ? order.icon.replace('$(', '').replace(')', '') : 'circle-filled');
				item.contextValue = 'finding';
				item.command = {
					command: 'workbench.action.problems.focus',
					title: 'Show in Problems',
				};
				return item;
			}
			case 'file': {
				const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.None);
				item.description = node.detail;
				item.command = { command: 'vscode.open', title: 'Open', arguments: [node.uri] };
				return item;
			}
			case 'runtime': {
				const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.None);
				item.description = node.detail;
				if (node.command !== undefined) {
					item.command = { command: node.command, title: 'Run', arguments: [] };
				}
				return item;
			}
			case 'server': {
				const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.None);
				item.description = node.detail;
				return item;
			}
		}
	}

	private sections(): Node[] {
		const counts = updateStatus();
		const findingsState = counts.error + counts.warning + counts.information + counts.hint;
		const files = detectProjectFiles(this.root());
		const projectState = [files.entry !== undefined ? 'spec' : null, `${files.configs.length} config`, files.workflows.length > 0 ? `${files.workflows.length} workflow` : null]
			.filter((x) => x !== null)
			.join(' · ');
		return [
			{ kind: 'section', id: 'findings', label: 'Findings', icon: '$(symbol-misc)', state: findingsState > 0 ? String(findingsState) : 'none' },
			{ kind: 'section', id: 'project', label: 'Project', icon: '$(folder-library)', state: projectState || 'no spec found' },
			{ kind: 'section', id: 'runtime', label: 'Runtime', icon: '$(play)', state: '' },
		];
	}

	private findings(): Node[] {
		const counts = updateStatus();
		return SEVERITY_ORDER
			.map(({ key, label, icon }) => ({ key, label, icon, count: counts[key] }))
			.filter((s) => s.count > 0)
			.map((s) => ({ kind: 'finding' as const, severity: s.key, label: `${s.label} (${s.count})` }));
	}

	private project(): Node[] {
		const root = this.root();
		const files = detectProjectFiles(root);
		const out: Node[] = [];
		if (files.entry !== undefined) {
			out.push({ kind: 'file', uri: vscode.Uri.file(path.join(root, files.entry)), label: files.entry, detail: 'entry spec' });
		}
		for (const config of files.configs) {
			out.push({ kind: 'file', uri: vscode.Uri.file(path.join(root, config.path)), label: config.path, detail: config.name });
		}
		for (const overlay of files.overlays) {
			out.push({ kind: 'file', uri: vscode.Uri.file(path.join(root, overlay)), label: path.basename(overlay), detail: 'overlay' });
		}
		return out;
	}

	private runtime(): Node[] {
		const files = detectProjectFiles(this.root());
		const out: Node[] = [
			{ kind: 'runtime', id: 'gateway', label: 'Mock gateway', detail: 'start/stop', command: '_suspect.toggleGateway' },
			{ kind: 'runtime', id: 'session', label: 'SDK session', detail: 'preview/watch', command: 'suspect.previewSdk' },
			{ kind: 'runtime', id: 'workflows', label: 'Workflows', detail: 'run', command: 'suspect.runWorkflow' },
		];
		if (files.workflows.length > 0) {
			out.push({ kind: 'file', uri: vscode.Uri.file(path.join(this.root(), files.workflows[0])), label: path.basename(files.workflows[0]), detail: 'first workflow' });
		}
		return out;
	}

	private root(): string {
		return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? '';
	}
}

type ProviderResult<T> = vscode.ProviderResult<T>;

export function registerOverview(context: vscode.ExtensionContext): void {
	const provider = new OverviewProvider();
	context.subscriptions.push(
		vscode.window.registerTreeDataProvider('suspect.overview', provider),
	);
	context.subscriptions.push(
		vscode.languages.onDidChangeDiagnostics(() => provider.refresh()),
	);
}
