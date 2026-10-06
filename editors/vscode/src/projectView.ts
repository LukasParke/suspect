// The Suspect side bar: one view, the project as its manifest declares it.
//
// Findings live in the Problems panel and the editor's own status-bar
// counter; tests live in the Testing view. What neither shows is the
// shape of the project — which document is the entry spec, which overlays
// each publish profile applies, which SDK targets ship, which suites are
// the contract, and what policy the configuration sets. That is this view.
// Every row opens the thing it names; the one control is the severity
// floor, whose current value is shown rather than hidden behind a command.

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { effectiveFloor } from './config';
import { DiscoveredProject, affectsProject, findProjects } from './discover';

type Node =
	| { kind: 'project'; project: DiscoveredProject }
	| { kind: 'section'; id: string; label: string; icon: string; detail: string; project: DiscoveredProject }
	| { kind: 'file'; label: string; detail?: string; uri: vscode.Uri; icon?: string; missing?: boolean }
	| { kind: 'profile'; label: string; overlays: string[]; project: DiscoveredProject }
	| { kind: 'floor'; label: string; detail: string }
	| { kind: 'action'; label: string; detail: string; icon: string; command: string; args?: unknown[] };

export class ProjectTreeProvider implements vscode.TreeDataProvider<Node> {
	private readonly emitter = new vscode.EventEmitter<Node | undefined>();
	readonly onDidChangeTreeData = this.emitter.event;
	private projects: Promise<DiscoveredProject[]> | undefined;

	refresh(): void {
		this.projects = undefined;
		this.emitter.fire(undefined);
	}

