import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { parseArazzo, ParsedWorkflow } from './parse';

type Node =
	| { kind: 'file'; uri: vscode.Uri; workflows: ParsedWorkflow[] }
	| { kind: 'workflow'; uri: vscode.Uri; workflowId: string }
	| { kind: 'step'; uri: vscode.Uri; workflowId: string; stepId: string };

export class WorkflowsTreeProvider implements vscode.TreeDataProvider<Node> {
	private readonly cache = new Map<string, ParsedWorkflow[]>();
	private readonly emitter = new vscode.EventEmitter<Node | undefined>();
	readonly onDidChangeTreeData = this.emitter.event;

	refresh(): void {
		this.cache.clear();
		this.emitter.fire(undefined);
	}

	getTreeItem(node: Node): vscode.TreeItem {
		switch (node.kind) {
			case 'file': {
				const item = new vscode.TreeItem(path.basename(node.uri.fsPath), vscode.TreeItemCollapsibleState.Collapsed);
				item.description = vscode.workspace.asRelativePath(node.uri, false);
				item.contextValue = 'suspectFile';
				item.tooltip = `${node.workflows.length} workflow(s)`;
				return item;
			}
			case 'workflow': {
				const item = new vscode.TreeItem(node.workflowId, vscode.TreeItemCollapsibleState.Collapsed);
				item.contextValue = 'suspectWorkflow';
				item.tooltip = `Run workflow ${node.workflowId}`;
				item.command = {
					command: 'suspect.runWorkflow',
					title: 'Run Workflow',
					arguments: [node.uri, node.workflowId],
				};
				return item;
			}
			case 'step':
				return new vscode.TreeItem(node.stepId, vscode.TreeItemCollapsibleState.None);
		}
	}

	async getChildren(element?: Node): Promise<Node[]> {
		if (element) {
			switch (element.kind) {
				case 'file':
					return element.workflows.map((wf) => ({
						kind: 'workflow' as const,
						uri: element.uri,
						workflowId: wf.workflowId,
					}));
				case 'workflow': {
					const steps = this.cache.get(element.uri.fsPath)?.find((wf) => wf.workflowId === element.workflowId)?.steps ?? [];
					return steps.map((step) => ({
						kind: 'step' as const,
						uri: element.uri,
						workflowId: element.workflowId,
						stepId: step.stepId,
					}));
				}
				case 'step':
					return [];
			}
		}
		const uris = await vscode.workspace.findFiles('**/*.arazzo.{yaml,yml}', '**/node_modules/**');
		const nodes: Node[] = [];
		for (const uri of uris.sort((a, b) => a.fsPath.localeCompare(b.fsPath))) {
			try {
				const workflows = parseArazzo(await fs.promises.readFile(uri.fsPath, 'utf8'));
				if (workflows.length === 0) {
					continue;
				}
				this.cache.set(uri.fsPath, workflows);
				nodes.push({ kind: 'file', uri, workflows });
			} catch {
				// unreadable between findFiles and read — skip
			}
		}
		return nodes;
	}
}

export function registerWorkflowsView(context: vscode.ExtensionContext): void {
	const provider = new WorkflowsTreeProvider();
	const tree = vscode.window.createTreeView('suspect.workflows', { treeDataProvider: provider });
	context.subscriptions.push(
		tree,
		vscode.commands.registerCommand('suspect.workflows.refresh', () => provider.refresh()),
		vscode.workspace.onDidSaveTextDocument((doc) => {
			if (/\.arazzo\.ya?ml$/i.test(doc.uri.fsPath)) {
				provider.refresh();
			}
		}),
		vscode.workspace.onDidChangeWorkspaceFolders(() => provider.refresh()),
	);
	void vscode.commands.executeCommand('suspect.workflows.refresh');
}
