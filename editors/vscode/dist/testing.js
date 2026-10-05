"use strict";
// The Testing view: one controller, "Suspect".
//
// VS Code groups the Testing view by controller, so two controllers with
// overlapping content ("Suspect Workflows" scanning file names, "Suspect
// Contract Tests" reading the manifest) read as two products. There is
// one question the view should answer — is this project green? — so
// there is one tree:
//
//   <project>                           suspect.project.json
//   ├ Gate                              suspect ci --stage …
//   │  ├ Validate · Lint · Breaking
//   └ Contract tests                    tests.arazzo, manifest base URL / cassette
//      └ <suite> └ <workflow> └ <step>  live ndjson step results
//   Other workflows                     Arazzo documents no manifest declares
//      └ <document> └ <workflow> └ <step>   editor base URL
//
// Arazzo documents are found by content, so a suite named `health.yaml`
// is as visible as `health.arazzo.yaml`.
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
exports.parseCiStages = parseCiStages;
exports.groupJobs = groupJobs;
exports.registerTesting = registerTesting;
const cp = __importStar(require("child_process"));
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const discover_1 = require("./discover");
const parse_1 = require("./parse");
const project_1 = require("./project");
const runner_1 = require("./runner");
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
/**
 * Groups a selection into the fewest `suspect test` runs: selecting a
 * document (or the group above it) runs it whole; selecting workflows
 * runs each by id. Pure: testable.
 */
