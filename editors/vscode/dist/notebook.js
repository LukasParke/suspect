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
exports.registerNotebook = registerNotebook;
const fs = __importStar(require("fs"));
const vscode = __importStar(require("vscode"));
const parse_1 = require("./parse");
const runner_1 = require("./runner");
class SuspectDocument {
    uri;
    constructor(uri) {
        this.uri = uri;
    }
    dispose() {
        // no resources to release
    }
}
/**
 * The custom-editor registrar moved between `workspace` and `window` across
 * vscode API versions; resolve it from whichever namespace exposes it.
 */
function resolveRegisterCustomEditor() {
    const typed = { registerCustomEditorProvider: undefined };
    if ('registerCustomEditorProvider' in vscode.workspace) {
        return vscode.workspace.registerCustomEditorProvider;
    }
    return vscode.window.registerCustomEditorProvider;
}
function registerNotebook(context) {
    context.subscriptions.push(resolveRegisterCustomEditor()('suspect.notebook', new SuspectNotebookProvider(), { supportsMultipleEditorsPerDocument: false, webviewOptions: { retainContextWhenHidden: true } }));
}
/**
 * Read-only notebook projection of an *.arazzo.yaml file.
 *
 * The webview never edits the document: it renders the workflow/step cells
 * parsed from the YAML source and runs them through the suspect CLI. All
 * edits happen in the underlying text document; the projection re-renders
 * when that document is saved.
 */