	getTreeItem(node: Node): vscode.TreeItem {
		switch (node.kind) {
			case 'project': {
				const item = new vscode.TreeItem(node.project.manifest.name ?? path.basename(node.project.dir), vscode.TreeItemCollapsibleState.Expanded);
				item.iconPath = new vscode.ThemeIcon('package');
				item.description = vscode.workspace.asRelativePath(node.project.manifestUri);
				item.tooltip = node.project.manifestUri.fsPath;
				item.contextValue = 'project';
				item.command = { command: 'vscode.open', title: 'Open manifest', arguments: [node.project.manifestUri] };
				return item;
			}
			case 'section': {
				const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.Expanded);
				item.iconPath = new vscode.ThemeIcon(node.icon);
				item.description = node.detail;
				item.contextValue = `section:${node.id}`;
				return item;
			}
			case 'file': {
				const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.None);
				item.description = node.missing ? 'missing' : node.detail;
				item.resourceUri = node.uri;
				item.iconPath = node.icon ? new vscode.ThemeIcon(node.icon) : vscode.ThemeIcon.File;
				item.tooltip = node.uri.fsPath;
				item.command = { command: 'vscode.open', title: 'Open', arguments: [node.uri] };
				return item;
			}
			case 'profile': {
				const item = new vscode.TreeItem(node.label, node.overlays.length > 0 ? vscode.TreeItemCollapsibleState.Expanded : vscode.TreeItemCollapsibleState.None);
				item.iconPath = new vscode.ThemeIcon('layers');
				item.description = `${node.overlays.length} overlay${node.overlays.length === 1 ? '' : 's'}`;
				return item;
			}
			case 'floor': {
				const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.None);
				item.iconPath = new vscode.ThemeIcon('filter');
				item.description = node.detail;
				item.tooltip = 'Click to change the minimum severity of reported findings';
				item.command = { command: 'suspect.setSeverityFloor', title: 'Set minimum severity' };
				return item;
			}
			case 'action': {
				const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.None);
				item.iconPath = new vscode.ThemeIcon(node.icon);
				item.description = node.detail;
				item.command = { command: node.command, title: node.label, arguments: node.args ?? [] };
				return item;
			}
		}
	}

	async getChildren(node?: Node): Promise<Node[]> {
		if (node === undefined) {
			const projects = await this.load();
			return projects.map((project) => ({ kind: 'project' as const, project }));
		}
		switch (node.kind) {
			case 'project':
				return this.sections(node.project);
			case 'section':
				return this.sectionChildren(node);
			case 'profile':
				return node.overlays.map((rel) => this.file(node.project.dir, rel, 'overlay'));
			default:
				return [];
		}
	}

	private load(): Promise<DiscoveredProject[]> {
		if (this.projects === undefined) {
			this.projects = findProjects().then((projects) => {
				void vscode.commands.executeCommand('setContext', 'suspect.hasProject', projects.length > 0);
				return projects;
			});
		}
		return this.projects;
	}

	private file(dir: string, rel: string, detail?: string, icon?: string): Node {
		const full = path.resolve(dir, rel);
		return { kind: 'file', label: rel, detail, uri: vscode.Uri.file(full), icon, missing: !fs.existsSync(full) };
	}

	private sections(project: DiscoveredProject): Node[] {
		const { manifest } = project;
		const out: Node[] = [];
		const section = (id: string, label: string, icon: string, detail: string): Node => ({ kind: 'section', id, label, icon, detail, project });
		out.push(section('spec', 'Spec', 'file-code', manifest.entry ?? 'no entry declared'));
		if (manifest.profiles.length > 0) {
			out.push(section('profiles', 'Publish profiles', 'layers', manifest.profiles.map((p) => p.name).join(' · ')));
		}
		if (manifest.codegen.length > 0) {
			out.push(section('sdk', 'SDK targets', 'symbol-method', manifest.codegen.map((t) => t.name).join(' · ')));
		}
		out.push(section('tests', 'Contract tests', 'beaker', manifest.tests ? `${manifest.tests.arazzo.length} suite${manifest.tests.arazzo.length === 1 ? '' : 's'}` : 'none declared'));
		out.push(section('config', 'Configuration', 'settings-gear', ''));
		return out;
	}

	private async sectionChildren(node: Extract<Node, { kind: 'section' }>): Promise<Node[]> {
		const { project } = node;
		const { manifest, dir } = project;
		switch (node.id) {
			case 'spec':
				return manifest.entry !== undefined ? [this.file(dir, manifest.entry, 'entry document')] : [];
			case 'profiles':
				return manifest.profiles.map((profile) => ({ kind: 'profile' as const, label: profile.name, overlays: profile.overlays, project }));
			case 'sdk':
				return manifest.codegen.map((target) => {
					const identity = [target.packageName, target.packageVersion].filter(Boolean).join('@');
					return {
						kind: 'action' as const,
						label: target.name,
						detail: [target.profile, identity].filter(Boolean).join(' · '),
						icon: 'symbol-package',
						command: 'vscode.open',
						args: [project.manifestUri],
					};
				});
			case 'tests': {
				if (manifest.tests === undefined) return [];
				const out: Node[] = manifest.tests.arazzo.map((rel) => this.file(dir, rel, 'Arazzo suite'));
				out.push({
					kind: 'action',
					label: 'Run in Testing view',
					detail: manifest.tests.cassette !== undefined ? 'offline · cassette' : manifest.tests.base_url,
					icon: 'beaker',
					command: 'workbench.view.testing.focus',
				});
				return out;
			}
			case 'config': {
				const out: Node[] = [];
				const workspaceConfig = path.join(dir, '.suspect.yaml');
				if (fs.existsSync(workspaceConfig)) {
					out.push(this.file(dir, '.suspect.yaml', 'workspace policy'));
				}
				const floor = await effectiveFloor(dir, manifest.lintMinSeverity);
				out.push({ kind: 'floor', label: 'Minimum severity', detail: `${floor.value} · ${floor.source}` });
				out.push({
					kind: 'action', label: 'Editor settings', detail: 'suspect.*', icon: 'settings',
					command: 'workbench.action.openWorkspaceSettings', args: ['suspect'],
				});
				return out;
			}
			default:
				return [];
		}
	}
}

export function registerProjectView(context: vscode.ExtensionContext): void {
	const provider = new ProjectTreeProvider();
	const tree = vscode.window.createTreeView('suspect.project', { treeDataProvider: provider, showCollapseAll: true });
	context.subscriptions.push(
		tree,
		vscode.commands.registerCommand('suspect.project.refresh', () => provider.refresh()),
		vscode.workspace.onDidSaveTextDocument((doc) => {
			if (affectsProject(doc)) provider.refresh();
		}),
		vscode.workspace.onDidChangeWorkspaceFolders(() => provider.refresh()),
		vscode.workspace.onDidChangeConfiguration((event) => {
			if (event.affectsConfiguration('suspect.lint.minSeverity')) provider.refresh();
		}),
	);
}
