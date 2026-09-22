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
exports.registerSdkGeneration = registerSdkGeneration;
const cp = __importStar(require("child_process"));
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const node_1 = require("vscode-languageclient/node");
const parse_1 = require("./parse");
const notebook_1 = require("./notebook");
const runner_1 = require("./runner");
const generation_1 = require("./generation");
const testExplorer_1 = require("./testExplorer");
const workflowsView_1 = require("./workflowsView");
let client;
let gateway;
let gatewayStatus;
let sdkGeneration;
function activate(context) {
    context.subscriptions.push(vscode.commands.registerCommand('suspect.runWorkflow', (uri, workflow) => runWorkflowCommand(uri, workflow)), vscode.commands.registerCommand('suspect.startGateway', () => startGateway()), vscode.commands.registerCommand('suspect.stopGateway', () => stopGateway()), vscode.commands.registerCommand('_suspect.toggleGateway', () => (gateway ? stopGateway() : void startGateway())), vscode.commands.registerCommand('suspect.openNotebook', (uri) => openNotebook(uri)));
    gatewayStatus = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 90);
    gatewayStatus.command = '_suspect.toggleGateway';
    gatewayStatus.name = 'Suspect Gateway';
    context.subscriptions.push(gatewayStatus);
    (0, testExplorer_1.registerTestExplorer)(context);
    (0, workflowsView_1.registerWorkflowsView)(context);
    (0, notebook_1.registerNotebook)(context);
    sdkGeneration = registerSdkGeneration(context);
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
    if (sdkGeneration)
        tasks.push(sdkGeneration.stop());
    await Promise.all(tasks);
}
function startClient() {
    const serverOptions = {
        command: (0, runner_1.suspectBinary)(),
        args: ['lsp'],
        // Executables use stdio by default. An explicit transport makes the client
        // append --stdio, which the canonical `suspect lsp` command does not accept.
    };
    const clientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'yaml' },
            { scheme: 'file', language: 'json' },
        ],
    };
    client = new node_1.LanguageClient('suspect', 'Suspect', serverOptions, clientOptions);
    // Both the LSP and the editor advertise this workflow action. Keep the editor's
    // picker/progress implementation as its single owner, including for LSP lenses,
    // while registering every other server command normally.
    const commands = client.getFeature('workspace/executeCommand');
    const registerCommands = commands.register.bind(commands);
    commands.register = (data) => registerCommands({
        ...data,
        registerOptions: { ...data.registerOptions, commands: data.registerOptions.commands.filter((command) => command !== 'suspect.runWorkflow') },
    });
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
    const uri = await pickArazzoDocument(typeof uriHint === 'string' ? vscode.Uri.parse(uriHint) : uriHint);
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
/** Prompts for explicit package identity and exact selectors shared by the canonical HTTP profiles. */
async function promptPackageIdentity(config, profile) {
    const compatibilityProfiles = (0, generation_1.readSdkCompatibilityProfiles)(config.get('sdk.compatibilityProfiles', []));
    const packagePrompts = {
        'typescript-http': 'Explicit npm package name (independent of the API title)',
        'rust-http': 'Explicit Cargo package name (independent of the API title)',
        'python-http': 'Explicit Python distribution name; the CLI derives its import name by replacing hyphens with underscores',
        'go-http': 'Explicit Go module path, e.g. example.com/team/sdk; the generated Go package is named sdk',
        'swift-http': 'Swift package name: a valid Swift identifier, e.g. ExampleSDK; the default module has the same name',
        'java-http': 'Explicit Maven group:artifact coordinates, e.g. com.example:widgets-sdk',
        'csharp-http': 'Explicit NuGet package ID, e.g. Example.Widgets',
        'kotlin-http': 'Explicit Maven group:artifact coordinates, e.g. com.example:widgets-sdk',
        'ruby-http': 'Explicit gem name, e.g. widgets-sdk; require path defaults to widgets_sdk',
        'php-http': 'Explicit Composer vendor/package name, e.g. example/widgets-sdk',
        'dart-http': 'Explicit lowercase pub package name, e.g. widgets_sdk',
        'cpp-http': 'Explicit CMake target/include identifier, e.g. widgets_sdk',
    };
    const packageName = await vscode.window.showInputBox({
        prompt: packagePrompts[profile],
        value: config.get('sdk.packageName', ''),
        validateInput: (value) => value.length ? undefined : 'A package name is required',
    });
    if (packageName === undefined)
        return undefined;
    const packageVersion = await vscode.window.showInputBox({
        prompt: profile === 'python-http'
            ? `Explicit package SemVer; default Python import: ${packageName.replace(/-/g, '_')}. Configure sdk.importNames for an override.`
            : profile === 'swift-http'
                ? `Exact stable SemVer (no prerelease/build metadata); default Swift module: ${packageName}. Configure sdk.importNames for an override.`
                : 'Explicit package SemVer (independent of the API version)',
        value: config.get('sdk.packageVersion', ''),
        validateInput: (value) => value.length ? undefined : 'A package version is required',
    });
    if (packageVersion === undefined)
        return undefined;
    const selectors = await vscode.window.showInputBox({
        prompt: 'Exact operation IDs as a JSON string array; [] attempts every operation',
        value: JSON.stringify(config.get('sdk.operationIds', [])),
        validateInput: (value) => {
            try {
                const ids = JSON.parse(value);
                if (Array.isArray(ids) && ids.every((id) => typeof id === 'string'))
                    return undefined;
            }
            catch { /* Show the same actionable validation message for invalid JSON. */ }
            return 'Enter a JSON array of exact operation ID strings, or [] for all';
        },
    });
    if (selectors === undefined)
        return undefined;
    const importName = config.get('sdk.importNames', {})[profile];
    return { packageName, packageVersion, ...(importName ? { importName } : {}), compatibilityProfiles, operationIds: JSON.parse(selectors) };
}
async function genPresetCommand() {
    const binary = (0, runner_1.suspectBinary)();
    let profiles;
    try {
        profiles = await (0, generation_1.availableSdkProfiles)(binary);
    }
    catch (error) {
        vscode.window.showErrorMessage(`Suspect profile discovery failed: ${(0, runner_1.errorMessage)(error)}`);
        return;
    }
    const labels = {
        'typescript-http': 'TypeScript / JavaScript', 'rust-http': 'Rust', 'python-http': 'Python',
        'go-http': 'Go', 'swift-http': 'Swift', 'java-http': 'Java', 'csharp-http': 'C#',
        'kotlin-http': 'Kotlin', 'ruby-http': 'Ruby', 'php-http': 'PHP', 'dart-http': 'Dart', 'cpp-http': 'C++',
    };
    const presetPick = await vscode.window.showQuickPick([
        ...profiles.map((profile) => ({ label: `${labels[profile.profile]} HTTP SDK`, generationKind: profile.profile, description: profile.description })),
        { label: 'Markdown documentation', generationKind: 'docs-md' },
        { label: 'Custom template manifest', generationKind: 'custom' },
    ], { placeHolder: 'Suspect: choose generation output' });
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
    const outDir = path.join(folder.uri.fsPath, 'gen-out', presetPick.generationKind);
    let generation;
    try {
        if ((0, generation_1.isSdkProfile)(presetPick.generationKind)) {
            const identity = await promptPackageIdentity(vscode.workspace.getConfiguration('suspect', spec), presetPick.generationKind);
            if (!identity)
                return;
            generation = { kind: presetPick.generationKind, ...identity };
        }
        else if (presetPick.generationKind === 'custom') {
            const manifests = await vscode.window.showOpenDialog({
                canSelectMany: false, openLabel: 'Select generation manifest', filters: { 'Generation manifest': ['toml'] },
            });
            if (!manifests?.[0])
                return;
            generation = { kind: 'custom', manifest: manifests[0].fsPath };
        }
        else {
            generation = { kind: presetPick.generationKind };
        }
        await vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title: `Suspect: ${presetPick.label}` }, () => (0, generation_1.runGeneration)(binary, (0, generation_1.generationArgs)(spec.fsPath, outDir, generation), folder.uri.fsPath));
    }
    catch (err) {
        vscode.window.showErrorMessage(`Suspect generation failed: ${(0, runner_1.errorMessage)(err)}`);
        return;
    }
    const produced = (0, generation_1.isSdkProfile)(generation.kind)
        ? path.join(outDir, generation_1.SDK_PROFILE_DIRECTORIES[generation.kind], 'README.md')
        : await firstFileRecursive(outDir);
    if (!produced) {
        vscode.window.showWarningMessage(`Suspect produced no files under ${outDir}.`);
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
const SDK_PREVIEW_SCHEME = 'suspect-sdk-preview';
const SDK_OUTPUT_LIMIT = 64 * 1024;
/** A single read-only diff pair. Replaced/failed sessions invalidate already-open documents. */
class SdkPreviewDocuments {
    emitter = new vscode.EventEmitter();
    onDidChange = this.emitter.event;
    contents = new Map();
    unavailable = 'This SDK preview has expired. Use Suspect: Show Latest SDK Preview.';
    provideTextDocumentContent(uri) {
        return this.contents.get(uri.toString())?.text ?? this.unavailable;
    }
    set(session, artifact, current, generated) {
        const identity = session.identity;
        const query = new URLSearchParams({
            session: String(session.epoch), config: identity.configPath,
            source: identity.sourcePath ?? '', output: identity.outDirectory,
        }).toString();
        const left = vscode.Uri.from({ scheme: SDK_PREVIEW_SCHEME, authority: 'current', path: `/${artifact}`, query });
        const right = vscode.Uri.from({ scheme: SDK_PREVIEW_SCHEME, authority: 'generated', path: `/${artifact}`, query });
        const next = new Map([
            [left.toString(), { uri: left, text: current }],
            [right.toString(), { uri: right, text: generated }],
        ]);
        const previous = this.contents;
        this.contents = next;
        this.unavailable = 'This SDK preview has expired. Use Suspect: Show Latest SDK Preview.';
        for (const [key, value] of previous)
            if (!next.has(key))
                this.emitter.fire(value.uri);
        for (const [key, value] of next)
            if (previous.get(key)?.text !== value.text)
                this.emitter.fire(value.uri);
        return [left, right];
    }
    clear(message) {
        this.unavailable = message;
        const previous = this.contents;
        this.contents = new Map();
        for (const value of previous.values())
            this.emitter.fire(value.uri);
    }
    dispose() {
        this.clear('SDK preview disposed.');
        this.emitter.dispose();
    }
}
/** Register separately so process/UI seam tests use the real command handlers without an LSP. */
function registerSdkGeneration(context) {
    const controller = new SdkGenerationController();
    context.subscriptions.push(controller);
    return controller;
}
class SdkGenerationController {
    output = vscode.window.createOutputChannel('Suspect SDK');
    status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 89);
    documents = new SdkPreviewDocuments();
    disposables;
    session;
    epoch = 0;
    disposed = false;
    stopping = Promise.resolve();
    constructor() {
        this.status.name = 'Suspect SDK Session';
        this.status.command = 'suspect.showSdkPreview';
        this.disposables = [
            vscode.workspace.registerTextDocumentContentProvider(SDK_PREVIEW_SCHEME, this.documents),
            vscode.commands.registerCommand('suspect.genPreset', () => genPresetCommand()),
            vscode.commands.registerCommand('suspect.previewSdk', (uri) => this.run('preview', uri)),
            vscode.commands.registerCommand('suspect.checkSdk', (uri) => this.run('check', uri)),
            vscode.commands.registerCommand('suspect.watchSdk', (uri) => this.run('watch', uri)),
            vscode.commands.registerCommand('suspect.stopSdkWatch', () => this.stop()),
            vscode.commands.registerCommand('suspect.showSdkPreview', () => this.showLatest()),
            vscode.workspace.onDidChangeConfiguration((event) => {
                if (['suspect.basePath', 'suspect.sdk.sessionConfig', 'suspect.sdk.sessionOutput'].some((key) => event.affectsConfiguration(key))) {
                    void this.stop('SDK settings changed. Start an SDK preview or watch again.');
                }
            }),
        ];
    }
    isCurrent(session) {
        return !this.disposed && this.session === session && this.epoch === session.epoch;
    }
    release(message) {
        const previous = this.session;
        this.session = undefined;
        previous?.diskWatcher?.dispose();
        this.documents.clear(message);
        this.status.hide();
        this.output.clear();
        this.output.appendLine(message);
        if (previous?.handle) {
            previous.handle.dispose();
            // Every later action also waits for a process already being terminated.
            this.stopping = Promise.all([this.stopping, previous.handle.done.catch(() => undefined)]).then(() => undefined);
        }
        return this.stopping;
    }
    async stop(message = 'SDK watch/preview stopped.') {
        this.epoch += 1;
        await this.release(message);
    }
    dispose() {
        if (this.disposed)
            return;
        this.disposed = true;
        this.epoch += 1;
        void this.release('SDK preview disposed.');
        for (const disposable of this.disposables)
            disposable.dispose();
        this.documents.dispose();
        this.status.dispose();
        this.output.dispose();
    }
    async pickIdentity(hint) {
        let configUri = hint;
        if (!configUri) {
            const active = vscode.window.activeTextEditor?.document.uri;
            let folder = active ? vscode.workspace.getWorkspaceFolder(active) : undefined;
            if (!folder && vscode.workspace.workspaceFolders?.length === 1)
                folder = vscode.workspace.workspaceFolders[0];
            let configured = vscode.workspace.getConfiguration('suspect', folder?.uri).get('sdk.sessionConfig', '');
            if (configured && !path.isAbsolute(configured) && !folder) {
                folder = await vscode.window.showWorkspaceFolderPick({ placeHolder: 'Workspace containing the SDK session config' });
                if (!folder)
                    return undefined;
                configured = vscode.workspace.getConfiguration('suspect', folder.uri).get('sdk.sessionConfig', '');
            }
            if (configured) {
                configUri = vscode.Uri.file(path.resolve(folder?.uri.fsPath ?? '', configured));
            }
            else {
                configUri = (await vscode.window.showOpenDialog({
                    canSelectMany: false, canSelectFiles: true, canSelectFolders: false,
                    openLabel: 'Select SDK session config', filters: { 'SDK session config': ['json'] },
                    defaultUri: active?.scheme === 'file' ? active : folder?.uri,
                }))?.[0];
            }
        }
        if (!configUri)
            return undefined;
        if (configUri.scheme !== 'file')
            throw new Error('Select a local JSON SDK session config.');
        const settings = vscode.workspace.getConfiguration('suspect', configUri);
        const out = await vscode.window.showInputBox({
            prompt: `SDK output directory to compare; relative paths resolve from ${path.dirname(configUri.fsPath)}`,
            value: settings.get('sdk.sessionOutput', 'sdk-out'),
            validateInput: (value) => value.length && !value.includes('\0') ? undefined : 'Enter an output directory',
        });
        if (out === undefined)
            return undefined;
        return (0, generation_1.readSdkSessionIdentity)(configUri.fsPath, out);
    }
    async run(action, hint) {
        if (this.disposed)
            return;
        const epoch = ++this.epoch;
        await this.release('Selecting SDK session configuration…');
        try {
            if (epoch !== this.epoch || this.disposed)
                return;
            const identity = await this.pickIdentity(hint);
            if (epoch !== this.epoch || this.disposed)
                return;
            if (!identity) {
                this.output.clear();
                this.output.appendLine('SDK action cancelled.');
                return;
            }
            const session = {
                identity, action, epoch, running: true, openDiff: false, initialPickerShown: false,
                revision: 0, refreshing: false, update: Promise.resolve(),
            };
            this.session = session;
            this.report(session, 'Starting canonical CLI session…');
            const launch = async (token) => {
                if (!this.isCurrent(session))
                    return;
                if (token?.isCancellationRequested) {
                    await this.stop('SDK action cancelled.');
                    return;
                }
                session.handle = (0, generation_1.startSdkSession)((0, runner_1.suspectBinary)(), identity, {
                    watch: action === 'watch', check: action === 'check', preview: action !== 'check',
                }, (record) => this.receive(session, record));
                const cancel = token?.onCancellationRequested(() => {
                    if (this.isCurrent(session))
                        void this.stop('SDK action cancelled.');
                });
                try {
                    await session.handle.done;
                    await session.update;
                    if (!this.isCurrent(session))
                        return;
                    session.running = false;
                    if (action === 'watch')
                        throw new Error('SDK watch ended. Start Watch SDK Preview to resume.');
                    this.report(session);
                    if (action === 'preview' && session.record?.status !== 'planning-error')
                        await this.showLatest();
                    else
                        this.output.show(true);
                }
                catch (error) {
                    if (this.isCurrent(session))
                        this.failed(session, error);
                }
                finally {
                    cancel?.dispose();
                }
            };
            if (action === 'watch') {
                void launch();
            }
            else {
                await vscode.window.withProgress({
                    location: vscode.ProgressLocation.Notification, title: `Suspect: SDK ${action}`, cancellable: true,
                }, (_progress, token) => launch(token));
            }
        }
        catch (error) {
            if (epoch === this.epoch && !this.disposed) {
                this.documents.clear('SDK preview unavailable. See Suspect SDK output.');
                this.output.clear();
                this.output.appendLine((0, runner_1.errorMessage)(error).slice(0, SDK_OUTPUT_LIMIT));
                vscode.window.showErrorMessage(`Suspect SDK: ${(0, runner_1.errorMessage)(error).slice(0, 2000)}`);
            }
        }
    }
    receive(session, record) {
        if (!this.isCurrent(session))
            return;
        if (record.source && record.source !== session.identity.sourcePath) {
            this.documents.clear('SDK source identity changed. Waiting for the new preview.');
            session.identity = { ...session.identity, sourcePath: record.source, sourceRoot: path.dirname(record.source) };
            session.openDiff = session.selectedPath !== undefined;
        }
        session.record = record;
        if (record.status === 'planning-error')
            this.documents.clear('SDK planning failed. See Suspect SDK output for the latest diagnostics.');
        else if (session.selectedPath && !this.artifactPaths(record).includes(session.selectedPath)) {
            this.documents.clear('This artifact is no longer in the SDK preview. Use Suspect: Show Latest SDK Preview.');
            session.selectedPath = undefined;
            session.diskWatcher?.dispose();
            session.diskWatcher = undefined;
        }
        this.report(session);
        this.refresh(session);
        if (session.action === 'watch' && !session.initialPickerShown && record.status !== 'planning-error') {
            session.initialPickerShown = true;
            void session.update.then(() => this.isCurrent(session) ? this.showLatest() : undefined).catch((error) => {
                if (this.isCurrent(session))
                    this.failed(session, error);
            });
        }
    }
    artifactPaths(record) {
        return [...new Set([...(record.artifacts ?? []).map((file) => file.path), ...record.changedArtifacts])];
    }
    report(session, detail) {
        const { identity, record } = session;
        const state = record?.status ?? (session.running ? 'starting' : 'error');
        const lines = [
            `SDK ${session.action}${session.action === 'watch' && session.running ? ' (watching saved files)' : ''}: ${state}${record ? ` · generation ${record.generation}` : ''}`,
            `Config: ${identity.configPath}`, `Config directory / CLI cwd: ${identity.configDirectory}`,
            `${record?.status === 'planning-error' ? 'Last resolved source' : 'Source'}: ${identity.sourcePath ?? '(unresolved; see CLI diagnostics)'}`,
            `Source root: ${identity.sourceRoot ?? '(unresolved)'}`, `Output root: ${identity.outDirectory}`,
        ];
        if (record) {
            if (record.revision)
                lines.push(`Revision: ${record.revision}`);
            if (record.compatibilityProfiles)
                lines.push(`Compatibility profiles: ${JSON.stringify(record.compatibilityProfiles)}`);
            lines.push(`Success: ${record.success}`, `Delta: ${JSON.stringify(record.delta)}`, `Stats: ${JSON.stringify(record.stats)}`, `Changed artifacts: ${record.changedArtifacts.length}`, `New documents: ${record.newDocuments.length}`);
            let size = lines.join('\n').length;
            for (const diagnostic of record.diagnostics) {
                const line = JSON.stringify(diagnostic);
                lines.push(line);
                size += line.length + 1;
                if (size > SDK_OUTPUT_LIMIT)
                    break;
            }
        }
        if (detail)
            lines.push(detail);
        const text = lines.join('\n');
        const bytes = Buffer.from(text);
        const suffix = '\n[Latest report truncated to 64 KiB]';
        this.output.clear();
        this.output.appendLine(bytes.length < SDK_OUTPUT_LIMIT ? text : `${bytes.subarray(0, SDK_OUTPUT_LIMIT - Buffer.byteLength(suffix) - 4).toString('utf8')}${suffix}`);
        const icon = state === 'planning-error' || state === 'write-conflict' || state === 'error' ? 'error' :
            state === 'drift' ? 'warning' : session.running ? 'sync~spin' : 'check';
        this.status.text = `$(${icon}) SDK${record ? ` #${record.generation}` : ''} · ${state}`;
        this.status.tooltip = `${lines.slice(0, 6).join('\n').slice(0, 8192)}\nClick for latest SDK preview/diagnostics.`;
        this.status.show();
    }
    failed(session, error) {
        session.handle?.dispose();
        session.diskWatcher?.dispose();
        session.diskWatcher = undefined;
        session.running = false;
        session.record = undefined;
        session.revision += 1;
        this.documents.clear('SDK preview unavailable. See Suspect SDK output for the latest error.');
        this.report(session, (0, runner_1.errorMessage)(error));
        this.output.show(true);
        vscode.window.showErrorMessage(`Suspect SDK: ${(0, runner_1.errorMessage)(error).slice(0, 2000)}`);
    }
    async showLatest() {
        const session = this.session;
        const record = session?.record;
        if (!session || !record || record.status === 'planning-error' || !record.artifacts) {
            this.output.show(true);
            return;
        }
        const changed = new Set(record.changedArtifacts);
        const desired = new Set(record.artifacts.map((file) => file.path));
        const paths = this.artifactPaths(record).sort((a, b) => {
            const order = Number(changed.has(b)) - Number(changed.has(a));
            return order || a.localeCompare(b);
        });
        if (!paths.length) {
            this.output.show(true);
            return;
        }
        const source = session.identity.sourcePath;
        const pick = await vscode.window.showQuickPick(paths.map((artifact) => ({
            label: artifact,
            description: !desired.has(artifact) ? 'removed from desired output' :
                changed.has(artifact) ? 'disk drift / ownership change' : 'current artifact',
        })), { placeHolder: `SDK generation ${record.generation}: ${record.status} — ${session.identity.configPath}` });
        if (!pick || !this.isCurrent(session) || source !== session.identity.sourcePath)
            return;
        // The picker may outlive several watch iterations. Use only the latest record.
        if (!session.record?.artifacts || session.record.status === 'planning-error' || !this.artifactPaths(session.record).includes(pick.label))
            return;
        session.selectedPath = pick.label;
        session.openDiff = true;
        this.watchCurrentArtifact(session);
        this.refresh(session);
        await session.update;
    }
    watchCurrentArtifact(session) {
        session.diskWatcher?.dispose();
        session.diskWatcher = undefined;
        if (session.action !== 'watch' || !session.selectedPath)
            return;
        const filename = path.join(session.identity.outDirectory, session.selectedPath);
        // A second user edit can leave the CLI's drift status/path list unchanged.
        // Observe only the selected disk file so its left-hand snapshot remains current.
        const literalName = path.basename(filename).replace(/[\[\]{}*?]/g, (character) => `[${character}]`);
        const watcher = vscode.workspace.createFileSystemWatcher(new vscode.RelativePattern(path.dirname(filename), literalName));
        const changed = (uri) => {
            if (this.isCurrent(session) && uri.fsPath === filename)
                this.refresh(session);
        };
        const subscriptions = [watcher.onDidChange(changed), watcher.onDidCreate(changed), watcher.onDidDelete(changed)];
        session.diskWatcher = { dispose: () => { subscriptions.forEach((subscription) => subscription.dispose()); watcher.dispose(); } };
    }
    /** Coalesce frequent records/selection changes into at most one in-flight disk read. */
    refresh(session, invalidate = true) {
        if (invalidate)
            session.revision += 1;
        if (session.refreshing)
            return;
        session.refreshing = true;
        let processedRevision = 0;
        session.update = (async () => {
            let revision;
            do {
                revision = session.revision;
                processedRevision = revision;
                const record = session.record;
                const selected = session.selectedPath;
                if (!this.isCurrent(session) || !record || record.status === 'planning-error' || !record.artifacts || !selected)
                    return;
                try {
                    const current = await (0, generation_1.readCurrentSdkArtifact)(session.identity.outDirectory, selected);
                    if (!this.isCurrent(session))
                        return;
                    if (revision !== session.revision)
                        continue;
                    const artifact = record.artifacts.find((file) => file.path === selected);
                    const [left, right] = this.documents.set(session, selected, current.content, artifact?.content ?? '');
                    if (session.openDiff) {
                        session.openDiff = false;
                        await vscode.commands.executeCommand('vscode.diff', left, right, `${selected} — disk${current.exists ? '' : ' (missing)'} ↔ SDK${artifact ? '' : ' (removed)'} · ${path.basename(session.identity.configPath)}`, { preview: true });
                    }
                }
                catch (error) {
                    if (!this.isCurrent(session))
                        return;
                    if (revision !== session.revision)
                        continue;
                    this.documents.clear('SDK diff unavailable. See Suspect SDK output.');
                    this.report(session, `Cannot read current artifact ${selected}: ${(0, runner_1.errorMessage)(error)}`);
                    this.output.show(true);
                }
            } while (this.isCurrent(session) && revision !== session.revision);
        })().finally(() => {
            session.refreshing = false;
            // A new record can arrive after an early return but before this microtask.
            if (this.isCurrent(session) && processedRevision !== session.revision) {
                this.refresh(session, false);
                return session.update;
            }
        });
    }
}
async function openNotebook(uriHint) {
    const uri = await pickArazzoDocument(uriHint);
    if (!uri) {
        return;
    }
    await vscode.commands.executeCommand('vscode.openWith', uri, 'suspect.notebook');
}
//# sourceMappingURL=extension.js.map