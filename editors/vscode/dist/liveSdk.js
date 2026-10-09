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
exports.registerLiveSdkPreview = registerLiveSdkPreview;
/**
 * Live SDK preview: renders a native SDK from the current buffer through
 * the server's `suspect/renderSdk` request, and re-renders on every
 * document change (debounced) — the server's incremental reparse keeps
 * its tree current on every keystroke, so each refresh costs the compile,
 * not the keystrokes that preceded it.
 */
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const generation_1 = require("./generation");
const runner_1 = require("./runner");
const LIVE_SDK_SCHEME = 'suspect-live-sdk';
const REFRESH_DEBOUNCE_MS = 400;
/** One open live preview. Refreshes render through the shared provider. */
class LiveSdkPreview {
    getClient;
    emitter;
    context;
    onDidChange;
    artifacts = new Map();
    watcher;
    pending;
    lastOpened;
    constructor(getClient, emitter, context) {
        this.getClient = getClient;
        this.emitter = emitter;
        this.context = context;
        this.onDidChange = emitter.event;
    }
    provideTextDocumentContent(uri) {
        return this.artifacts.get(uri.path) ?? `No artifact at ${uri.path}. Use Suspect: Live SDK Preview.`;
    }
    /** One render pass; artifacts land in the provider, open documents refresh. */
    async refresh() {
        const client = this.getClient();
        if (!client)
            return;
        try {
            const result = await client.sendRequest('suspect/renderSdk', {
                uri: this.context.uri.toString(),
                profile: this.context.profile,
                packageName: this.context.packageName,
                packageVersion: this.context.packageVersion,
                importName: this.context.importName,
                operationIds: this.context.operationIds,
            });
            if (!result.rendered) {
                const first = result.diagnostics[0]?.message ?? 'the contract refused to plan';
                void vscode.window.showWarningMessage(`Suspect live SDK: ${first}`, 'Show All').then((choice) => {
                    if (choice === 'Show All') {
                        const output = vscode.window.createOutputChannel('Suspect Live SDK');
                        output.appendLine(result.diagnostics.map((d) => `${d.code}: ${d.message}`).join('\n'));
                        output.show(true);
                    }
                });
                return;
            }
            const previous = new Map(this.artifacts);
            this.artifacts.clear();
            for (const artifact of result.artifacts) {
                this.artifacts.set(`/${artifact.path}`, artifact.content);
            }
            // Refresh every open preview tab whose content changed.
            for (const open of this.artifacts) {
                if (previous.get(open[0]) !== open[1]) {
                    this.emitter.fire(this.uriFor(open[0].slice(1)));
                }
            }
            // First render: open the README (or first artifact).
            if (!this.lastOpened) {
                const lead = result.artifacts.find((a) => path.basename(a.path).toLowerCase() === 'readme.md') ??
                    result.artifacts[0];
                if (lead) {
                    this.lastOpened = `/${lead.path}`;
                    void vscode.window.showTextDocument(this.uriFor(lead.path), { preview: true });
                }
            }
            void vscode.window.setStatusBarMessage(`Suspect live SDK: ${result.operations} operations · ${result.artifacts.length} artifacts`, 4000);
        }
        catch (error) {
            void vscode.window.showWarningMessage(`Suspect live SDK: ${(0, runner_1.errorMessage)(error)}`);
        }
    }
    uriFor(artifactPath) {
        return vscode.Uri.from({ scheme: LIVE_SDK_SCHEME, path: `/${artifactPath}` });
    }
    /** Follows the source document: every change re-renders, debounced. */
    start() {
        this.watcher?.dispose();
        this.watcher = vscode.workspace.onDidChangeTextDocument((event) => {
            if (event.document.uri.toString() !== this.context.uri.toString())
                return;
            if (this.pending)
                clearTimeout(this.pending);
            this.pending = setTimeout(() => {
                this.pending = undefined;
                void this.refresh();
            }, REFRESH_DEBOUNCE_MS);
            this.pending.unref?.();
        });
        void this.refresh();
    }
    dispose() {
        this.watcher?.dispose();
        if (this.pending)
            clearTimeout(this.pending);
        this.artifacts.clear();
    }
}
let active;
/** Registers the live-preview commands and the scheme they serve. */
function registerLiveSdkPreview(getClient) {
    const emitter = new vscode.EventEmitter();
    const provider = vscode.workspace.registerTextDocumentContentProvider(LIVE_SDK_SCHEME, {
        onDidChange: emitter.event,
        provideTextDocumentContent: (uri) => active?.provideTextDocumentContent(uri) ?? 'Start Suspect: Live SDK Preview to see artifacts.',
    });
    return [
        provider,
        vscode.commands.registerCommand('suspect.liveSdkPreview', async () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor || editor.document.uri.scheme !== 'file') {
                void vscode.window.showWarningMessage('Open the specification to preview its SDK.');
                return;
            }
            let profiles;
            try {
                profiles = await (0, generation_1.availableSdkProfiles)((0, runner_1.suspectBinary)());
            }
            catch (error) {
                void vscode.window.showErrorMessage(`Suspect profile discovery failed: ${(0, runner_1.errorMessage)(error)}`);
                return;
            }
            const pick = await vscode.window.showQuickPick(profiles.map((p) => ({ label: p.profile, description: p.description })), { placeHolder: 'Suspect: live SDK preview — choose a profile' });
            if (!pick)
                return;
            const config = vscode.workspace.getConfiguration('suspect', editor.document.uri);
            const packageName = config.get('sdk.packageName', '');
            if (!packageName) {
                void vscode.window.showWarningMessage('Set suspect.sdk.packageName to identify the preview package.');
                return;
            }
            active?.dispose();
            active = new LiveSdkPreview(getClient, emitter, {
                uri: editor.document.uri,
                profile: pick.label,
                packageName,
                packageVersion: config.get('sdk.packageVersion', '0.1.0'),
                importName: config.get('sdk.importNames') || undefined,
                operationIds: config.get('sdk.operationIds', []),
            });
            active.start();
        }),
        vscode.commands.registerCommand('suspect.stopLiveSdkPreview', () => {
            active?.dispose();
            active = undefined;
        }),
    ];
}
//# sourceMappingURL=liveSdk.js.map