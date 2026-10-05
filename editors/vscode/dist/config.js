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
exports.resolveFloor = resolveFloor;
exports.effectiveFloor = effectiveFloor;
exports.setSeverityFloor = setSeverityFloor;
exports.openSuspectConfig = openSuspectConfig;
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const FLOORS = [
    { label: 'Error', detail: 'only errors — the publish gate', value: 'error' },
    { label: 'Warning', detail: 'errors and warnings — the recommended default', value: 'warning' },
    { label: 'Information', detail: 'everything but hints', value: 'information' },
    { label: 'Hint', detail: 'show everything', value: 'hint' },
];
/**
 * The floor in effect, by precedence: the editor setting, the committed
 * `.suspect.yaml`, the project manifest, then the server default. Pure.
 */
function resolveFloor(input) {
    if (input.setting)
        return { value: input.setting, source: 'editor setting' };
    const fromYaml = input.workspaceYaml?.match(/^\s*min_severity:\s*(\w+)/m)?.[1];
    if (fromYaml)
        return { value: fromYaml, source: '.suspect.yaml' };
    if (input.manifest)
        return { value: input.manifest, source: 'suspect.project.json' };
    return { value: 'hint', source: 'default' };
}
/** `resolveFloor` over the live workspace. */
async function effectiveFloor(root, manifestFloor) {
    const setting = vscode.workspace.getConfiguration('suspect').get('lint.minSeverity');
    let workspaceYaml;
    if (root !== undefined) {
        try {
            workspaceYaml = await fs.promises.readFile(path.join(root, '.suspect.yaml'), 'utf8');
        }
        catch {
            workspaceYaml = undefined;
        }
    }
    return resolveFloor({ setting, workspaceYaml, manifest: manifestFloor });
}
function workspaceRoot() {
    return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}
async function setSeverityFloor() {
    const current = await effectiveFloor(workspaceRoot());
    const picked = await vscode.window.showQuickPick(FLOORS.map((f) => ({ label: f.label, description: f.detail, picked: f.value === current.value })), { placeHolder: `Minimum severity of reported findings — currently ${current.value} (${current.source})` });
    if (picked === undefined)
        return;
    const chosen = FLOORS.find((f) => f.label === picked.label);
    if (chosen === undefined)
        return;
    await vscode.workspace
        .getConfiguration('suspect')
        .update('lint.minSeverity', chosen.value, vscode.ConfigurationTarget.Workspace);
    void vscode.window.showInformationMessage(`Suspect now reports ${chosen.label.toLowerCase()} and above. Findings re-filter in the editor and the Problems panel.`);
}
/** Opens the configuration surface: the committed files, or the editor settings. */
async function openSuspectConfig() {
    const root = workspaceRoot();
    if (root === undefined) {
        void vscode.window.showWarningMessage('Open a folder first — suspect configuration is per-project.');
        return;
    }
    const picks = [];
    for (const [file, description] of [['.suspect.yaml', 'workspace policy'], ['suspect.project.json', 'project manifest']]) {
        const full = path.join(root, file);
        if (fs.existsSync(full))
            picks.push({ label: file, description, target: vscode.Uri.file(full) });
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