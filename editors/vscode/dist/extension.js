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
exports.activate = activate;
exports.deactivate = deactivate;
const cp = __importStar(require("child_process"));
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const node_1 = require("vscode-languageclient/node");
const parse_1 = require("./parse");
const notebook_1 = require("./notebook");
const runner_1 = require("./runner");
const testExplorer_1 = require("./testExplorer");
const workflowsView_1 = require("./workflowsView");
let client;
let gateway;
let gatewayStatus;
function activate(context) {
    context.subscriptions.push(vscode.commands.registerCommand('suspect.runWorkflow', (uri, workflow) => runWorkflowCommand(uri, workflow)), vscode.commands.registerCommand('suspect.startGateway', () => startGateway()), vscode.commands.registerCommand('suspect.stopGateway', () => stopGateway()), vscode.commands.registerCommand('_suspect.toggleGateway', () => (gateway ? stopGateway() : void startGateway())), vscode.commands.registerCommand('suspect.genPreset', () => genPresetCommand()), vscode.commands.registerCommand('suspect.openNotebook', (uri) => openNotebook(uri)));
    gatewayStatus = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 90);
    gatewayStatus.command = '_suspect.toggleGateway';
    gatewayStatus.name = 'Suspect Gateway';
    context.subscriptions.push(gatewayStatus);
    (0, testExplorer_1.registerTestExplorer)(context);
    (0, workflowsView_1.registerWorkflowsView)(context);
    (0, notebook_1.registerNotebook)(context);
    startClient();
    context.subscriptions.push(vscode.workspace.onDidChangeConfiguration((event) => {
        if (event.affectsConfiguration('suspect.basePath')) {
            void restartClient();
        }
    }));
}
async function deactivate() {
    const tasks = [];
    if (client !== undefined) {
        tasks.push(Promise.resolve(client.stop()).catch(() => undefined));
    }
    stopGateway();
    await Promise.all(tasks);
}
function startClient() {
    const serverOptions = {
        command: (0, runner_1.suspectBinary)(),
        args: ['lsp'],
        transport: node_1.TransportKind.stdio,
    };
    const clientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'yaml' },
            { scheme: 'file', language: 'json' },
        ],
    };
    client = new node_1.LanguageClient('suspect', 'Suspect', serverOptions, clientOptions);
    void Promise.resolve(client.start()).catch((err) => {
        vscode.window.showWarningMessage(`Suspect language server failed to start: ${(0, runner_1.errorMessage)(err)}`);
        client = undefined;
    });
}
async function restartClient() {
    if (client !== undefined) {
        const current = client;
        client = undefined;
        await Promise.resolve(current.stop()).catch(() => undefined);
    }
    startClient();
}
async function pickArazzoDocument(hint) {
    if (hint) {
        return hint;
    }
    const active = vscode.window.activeTextEditor?.document;
    if (active && /\.arazzo\.ya?ml$/i.test(active.fileName)) {
        return active.uri;
    }
    const uris = await vscode.workspace.findFiles('**/*.arazzo.{yaml,yml}', '**/node_modules/**');
    if (uris.length === 0) {
        vscode.window.showErrorMessage('No *.arazzo.yaml documents found in the workspace.');
        return undefined;
    }
    if (uris.length === 1) {
        return uris[0];
    }
    const pick = await vscode.window.showQuickPick(uris.map((uri) => ({ label: vscode.workspace.asRelativePath(uri), uri })), { placeHolder: 'Select an Arazzo document' });
    return pick?.uri;
}
async function pickWorkflowId(uri) {
    let workflows;
    try {
        workflows = (0, parse_1.parseArazzo)(await fs.promises.readFile(uri.fsPath, 'utf8'));
    }
    catch (err) {
        vscode.window.showErrorMessage(`Could not read ${vscode.workspace.asRelativePath(uri)}: ${(0, runner_1.errorMessage)(err)}`);
        return undefined;
    }
    if (workflows.length === 0) {
        vscode.window.showErrorMessage(`No workflows found in ${vscode.workspace.asRelativePath(uri)}.`);
        return undefined;
    }
    if (workflows.length === 1) {
        return workflows[0].workflowId;
    }
    const pick = await vscode.window.showQuickPick(workflows.map((wf) => ({ label: wf.workflowId, description: `${wf.steps.length} step(s)` })), { placeHolder: 'Select a workflow to run' });
    return pick?.label;
}
async function runWorkflowCommand(uriHint, workflowHint) {
    const uri = await pickArazzoDocument(uriHint);
    if (!uri) {
        return;
    }
    const workflowId = workflowHint ?? (await pickWorkflowId(uri));
    if (!workflowId) {
        return;
    }
    let startedSteps = 0;
    let finishedSteps = 0;
    await vscode.window.withProgress({
        location: vscode.ProgressLocation.Notification,
        title: `Suspect: running '${workflowId}'`,
        cancellable: true,
    }, async (progress, token) => {
        progress.report({ message: 'starting…' });
        const handle = (0, runner_1.spawnSuspectRun)(uri.fsPath, workflowId, (event) => {
            switch (event.event) {
                case 'step_started':
                    startedSteps += 1;
                    break;
                case 'response_got':
                case 'criterion_fail':
                    finishedSteps += 1;
                    break;
                default:
                    break;
            }
            progress.report({ message: `${Math.max(startedSteps - finishedSteps, 0)} running · ${finishedSteps} finished` });
        });
        token.onCancellationRequested(() => handle.kill());
        try {
            const totals = await handle.done;
            vscode.window.showInformationMessage(`Suspect run '${workflowId}': ${totals.passed} passed, ${totals.failed} failed.`);
        }
        catch (err) {
            vscode.window.showErrorMessage(`Suspect run failed: ${(0, runner_1.errorMessage)(err)}`);
        }
    });
}
async function pickOpenApiSpec() {
    const candidates = new Map();
    for (const pattern of ['**/*.openapi.{yaml,yml,json}', '**/openapi*.{yaml,yml,json}', '**/swagger*.{yaml,yml,json}']) {
        for (const uri of await vscode.workspace.findFiles(pattern, '**/node_modules/**')) {
            candidates.set(uri.fsPath, uri);
        }
    }
    const activeUri = vscode.window.activeTextEditor?.document.uri;
    if (activeUri && !/\.arazzo\.ya?ml$/i.test(activeUri.fsPath) && fs.existsSync(activeUri.fsPath)) {
        candidates.set(activeUri.fsPath, activeUri);
    }
    if (candidates.size === 0) {
        vscode.window.showErrorMessage('No OpenAPI spec (*.openapi.yaml / openapi.json / swagger.*) found.');
        return undefined;
    }
    if (candidates.size === 1) {
        return [...candidates.values()][0];
    }
    const pick = await vscode.window.showQuickPick([...candidates.values()].map((uri) => ({ label: vscode.workspace.asRelativePath(uri), uri })), { placeHolder: 'Select an OpenAPI spec' });
    return pick?.uri;
}
async function startGateway() {
    if (gateway !== undefined) {
        vscode.window.showInformationMessage(`Suspect gateway already running on port ${gateway.port}.`);
        return;
    }
    const spec = await pickOpenApiSpec();
    if (!spec) {
        return;
    }
    const port = (0, runner_1.gatewayPort)();
    const specLabel = vscode.workspace.asRelativePath(spec);
    let child;
    try {
        child = cp.spawn((0, runner_1.suspectBinary)(), ['gateway', spec.fsPath, '--port', String(port), '--mode', 'mock'], {
            stdio: 'ignore',
        });
    }
    catch (err) {
        vscode.window.showErrorMessage(`Failed to launch suspect gateway: ${(0, runner_1.errorMessage)(err)}`);
        return;
    }
    gateway = { child, port, specLabel };
    child.on('error', (err) => {
        vscode.window.showErrorMessage(`Suspect gateway error: ${(0, runner_1.errorMessage)(err)}`);
        if (gateway?.child === child) {
            gateway = undefined;
            updateGatewayStatus();
        }
    });
    child.on('exit', () => {
        if (gateway?.child === child) {
            gateway = undefined;
            updateGatewayStatus();
        }
    });
    updateGatewayStatus();
    vscode.window.showInformationMessage(`Suspect gateway mocking ${specLabel} on http://127.0.0.1:${port}.`);
}
function stopGateway() {
    if (gateway === undefined) {
        vscode.window.showInformationMessage('Suspect gateway is not running.');
        return;
    }
    const state = gateway;
    gateway = undefined;
    state.child.kill('SIGTERM');
    updateGatewayStatus();
}
function updateGatewayStatus() {
    if (gatewayStatus === undefined) {
        return;
    }
    if (gateway === undefined) {
        gatewayStatus.text = '$(circle-slash) suspect';
        gatewayStatus.tooltip = 'Suspect: start mock gateway';
        gatewayStatus.hide();
        return;
    }
    gatewayStatus.text = `$(broadcast) suspect:${gateway.port}`;
    gatewayStatus.tooltip = `Suspect gateway mocking ${gateway.specLabel} on http://127.0.0.1:${gateway.port} — click to stop`;
    gatewayStatus.show();
}
async function genPresetCommand() {
    const presetPick = await vscode.window.showQuickPick(['docs-md', 'ts-sdk', 'rust-sdk'], {
        placeHolder: 'Suspect: choose a generation preset',
    });
    if (!presetPick) {
        return;
    }
    const spec = await pickOpenApiSpec();
    if (!spec) {
        return;
    }
    const folder = vscode.workspace.getWorkspaceFolder(spec) ??
        vscode.workspace.workspaceFolders?.[0];
    if (!folder) {
        vscode.window.showErrorMessage('No workspace folder is open.');
        return;
    }
    const outDir = path.join(folder.uri.fsPath, 'gen-out', presetPick);
    try {
        await vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title: `Suspect gen: ${presetPick}` }, () => new Promise((resolve, reject) => {
            const child = cp.spawn((0, runner_1.suspectBinary)(), ['gen', spec.fsPath, '--preset', presetPick, '--out', outDir], { stdio: ['ignore', 'ignore', 'pipe'] });
            let stderrTail = '';
            child.stderr?.on('data', (chunk) => {
                stderrTail = (stderrTail + chunk.toString()).slice(-2000);
            });
            child.on('error', (err) => reject(err));
            child.on('exit', (code) => {
                if (code === 0) {
                    resolve();
                    return;
                }
                reject(new Error(`suspect gen exited with code ${code}${stderrTail.trim() ? `: ${stderrTail.trim()}` : ''}`));
            });
        }));
    }
    catch (err) {
        vscode.window.showErrorMessage(`Suspect gen failed: ${(0, runner_1.errorMessage)(err)}`);
        return;
    }
    const produced = await firstFileRecursive(outDir);
    if (!produced) {
        vscode.window.showWarningMessage(`suspect gen produced no files under gen-out/${presetPick}.`);
        return;
    }
    const doc = await vscode.workspace.openTextDocument(produced);
    await vscode.window.showTextDocument(doc);
}
async function firstFileRecursive(dir) {
    let entries;
    try {
        entries = await fs.promises.readdir(dir, { withFileTypes: true });
    }
    catch {
        return undefined;
    }
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
        const full = path.join(dir, entry.name);
        if (entry.isFile()) {
            return full;
        }
        if (entry.isDirectory()) {
            const nested = await firstFileRecursive(full);
            if (nested) {
                return nested;
            }
        }
    }
    return undefined;
}
async function openNotebook(uriHint) {
    const uri = await pickArazzoDocument(uriHint);
    if (!uri) {
        return;
    }
    await vscode.commands.executeCommand('vscode.openWith', uri, 'suspect.notebook');
}
//# sourceMappingURL=extension.js.map