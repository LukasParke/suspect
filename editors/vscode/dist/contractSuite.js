"use strict";
// The contract suite, in the native Testing view.
//
// The workflow explorer runs any *.arazzo.yaml it finds, live, against the
// editor's base URL. Contract testing is what the project declares: the
// manifest's `tests` section names the Arazzo suites, the base URL they
// run against, and — the part a live run can never be — the recorded
// cassette to replay offline. This controller surfaces exactly that, and
// the CI gate stages beside it, so the Testing view answers the question
// CI answers: is this project green?
//
//   Suspect Contract Tests
//   └ <project>                       (suspect.project.json)
//     ├ validate / lint / breaking    (suspect ci --stage …, per-stage)
//     └ contract tests                (tests.arazzo, cassette honored)
//       └ <suite> └ <workflow> └ <steps>   (live ndjson step results)
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
exports.readManifestTests = readManifestTests;
exports.parseCiStages = parseCiStages;
exports.registerContractSuite = registerContractSuite;
const cp = __importStar(require("child_process"));
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const runner_1 = require("./runner");
const parse_1 = require("./parse");
/** Reads one project manifest's contract-test declaration. Pure: testable. */
function readManifestTests(text) {
    let parsed;
    try {
        parsed = JSON.parse(text);
    }
    catch {
        return undefined;
    }
    if (typeof parsed !== 'object' || parsed === null) {
        return undefined;
    }
    const tests = parsed.tests;
    if (typeof tests !== 'object' || tests === null) {
        return undefined;
    }
    const raw = tests;
    if (!Array.isArray(raw.arazzo)) {
        return undefined;
    }
    const arazzo = raw.arazzo.filter((f) => typeof f === 'string');
    const base_url = typeof raw.base_url === 'string' ? raw.base_url : 'http://localhost:8080';
    const cassette = typeof raw.cassette === 'string' ? raw.cassette : undefined;
    return { arazzo, base_url, cassette };
}
/** Extracts the first project's stages from a `ci --format json` document. Pure. */
function parseCiStages(text) {
    let parsed;
    try {
        parsed = JSON.parse(text);
    }
    catch {
        return [];
    }
    if (typeof parsed !== 'object' || parsed === null) {
        return [];
    }
    const projects = parsed.projects;
    if (!Array.isArray(projects) || projects.length === 0) {
        return [];
    }
    const stages = projects[0].stages;
    if (!Array.isArray(stages)) {
        return [];
    }
    return stages
        .filter((s) => typeof s === 'object' && s !== null)
        .map((s) => ({
        stage: String(s.stage ?? ''),
        passed: s.passed === true,
        errors: Number(s.errors ?? 0),
        warnings: Number(s.warnings ?? 0),
        summary: String(s.summary ?? ''),
    }));
}
const nodeDataByItem = new WeakMap();
const nodeData = (item) => nodeDataByItem.get(item);
const STAGES = [
    { id: 'validate', label: 'Validate' },
    { id: 'lint', label: 'Lint' },
    { id: 'breaking', label: 'Breaking' },
];
/** Runs one CI stage and returns its result. */
function runStage(binary, dir, stage) {
    return new Promise((resolve, reject) => {
        let out = '';
        const child = cp.spawn(binary, ['ci', '--stage', stage, '--format', 'json', '.'], { cwd: dir });
        child.stdout?.on('data', (c) => { out += c.toString(); });
        child.stderr?.on('data', () => { });
        child.on('error', reject);
        child.on('exit', () => {
            const stages = parseCiStages(out);
            const mine = stages.find((s) => s.stage === stage);
            resolve(mine ?? { stage, passed: false, errors: 0, warnings: 0, summary: 'no result' });
        });
    });
}
function registerContractSuite(context, binaryOf) {
    const controller = vscode.tests.createTestController('suspect.contract', 'Suspect Contract Tests');
    controller.resolveHandler = async (item) => {
        if (!item) {
            await refresh();
        }
    };
    controller.createRunProfile('Run', vscode.TestRunProfileKind.Run, runHandler, true);
    context.subscriptions.push(controller, vscode.commands.registerCommand('suspect.contract.refresh', () => void refresh()), vscode.workspace.onDidSaveTextDocument((doc) => {
        if (doc.uri.fsPath.endsWith('suspect.project.json') || /\.arazzo\.ya?ml$/i.test(doc.uri.fsPath)) {
            void refresh();
        }
    }));
    async function refresh() {
        const items = [];
        const manifests = await vscode.workspace.findFiles('**/suspect.project.json', '**/node_modules/**');
        for (const manifest of manifests.sort((a, b) => a.fsPath.localeCompare(b.fsPath))) {
            const dir = path.dirname(manifest.fsPath);
            const projectItem = controller.createTestItem(manifest.toString(), path.basename(dir), manifest);
            nodeDataByItem.set(projectItem, { kind: 'project', dir, manifest: manifest.fsPath });
            projectItem.canResolveChildren = true;
            for (const stage of STAGES) {
                const stageItem = controller.createTestItem(`${manifest.toString()}#stage/${stage.id}`, stage.label, manifest);
                nodeDataByItem.set(stageItem, { kind: 'stage', dir, stage: stage.id });
                projectItem.children.add(stageItem);
            }
            let tests;
            try {
                tests = readManifestTests(await fs.promises.readFile(manifest.fsPath, 'utf8'));
            }
            catch {
                tests = undefined;
            }
            if (tests !== undefined) {
                for (const rel of tests.arazzo) {
                    const suiteUri = vscode.Uri.file(path.join(dir, rel));
                    let parsed;
                    try {
                        parsed = (0, parse_1.parseArazzo)(await fs.promises.readFile(suiteUri.fsPath, 'utf8'));
                    }
                    catch {
                        const broken = controller.createTestItem(`${manifest}#suite/${rel}`, rel, suiteUri);
                        broken.description = 'unreadable';
                        projectItem.children.add(broken);
                        continue;
                    }
                    const suiteItem = controller.createTestItem(`${manifest}#suite/${rel}`, rel, suiteUri);
                    nodeDataByItem.set(suiteItem, { kind: 'suite', dir, file: suiteUri.fsPath, baseUrl: tests.base_url, cassette: tests.cassette });
                    suiteItem.description = tests.cassette !== undefined ? 'offline (cassette)' : tests.base_url;
                    for (const wf of parsed) {
                        const wfItem = controller.createTestItem(`${manifest}#suite/${rel}#${wf.workflowId}`, wf.workflowId, suiteUri);
                        nodeDataByItem.set(wfItem, { kind: 'workflow', dir, file: suiteUri.fsPath, baseUrl: tests.base_url, cassette: tests.cassette, workflowId: wf.workflowId });
                        wfItem.range = new vscode.Range(wf.line, 0, wf.line, 0);
                        for (const step of wf.steps) {
                            const stepItem = controller.createTestItem(`${manifest}#suite/${rel}#${wf.workflowId}\u0000${step.stepId}`, step.stepId, suiteUri);
                            nodeDataByItem.set(stepItem, {
                                kind: 'step',
                                dir,
                                file: suiteUri.fsPath,
                                baseUrl: tests.base_url,
                                cassette: tests.cassette,
                                workflowId: wf.workflowId,
                                stepId: step.stepId,
                            });
                            stepItem.range = new vscode.Range(step.line, 0, step.line, 0);
                            wfItem.children.add(stepItem);
                        }
                        suiteItem.children.add(wfItem);
                    }
                    projectItem.children.add(suiteItem);
                }
            }
            items.push(projectItem);
        }
        controller.items.replace(items);
    }
    async function runHandler(request, token) {
        const run = controller.createTestRun(request);
        const binary = binaryOf();
        const stageItems = [];
        const suites = new Map();
        const collect = (item) => {
            const data = nodeData(item);
            if (!data)
                return;
            switch (data.kind) {
                case 'project':
                    item.children.forEach(collect);
                    return;
                case 'stage':
                    stageItems.push(item);
                    return;
                case 'suite':
                    suites.set(item, data);
                    return;
                case 'workflow':
                    suites.set(item.parent, { ...data, kind: 'suite' });
                    return;
                case 'step':
                    suites.set(item.parent.parent, { ...nodeData(item.parent), kind: 'suite' });
                    return;
            }
        };
        if (request.include && request.include.length > 0) {
            request.include.forEach(collect);
        }
        else {
            controller.items.forEach(collect);
        }
        for (const item of stageItems) {
            const data = nodeData(item);
            if (!data || data.kind !== 'stage' || token.isCancellationRequested)
                continue;
            run.started(item);
            const result = await runStage(binary, data.dir, data.stage);
            const message = `${result.errors} error(s), ${result.warnings} warning(s) — ${result.summary}`;
            if (result.passed) {
                // Duration keeps the passing stage visible rather than
                // vanishing; the message carries the warning count.
                run.passed(item, 1);
            }
            else {
                run.failed(item, new vscode.TestMessage(message));
            }
        }
        for (const [suiteItem, data] of suites) {
            if (token.isCancellationRequested)
                break;
            await executeSuite(run, suiteItem, data, token, binary);
        }
        run.end();
    }
    async function executeSuite(run, suiteItem, data, token, binary) {
        const filter = data.kind === 'suite' ? undefined : data.workflowId;
        const wfItems = [];
        const stepItems = new Map();
        suiteItem.children.forEach((wf) => {
            if (filter !== undefined && wf.label !== filter)
                return;
            wfItems.push(wf);
            wf.children.forEach((step) => stepItems.set(`${wf.label}\u0000${step.label}`, step));
        });
        for (const item of wfItems)
            run.enqueued(item);
        const onEvent = (ev) => {
            switch (ev.event) {
                case 'step_started': {
                    const step = stepItems.get(`${ev.wf}\u0000${ev.step}`);
                    if (step)
                        run.started(step);
                    return;
                }
                case 'criterion_fail': {
                    const step = stepItems.get(`${ev.wf}\u0000${ev.step}`);
                    if (step) {
                        run.failed(step, new vscode.TestMessage(`${ev.crit} — expected: ${ev.expected}, actual: ${ev.actual}`));
                    }
                    return;
                }
                case 'wf_done': {
                    const wf = wfItems.find((w) => w.label === ev.wf);
                    if (wf) {
                        if (ev.passed) {
                            run.passed(wf);
                        }
                        else {
                            run.failed(wf, new vscode.TestMessage(`workflow ${ev.wf} finished with failures`));
                        }
                    }
                    // Steps that never reported stay in whatever state the
                    // criteria left them; the workflow item carries the verdict.
                    return;
                }
            }
        };
        const handle = (0, runner_1.spawnSuspectRunWith)(data.file, { baseUrl: data.baseUrl, cassette: data.cassette, filter: data.workflowId }, onEvent, binary);
        const cancelSub = token.onCancellationRequested(() => handle.kill());
        try {
            await handle.done;
            if (filter === undefined)
                run.passed(suiteItem);
        }
        catch (err) {
            run.errored(suiteItem, new vscode.TestMessage((0, runner_1.errorMessage)(err)));
        }
        finally {
            cancelSub.dispose();
        }
        void filter;
    }
}
//# sourceMappingURL=contractSuite.js.map