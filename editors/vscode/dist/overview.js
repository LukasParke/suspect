"use strict";
// The Suspect Overview: the project's spec tooling in one tree.
//
// The activity-bar container used to hold only the workflows list. The
// Overview adds what the improved LSP makes possible — findings by
// severity from the published diagnostics, the detected configuration
// surface, and the runtime state (gateway, SDK session) — so the
// extension's own functionality is manageable from a native view instead
// of scattered commands.
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
exports.OverviewProvider = void 0;
exports.detectProjectFiles = detectProjectFiles;
exports.registerOverview = registerOverview;
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const status_1 = require("./status");
function detectProjectFiles(root) {
    const exists = (rel) => fs.existsSync(path.join(root, rel));
    const readDir = (rel) => {
        try {
            return fs.readdirSync(path.join(root, rel))
                .filter((f) => /\.(yaml|yml|json)$/.test(f))
                .map((f) => `${rel}/${f}`);
        }
        catch {
            return [];
        }
    };
    const configs = [];
    if (exists('.suspect.yaml'))
        configs.push({ name: 'Workspace settings', path: '.suspect.yaml' });
    if (exists('suspect.project.json'))
        configs.push({ name: 'Project manifest', path: 'suspect.project.json' });
    return {
        entry: ['openapi.yaml', 'spec.yaml', 'main.yaml'].find(exists),
        configs,
        overlays: readDir('overlays'),
        workflows: readDir('workflows'),
    };
}
class OverviewProvider {
    emitter = new vscode.EventEmitter();
    onDidChangeTreeData = this.emitter.event;
    refresh() {
        this.emitter.fire(undefined);
    }
    getChildren(node) {
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
    getTreeItem(node) {
        switch (node.kind) {
            case 'section': {
                const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.Expanded);
                item.iconPath = new vscode.ThemeIcon(node.icon);
                item.description = node.state;
                item.contextValue = `section:${node.id}`;
                return item;
            }
            case 'finding': {
                const order = status_1.SEVERITY_ORDER.find((s) => s.key === node.severity);
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
    sections() {
        const counts = (0, status_1.updateStatus)();
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
    findings() {
        const counts = (0, status_1.updateStatus)();
        return status_1.SEVERITY_ORDER
            .map(({ key, label, icon }) => ({ key, label, icon, count: counts[key] }))
            .filter((s) => s.count > 0)
            .map((s) => ({ kind: 'finding', severity: s.key, label: `${s.label} (${s.count})` }));
    }
    project() {
        const root = this.root();
        const files = detectProjectFiles(root);
        const out = [];
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
    runtime() {
        const files = detectProjectFiles(this.root());
        const out = [
            { kind: 'runtime', id: 'gateway', label: 'Mock gateway', detail: 'start/stop', command: '_suspect.toggleGateway' },
            { kind: 'runtime', id: 'session', label: 'SDK session', detail: 'preview/watch', command: 'suspect.previewSdk' },
            { kind: 'runtime', id: 'workflows', label: 'Workflows', detail: 'run', command: 'suspect.runWorkflow' },
        ];
        if (files.workflows.length > 0) {
            out.push({ kind: 'file', uri: vscode.Uri.file(path.join(this.root(), files.workflows[0])), label: path.basename(files.workflows[0]), detail: 'first workflow' });
        }
        return out;
    }
    root() {
        return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? '';
    }
}
exports.OverviewProvider = OverviewProvider;
function registerOverview(context) {
    const provider = new OverviewProvider();
    context.subscriptions.push(vscode.window.registerTreeDataProvider('suspect.overview', provider));
    context.subscriptions.push(vscode.languages.onDidChangeDiagnostics(() => provider.refresh()));
}
//# sourceMappingURL=overview.js.map