"use strict";
// Configuration management with a UI, instead of hand-editing YAML.
//
// The server already re-derives its severity floor when the client's
// settings change — the same path the regression test
// `a_configuration_change_refilters_diagnostics` pins — so these commands
// are live controls, not "restart and pray": pick a floor, the editor and
// the Problems panel re-filter within the session.
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
exports.currentFloor = currentFloor;
exports.setSeverityFloor = setSeverityFloor;
exports.openSuspectConfig = openSuspectConfig;
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const overview_1 = require("./overview");
const FLOORS = [
    { label: 'Error', detail: 'only errors — the publish gate', value: 'error' },
    { label: 'Warning', detail: 'errors and warnings — the recommended default', value: 'warning' },
    { label: 'Information', detail: 'everything but hints', value: 'information' },
    { label: 'Hint', detail: 'show everything', value: 'hint' },
];
/** The configured floor, falling back to what a committed `.suspect.yaml` says. */
async function currentFloor() {
    const fromSettings = vscode.workspace.getConfiguration('suspect').get('lint.minSeverity');
    if (fromSettings !== undefined && fromSettings !== '') {
        return fromSettings;
    }
    // The file's floor is the server's real default; read it so the pick
    // shows the truth instead of a stale label.
    const root = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (root === undefined)
        return 'hint';
    try {
        const document = await vscode.workspace.openTextDocument(path.join(root, '.suspect.yaml'));
        const match = document.getText().match(/min_severity:\s*(\w+)/);
        return match ? match[1] : 'hint';
    }
    catch {
        return 'hint';
    }
}
async function setSeverityFloor() {
    const current = await currentFloor();
    const picked = await vscode.window.showQuickPick(FLOORS.map((f) => ({ label: f.label, description: f.detail, picked: f.value === current })), { placeHolder: `Findings floor — currently ${current}` });
    if (picked === undefined)
        return;
    const chosen = FLOORS.find((f) => f.label === picked.label);
    if (chosen === undefined)
        return;
    await vscode.workspace
        .getConfiguration('suspect')
        .update('lint.minSeverity', chosen.value, vscode.ConfigurationTarget.Workspace);
    void vscode.window.showInformationMessage(`Suspect floor set to ${chosen.label.toLowerCase()}. Findings re-filter in the editor and the Problems panel.`);
}
/** Opens the detected config surface: the committed files, or the editor settings when there are none. */
async function openSuspectConfig() {
    const root = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (root === undefined) {
        void vscode.window.showWarningMessage('Open a folder first — suspect configuration is per-project.');
        return;
    }
    const files = (0, overview_1.detectProjectFiles)(root);
    const picks = [];
    for (const config of files.configs) {
        picks.push({
            label: config.path,
            description: config.name,
            target: vscode.Uri.file(path.join(root, config.path)),
        });
    }
    picks.push({ label: 'Editor settings', description: 'suspect.* in settings.json', target: 'settings' });
    const picked = await vscode.window.showQuickPick(picks, { placeHolder: 'Open suspect configuration' });
    if (picked === undefined)
        return;
    if (picked.target === 'settings') {
        void vscode.commands.executeCommand('workbench.action.openWorkspaceSettings', 'suspect');
        return;
    }
    void vscode.window.showTextDocument(picked.target);
}
//# sourceMappingURL=config.js.map