function groupJobs(selection) {
    const jobs = new Map();
    for (const pick of selection) {
        let job = jobs.get(pick.file);
        if (job === undefined) {
            job = { file: pick.file, cwd: pick.cwd, baseUrl: pick.baseUrl, cassette: pick.cassette, workflows: [] };
            jobs.set(pick.file, job);
        }
        if (pick.workflowId === undefined) {
            job.workflows = undefined;
        }
        else if (job.workflows !== undefined && !job.workflows.includes(pick.workflowId)) {
            job.workflows.push(pick.workflowId);
        }
    }
    return [...jobs.values()];
}
const nodeDataByItem = new WeakMap();
const nodeData = (item) => nodeDataByItem.get(item);
const STAGES = [
    { id: 'validate', label: 'Validate' },
    { id: 'lint', label: 'Lint' },
    { id: 'breaking', label: 'Breaking' },
];
const UNDECLARED_ID = 'other-workflows';
function ciBaseline() {
    return vscode.workspace.getConfiguration('suspect').get('ci.baseline', '').trim();
}
/** Runs one CI stage and returns its result. */
function runStage(binary, dir, stage) {
    return new Promise((resolve, reject) => {
        let out = '';
        const args = ['ci', '--stage', stage, '--format', 'json'];
        const baseline = ciBaseline();
        if (baseline)
            args.push('--baseline', baseline);
        args.push('.');
        const child = cp.spawn(binary, args, { cwd: dir });
        child.stdout?.on('data', (c) => { out += c.toString(); });
        child.stderr?.on('data', () => { });
        child.on('error', reject);
        child.on('exit', () => {
            const mine = parseCiStages(out).find((s) => s.stage === stage);
            resolve(mine ?? { stage, passed: false, errors: 0, warnings: 0, summary: 'no result from suspect ci' });
        });
    });
}
function registerTesting(context, binaryOf) {
    const controller = vscode.tests.createTestController('suspect', 'Suspect');
    const log = vscode.window.createOutputChannel('Suspect Tests');
    controller.resolveHandler = async (item) => {
        if (!item)
            await safeRefresh();
    };
    controller.refreshHandler = () => safeRefresh();
    controller.createRunProfile('Run', vscode.TestRunProfileKind.Run, runHandler, true);
    context.subscriptions.push(controller, log, vscode.workspace.onDidChangeWorkspaceFolders(() => void safeRefresh()), vscode.workspace.onDidSaveTextDocument((doc) => {
        if ((0, discover_1.affectsProject)(doc))
            void safeRefresh();
    }));
    // Discover eagerly: the tree is small, and open Arazzo documents get
    // their gutter run controls without the Testing view being opened first.
    void safeRefresh();
    async function safeRefresh() {
        try {
            const built = await refresh();
            log.appendLine(`${new Date().toISOString()} discovered ${built.projects} project(s), ${built.documents} Arazzo document(s), ${built.workflows} workflow(s), ${built.steps} step(s)`);
        }
        catch (err) {
            // A throwing resolve handler empties the Testing view silently;
            // say what happened where the user can find it.
            const message = `Suspect: could not build the test tree: ${(0, runner_1.errorMessage)(err)}`;
            log.appendLine(message);
            console.error(message);
            void vscode.window.showErrorMessage(message, 'Show log').then((pick) => pick && log.show());
        }
    }
    function addDocument(parent, idPrefix, label, uri, cwd, baseUrl, cassette, counts) {
        return fs.promises.readFile(uri.fsPath, 'utf8').then((text) => {
            counts.documents += 1;
            const docItem = controller.createTestItem((0, project_1.testId)(idPrefix, uri.fsPath), label, uri);
            nodeDataByItem.set(docItem, { kind: 'document', file: uri.fsPath, cwd, baseUrl, cassette });
            docItem.description = cassette !== undefined ? 'offline · cassette' : baseUrl;
            for (const wf of (0, parse_1.parseArazzo)(text)) {
                counts.workflows += 1;
                const wfItem = controller.createTestItem((0, project_1.testId)(idPrefix, uri.fsPath, wf.workflowId), wf.workflowId, uri);
                nodeDataByItem.set(wfItem, { kind: 'workflow', file: uri.fsPath, cwd, baseUrl, cassette, workflowId: wf.workflowId });
                wfItem.range = new vscode.Range(wf.line, 0, wf.line, 0);
                for (const step of wf.steps) {
                    counts.steps += 1;
                    const stepItem = controller.createTestItem((0, project_1.testId)(idPrefix, uri.fsPath, wf.workflowId, step.stepId), step.stepId, uri);
                    nodeDataByItem.set(stepItem, {
                        kind: 'step', file: uri.fsPath, cwd, baseUrl, cassette, workflowId: wf.workflowId, stepId: step.stepId,
                    });
                    stepItem.range = new vscode.Range(step.line, 0, step.line, 0);
                    wfItem.children.add(stepItem);
                }
                docItem.children.add(wfItem);
            }
            parent.children.add(docItem);
        }, (err) => {
            const broken = controller.createTestItem((0, project_1.testId)(idPrefix, uri.fsPath), label, uri);
            broken.description = 'unreadable';
            broken.error = (0, runner_1.errorMessage)(err);
            parent.children.add(broken);
        });
    }
    async function refresh() {
        const counts = { projects: 0, documents: 0, workflows: 0, steps: 0 };
        const items = [];
        const declared = [];
        const projects = await (0, discover_1.findProjects)();
        counts.projects = projects.length;
        for (const project of projects) {
            const { manifestUri, dir, manifest } = project;
            const projectItem = controller.createTestItem((0, project_1.testId)('project', manifestUri.fsPath), manifest.name ?? path.basename(dir), manifestUri);
            nodeDataByItem.set(projectItem, { kind: 'project', dir });
            projectItem.description = vscode.workspace.asRelativePath(manifestUri);
            const gate = controller.createTestItem((0, project_1.testId)('gate', manifestUri.fsPath), 'Gate', manifestUri);
            nodeDataByItem.set(gate, { kind: 'gate', dir });
            gate.description = 'suspect ci';
            gate.sortText = '0';
            for (const stage of STAGES) {
                const stageItem = controller.createTestItem((0, project_1.testId)('stage', manifestUri.fsPath, stage.id), stage.label, manifestUri);
                nodeDataByItem.set(stageItem, { kind: 'stage', dir, stage: stage.id });
                gate.children.add(stageItem);
            }
            projectItem.children.add(gate);
            if (manifest.tests !== undefined) {
                const suites = controller.createTestItem((0, project_1.testId)('suites', manifestUri.fsPath), 'Contract tests', manifestUri);
                nodeDataByItem.set(suites, { kind: 'group' });
                suites.description = manifest.tests.cassette !== undefined ? 'offline · cassette' : manifest.tests.base_url;
                suites.sortText = '1';
                const cassette = manifest.tests.cassette !== undefined ? path.resolve(dir, manifest.tests.cassette) : undefined;
                for (const rel of manifest.tests.arazzo) {
                    const file = path.resolve(dir, rel);
                    declared.push(file);
                    await addDocument(suites, (0, project_1.testId)('suite', manifestUri.fsPath), rel, vscode.Uri.file(file), dir, manifest.tests.base_url, cassette, counts);
                }
                projectItem.children.add(suites);
            }
            items.push(projectItem);
        }
        const found = (await (0, discover_1.findArazzoDocuments)()).map((uri) => uri.fsPath);
        const others = (0, project_1.undeclaredDocuments)(found, declared);
        if (others.length > 0) {
            const group = controller.createTestItem(UNDECLARED_ID, projects.length > 0 ? 'Other workflows' : 'Workflows');
            nodeDataByItem.set(group, { kind: 'group' });
            group.description = projects.length > 0 ? 'not declared by a manifest' : (0, runner_1.testBaseUrl)();
            group.sortText = 'z';
            for (const file of others) {
                const uri = vscode.Uri.file(file);
                const folder = vscode.workspace.getWorkspaceFolder(uri);
                await addDocument(group, UNDECLARED_ID, vscode.workspace.asRelativePath(uri), uri, folder?.uri.fsPath ?? path.dirname(file), (0, runner_1.testBaseUrl)(), undefined, counts);
            }
            items.push(group);
        }
        controller.items.replace(items);
        return counts;
    }
    async function runHandler(request, token) {
        const run = controller.createTestRun(request);
        const binary = binaryOf();
        const stageItems = [];
        const selection = [];
        const docItems = new Map();
        const collect = (item) => {
            const data = nodeData(item);
            if (!data)
                return;
            switch (data.kind) {
                case 'project':
                case 'gate':
                case 'group':
                    item.children.forEach(collect);
                    return;
                case 'stage':
                    stageItems.push(item);
                    return;
                case 'document':
                    docItems.set(data.file, item);
                    selection.push({ file: data.file, cwd: data.cwd, baseUrl: data.baseUrl, cassette: data.cassette });
                    return;
                case 'workflow':
                    docItems.set(data.file, item.parent);
                    selection.push({ file: data.file, cwd: data.cwd, baseUrl: data.baseUrl, cassette: data.cassette, workflowId: data.workflowId });
                    return;
                case 'step':
                    docItems.set(data.file, item.parent.parent);
                    selection.push({ file: data.file, cwd: data.cwd, baseUrl: data.baseUrl, cassette: data.cassette, workflowId: data.workflowId });
                    return;
            }
        };
        if (request.include && request.include.length > 0) {
            request.include.forEach(collect);
        }
        else {
            controller.items.forEach(collect);
        }
        try {
            for (const item of stageItems) {
                const data = nodeData(item);
                if (!data || data.kind !== 'stage' || token.isCancellationRequested)
                    continue;
                run.started(item);
                const started = Date.now();
                const result = await runStage(binary, data.dir, data.stage);
                const duration = Date.now() - started;
                const message = `${result.errors} error(s), ${result.warnings} warning(s) — ${result.summary}`;
                run.appendOutput(`${data.stage}: ${message}\r\n`, undefined, item);
                if (result.passed) {
                    run.passed(item, duration);
                }
                else {
                    run.failed(item, new vscode.TestMessage(message), duration);
                }
            }
            for (const job of groupJobs(selection)) {
                if (token.isCancellationRequested)
                    break;
                const docItem = docItems.get(job.file);
                if (docItem === undefined)
                    continue;
                if (job.workflows === undefined) {
                    await executeDocument(run, docItem, job, undefined, token, binary);
                }
                else {
                    for (const workflowId of job.workflows) {
                        if (token.isCancellationRequested)
                            break;
                        await executeDocument(run, docItem, job, workflowId, token, binary);
                    }
                }
            }
        }
        finally {
            run.end();
        }
    }
    async function executeDocument(run, docItem, job, filter, token, binary) {
        const wfItems = new Map();
        const stepItems = new Map();
        docItem.children.forEach((wf) => {
            if (filter !== undefined && wf.label !== filter)
                return;
            wfItems.set(wf.label, wf);
            wf.children.forEach((step) => stepItems.set(`${wf.label}/${step.label}`, step));
        });
        for (const item of wfItems.values())
            run.enqueued(item);
        const started = new Set();
        const failed = new Set();
        const onEvent = (ev) => {
            switch (ev.event) {
                case 'step_started': {
                    const step = stepItems.get(`${ev.wf}/${ev.step}`);
                    if (step) {
                        started.add(step);
                        run.started(step);
                    }
                    return;
                }
                case 'request_sent':
                    run.appendOutput(`${ev.wf}/${ev.step}: ${ev.method} ${ev.url}\r\n`, undefined, stepItems.get(`${ev.wf}/${ev.step}`));
                    return;
                case 'response_got': {
                    const step = stepItems.get(`${ev.wf}/${ev.step}`);
                    run.appendOutput(`${ev.wf}/${ev.step}: ${ev.status} in ${ev.duration_ms} ms\r\n`, undefined, step);
                    return;
                }
                case 'criterion_fail': {
                    const step = stepItems.get(`${ev.wf}/${ev.step}`);
                    if (step) {
                        const message = new vscode.TestMessage(`${ev.crit} — expected: ${ev.expected}, actual: ${ev.actual}`);
                        message.expectedOutput = ev.expected;
                        message.actualOutput = ev.actual;
                        failed.add(step);
                        run.failed(step, message);
                    }
                    return;
                }
                case 'wf_done': {
                    const wf = wfItems.get(ev.wf);
                    if (!wf)
                        return;
                    // A step that ran without a failing criterion passed; one
                    // the runner never reached (it stops at the first failure)
                    // is skipped, not silently left pending.
                    wf.children.forEach((step) => {
                        if (failed.has(step))
                            return;
                        if (started.has(step) || ev.passed)
                            run.passed(step);
                        else
                            run.skipped(step);
                    });
                    if (ev.passed) {
                        run.passed(wf);
                    }
                    else {
                        run.failed(wf, new vscode.TestMessage(`workflow ${ev.wf} finished with failures`));
                    }
                    return;
                }
            }
        };
        const handle = (0, runner_1.spawnSuspectRunWith)(job.file, { baseUrl: job.baseUrl, cassette: job.cassette, filter, cwd: job.cwd }, onEvent, binary);
        const cancelSub = token.onCancellationRequested(() => handle.kill());
        try {
            await handle.done;
        }
        catch (err) {
            const message = new vscode.TestMessage((0, runner_1.errorMessage)(err));
            for (const wf of wfItems.values())
                run.errored(wf, message);
        }
        finally {
            cancelSub.dispose();
        }
    }
}
//# sourceMappingURL=testing.js.map