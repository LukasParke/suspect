"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.WorkflowsTreeProvider = void 0;
exports.registerWorkflowsView = registerWorkflowsView;
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const parse_1 = require("./parse");
class WorkflowsTreeProvider {
    cache = new Map();
    emitter = new vscode.EventEmitter();
    onDidChangeTreeData = this.emitter.event;
    refresh() {
        this.cache.clear();
        this.emitter.fire(undefined);
    }
    getTreeItem(node) {
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
    async getChildren(element) {
        if (element) {
            switch (element.kind) {
                case 'file':
                    return element.workflows.map((wf) => ({
                        kind: 'workflow',
                        uri: element.uri,
                        workflowId: wf.workflowId,
                    }));
                case 'workflow': {
                    const steps = this.cache.get(element.uri.fsPath)?.find((wf) => wf.workflowId === element.workflowId)?.steps ?? [];
                    return steps.map((step) => ({
                        kind: 'step',
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
        const nodes = [];
        for (const uri of uris.sort((a, b) => a.fsPath.localeCompare(b.fsPath))) {
            try {
                const workflows = (0, parse_1.parseArazzo)(await fs.promises.readFile(uri.fsPath, 'utf8'));
                if (workflows.length === 0) {
                    continue;
                }
                this.cache.set(uri.fsPath, workflows);
                nodes.push({ kind: 'file', uri, workflows });
            }
            catch {
                // unreadable between findFiles and read — skip
            }
        }
        return nodes;
    }
}
exports.WorkflowsTreeProvider = WorkflowsTreeProvider;
function registerWorkflowsView(context) {
    const provider = new WorkflowsTreeProvider();
    const tree = vscode.window.createTreeView('suspect.workflows', { treeDataProvider: provider });
    context.subscriptions.push(tree, vscode.commands.registerCommand('suspect.workflows.refresh', () => provider.refresh()), vscode.workspace.onDidSaveTextDocument((doc) => {
        if (/\.arazzo\.ya?ml$/i.test(doc.uri.fsPath)) {
            provider.refresh();
        }
    }), vscode.workspace.onDidChangeWorkspaceFolders(() => provider.refresh()));
    void vscode.commands.executeCommand('suspect.workflows.refresh');
}
//# sourceMappingURL=workflowsView.js.map