class SuspectNotebookProvider {
    activeRuns = new Set();
    livePanels = new Set();
    changeEmitter = new vscode.EventEmitter();
    onDidChangeCustomDocument = this.changeEmitter.event;
    cancelRequested = false;
    openCustomDocument(uri) {
        return new SuspectDocument(uri);
    }
    resolveCustomEditor(document, panel) {
        panel.webview.options = { enableScripts: true, localResourceRoots: [] };
        this.livePanels.add(panel);
        let lastModelJson = '';
        const rerenderIfChanged = async () => {
            const modelJson = await readModelJson(document.uri);
            if (modelJson === undefined || modelJson === lastModelJson) {
                return;
            }
            lastModelJson = modelJson;
            panel.webview.html = renderHtml(modelJson);
        };
        panel.webview.onDidReceiveMessage((msg) => {
            void this.handleMessage(msg, panel, document.uri);
        });
        const disposables = [
            vscode.workspace.onDidSaveTextDocument((doc) => {
                if (doc.uri.toString() === document.uri.toString()) {
                    void rerenderIfChanged();
                }
            }),
            panel.onDidDispose(() => {
                this.livePanels.delete(panel);
                this.cancelActiveRuns();
                disposables.forEach((d) => d.dispose());
            }),
        ];
        void rerenderIfChanged();
    }
    saveCustomDocument() {
        // the webview never edits; nothing to persist beyond the source file
        return Promise.resolve();
    }
    saveCustomDocumentAs(document, destination) {
        return fs.promises.copyFile(document.uri.fsPath, destination.fsPath);
    }
    revertCustomDocument() {
        return Promise.resolve();
    }
    backupCustomDocument(_document, context) {
        return Promise.resolve({
            id: context.destination.toString(),
            delete: async () => {
                await fs.promises.rm(context.destination.fsPath, { force: true });
            },
        });
    }
    cancelActiveRuns() {
        for (const run of this.activeRuns) {
            run.kill();
        }
        this.activeRuns.clear();
    }
    async handleMessage(msg, panel, uri) {
        if (typeof msg !== 'object' || msg === null || panel === undefined) {
            return;
        }
        const { type, wf } = msg;
        switch (type) {
            case 'runStep':
                if (wf !== undefined) {
                    await this.runWorkflow(panel, uri, wf);
                }
                return;
            case 'runAll': {
                this.cancelRequested = false;
                const workflows = (0, parse_1.parseArazzo)(await fs.promises.readFile(uri.fsPath, 'utf8'));
                for (const workflow of workflows) {
                    if (this.cancelRequested) {
                        return;
                    }
                    await this.runWorkflow(panel, uri, workflow.workflowId);
                }
                return;
            }
            case 'cancel':
                this.cancelRequested = true;
                this.cancelActiveRuns();
                return;
            default:
                return;
        }
    }
    async runWorkflow(panel, uri, workflowId) {
        const post = (message) => {
            if (this.livePanels.has(panel)) {
                void panel.webview.postMessage(message);
            }
        };
        const setStep = (stepId, state, detail) => {
            post({ type: 'result', wf: workflowId, step: stepId, state, detail });
        };
        const running = new Set();
        const handle = (0, runner_1.spawnSuspectRun)(uri.fsPath, workflowId, (ev) => {
            switch (ev.event) {
                case 'wf_started':
                    return;
                case 'step_started':
                    running.add(ev.step);
                    setStep(ev.step, 'running');
                    return;
                case 'response_got':
                    setStep(ev.step, 'running', `HTTP ${ev.status} · ${ev.duration_ms} ms`);
                    return;
                case 'criterion_fail':
                    running.delete(ev.step);
                    setStep(ev.step, 'failed', `${ev.crit} — expected: ${ev.expected}, actual: ${ev.actual}`);
                    return;
                case 'wf_done': {
                    for (const stepId of running) {
                        setStep(stepId, ev.passed ? 'passed' : 'failed');
                    }
                    running.clear();
                    post({ type: 'runFinished', wf: workflowId, passed: ev.passed ? 1 : 0, failed: ev.passed ? 0 : 1 });
                    return;
                }
                default:
                    return;
            }
        });
        this.activeRuns.add(handle);
        try {
            const totals = await handle.done;
            post({ type: 'runFinished', wf: workflowId, passed: totals.passed, failed: totals.failed });
        }
        catch (err) {
            vscode.window.showErrorMessage(`Suspect run failed for ${workflowId}: ${(0, runner_1.errorMessage)(err)}`);
        }
        finally {
            this.activeRuns.delete(handle);
        }
    }
}
async function readModelJson(uri) {
    try {
        const workflows = (0, parse_1.parseArazzo)(await fs.promises.readFile(uri.fsPath, 'utf8'));
        return JSON.stringify(workflows.map((wf) => ({ id: wf.workflowId, steps: wf.steps.map((s) => s.stepId) })));
    }
    catch {
        return undefined;
    }
}
function renderHtml(modelJson) {
    const nonce = Array.from(crypto.getRandomValues(new Uint8Array(16)))
        .map((b) => b.toString(16).padStart(2, '0'))
        .join('');
    const safeModel = modelJson.replace(/</g, '\\u003c');
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-${nonce}';">
<style>
  body { font-family: var(--vscode-font-family); color: var(--vscode-foreground); padding: 8px 16px; }
  header { display: flex; align-items: center; gap: 12px; border-bottom: 1px solid var(--vscode-panel-border); padding-bottom: 8px; margin-bottom: 8px; flex-wrap: wrap; }
  header .note { opacity: 0.75; font-size: 0.9em; }
  h2 { font-size: 1.05em; margin: 14px 0 6px; color: var(--vscode-symbol-class-color); }
  h2 span { opacity: 0.75; font-size: 0.85em; margin-left: 10px; }
  .cell { display: flex; align-items: baseline; gap: 10px; padding: 4px 8px; border-radius: 4px; flex-wrap: wrap; }
  .cell:hover { background: var(--vscode-list-hoverBackground); }
  .cell .title { min-width: 160px; font-family: var(--vscode-editor-font-family); }
  button { cursor: pointer; background: var(--vscode-button-background); color: var(--vscode-button-foreground); border: none; border-radius: 2px; padding: 2px 10px; }
  button.secondary { background: var(--vscode-button-secondaryBackground); color: var(--vscode-button-secondaryForeground); }
  .badge { font-size: 0.85em; padding: 1px 8px; border-radius: 8px; }
  .badge.pending { opacity: 0.5; }
  .badge.running { background: var(--vscode-editorWarning-foreground, #cb0); color: var(--vscode-editor-background); }
  .badge.passed { background: rgba(46, 160, 67, 0.45); }
  .badge.failed { background: var(--vscode-errorForeground, #f44); color: var(--vscode-editor-background); }
  .detail { opacity: 0.8; font-size: 0.85em; white-space: pre-wrap; }
</style>
</head>
<body>
<header>
  <button id="runAll">Run all</button>
  <button id="cancel" class="secondary">Cancel</button>
  <span class="note">Read-only projection — edit the YAML source file to change these workflows; this notebook runs them.</span>
</header>
<div id="cells"></div>
<script nonce="${nonce}">
const MODEL = ${safeModel};
const vscode = acquireVsCodeApi();
const cellsRoot = document.getElementById('cells');
for (const wf of MODEL) {
  const heading = document.createElement('h2');
  heading.textContent = wf.id;
  const summary = document.createElement('span');
  heading.appendChild(summary);
  cellsRoot.appendChild(heading);
  for (const stepId of wf.steps) {
    const cell = document.createElement('div');
    cell.className = 'cell';
    cell.dataset.wf = wf.id;
    cell.dataset.step = stepId;
    const title = document.createElement('span');
    title.className = 'title';
    title.textContent = stepId;
    const badge = document.createElement('span');
    badge.className = 'badge pending';
    badge.textContent = 'idle';
    const detail = document.createElement('span');
    detail.className = 'detail';
    const runBtn = document.createElement('button');
    runBtn.textContent = 'Run';
    runBtn.addEventListener('click', () => {
      badge.className = 'badge pending'; badge.textContent = 'queued';
      detail.textContent = '';
      vscode.postMessage({ type: 'runStep', wf: wf.id, step: stepId });
    });
    cell.append(title, badge, detail, runBtn);
    cellsRoot.appendChild(cell);
  }
}
document.getElementById('runAll').addEventListener('click', () => vscode.postMessage({ type: 'runAll' }));
document.getElementById('cancel').addEventListener('click', () => vscode.postMessage({ type: 'cancel' }));
window.addEventListener('message', (event) => {
  const m = event.data;
  if (m.type === 'result') {
    const cell = document.querySelector('.cell[data-wf="' + cssEscape(m.wf) + '"][data-step="' + cssEscape(m.step) + '"]');
    if (!cell) { return; }
    const badge = cell.querySelector('.badge');
    badge.className = 'badge ' + m.state;
    badge.textContent = m.state;
    cell.querySelector('.detail').textContent = m.detail || '';
  } else if (m.type === 'runFinished') {
    const heading = Array.from(cellsRoot.querySelectorAll('h2')).find((h) => h.firstChild && h.firstChild.textContent === m.wf);
    if (heading && heading.querySelector('span')) {
      heading.querySelector('span').textContent = '— last run: ' + (m.passed + m.failed) + ' step(s), ' + m.failed + ' failed';
    }
  }
});
function cssEscape(value) {
  return window.CSS && CSS.escape ? CSS.escape(value) : value.replace(/["\\\\]/g, '\\\\$&');
}
</script>
</body>
</html>`;
}
//# sourceMappingURL=notebook.js.map