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
exports.registerTestExplorer = registerTestExplorer;
const fs = __importStar(require("fs"));
const vscode = __importStar(require("vscode"));
const parse_1 = require("./parse");
const runner_1 = require("./runner");
const nodeDataByItem = new WeakMap();
function nodeData(item) {
    return nodeDataByItem.get(item);
}
function registerTestExplorer(context) {
    const controller = vscode.tests.createTestController('suspect.tests', 'Suspect Workflows');
    controller.resolveHandler = async (item) => {
        if (!item) {
            await refresh(controller);
        }
    };
    controller.createRunProfile('Suspect Run', vscode.TestRunProfileKind.Run, runHandler.bind(undefined, controller), true);
    context.subscriptions.push(controller, vscode.workspace.onDidChangeWorkspaceFolders(() => void refresh(controller)), vscode.workspace.onDidSaveTextDocument((doc) => {
        if (/\.arazzo\.ya?ml$/i.test(doc.uri.fsPath)) {
            void refresh(controller);
        }
    }), vscode.commands.registerCommand('suspect.tests.refresh', () => void refresh(controller)));
    async function refresh(controller) {
        const items = [];
        const uris = await vscode.workspace.findFiles('**/*.arazzo.{yaml,yml}', '**/node_modules/**');
        for (const uri of uris.sort((a, b) => a.fsPath.localeCompare(b.fsPath))) {
            let parsed;
            try {
                parsed = (0, parse_1.parseArazzo)(await fs.promises.readFile(uri.fsPath, 'utf8'));
            }
            catch {
                continue; // unreadable/deleted between findFiles and read
            }
            const fileItem = controller.createTestItem(uri.toString(), vscode.workspace.asRelativePath(uri), uri);
            nodeDataByItem.set(fileItem, { kind: 'file', file: uri });
            fileItem.canResolveChildren = false;
            for (const wf of parsed) {
                const wfItem = controller.createTestItem(`${uri.toString()}\u0000${wf.workflowId}`, wf.workflowId, uri);
                nodeDataByItem.set(wfItem, { kind: 'workflow', file: uri, workflowId: wf.workflowId });
                wfItem.range = new vscode.Range(wf.line, 0, wf.line, 0);
                for (const step of wf.steps) {
                    const stepItem = controller.createTestItem(`${uri.toString()}\u0000${wf.workflowId}\u0000${step.stepId}`, step.stepId, uri);
                    nodeDataByItem.set(stepItem, {
                        kind: 'step',
                        file: uri,
                        workflowId: wf.workflowId,
                        stepId: step.stepId,
                    });
                    stepItem.range = new vscode.Range(step.line, 0, step.line, 0);
                }
                fileItem.children.add(wfItem);
            }
            items.push(fileItem);
        }
        controller.items.replace(items);
    }
    async function runHandler(controller, request, token) {
        const run = controller.createTestRun(request);
        const jobs = new Map();
        const visit = (item, wfItem, wf) => {
            const data = nodeData(item);
            if (!data) {
                return;
            }
            switch (data.kind) {
                case 'file':
                    item.children.forEach((child) => visit(child));
                    return;
                case 'workflow':
                    item.children.forEach((child) => visit(child, item, data));
                    return;
                case 'step': {
                    const key = `${data.file.fsPath}\u0000${data.workflowId}`;
                    let job = jobs.get(key);
                    if (!job) {
                        job = { file: data.file, workflowId: data.workflowId, wfItem, steps: new Map() };
                        jobs.set(key, job);
                    }
                    job.steps.set(data.stepId, { item, ended: false });
                    run.enqueued(item);
                    return;
                }
            }
        };
        if (request.include && request.include.length > 0) {
            request.include.forEach((item) => visit(item));
        }
        else {
            controller.items.forEach((item) => visit(item));
        }
        try {
            for (const job of jobs.values()) {
                if (token.isCancellationRequested) {
                    break;
                }
                await executeWorkflow(run, job, token);
            }
        }
        finally {
            run.end();
        }
    }
    function executeWorkflow(run, job, token) {
        if (job.wfItem) {
            run.started(job.wfItem);
        }
        const finishStep = (stepId, outcome, message) => {
            const entry = job.steps.get(stepId);
            if (!entry || entry.ended) {
                return;
            }
            switch (outcome) {
                case 'passed':
                    run.passed(entry.item);
                    return;
                case 'failed':
                    run.failed(entry.item, message ?? new vscode.TestMessage(`step ${stepId} failed`));
                    return;
                case 'errored':
                    run.errored(entry.item, message ?? new vscode.TestMessage(`step ${stepId} could not run`));
                    return;
            }
        };
        const onEvent = (ev) => {
            switch (ev.event) {
                case 'step_started': {
                    const entry = job.steps.get(ev.step);
                    if (entry && !entry.ended) {
                        run.started(entry.item);
                    }
                    return;
                }
                case 'criterion_fail':
                    finishStep(ev.step, 'failed', new vscode.TestMessage(`${ev.crit} — expected: ${ev.expected}, actual: ${ev.actual}`));
                    return;
                case 'wf_done':
                    for (const stepId of job.steps.keys()) {
                        finishStep(stepId, ev.passed ? 'passed' : 'failed');
                    }
                    if (job.wfItem) {
                        if (ev.passed) {
                            run.passed(job.wfItem);
                        }
                        else {
                            run.failed(job.wfItem, new vscode.TestMessage(`workflow ${job.workflowId} finished with failures`));
                        }
                    }
                    return;
            }
        };
        const handle = (0, runner_1.spawnSuspectRun)(job.file.fsPath, job.workflowId, onEvent);
        const cancelSub = token.onCancellationRequested(() => handle.kill());
        return handle.done
            .catch(async (err) => {
            const msg = new vscode.TestMessage((0, runner_1.errorMessage)(err));
            if (job.wfItem) {
                run.errored(job.wfItem, msg);
            }
            for (const [stepId, entry] of job.steps) {
                if (!entry.ended) {
                    run.errored(entry.item, msg);
                    entry.ended = true;
                }
            }
            await vscode.window.showErrorMessage(`Suspect could not run ${job.workflowId}: ${(0, runner_1.errorMessage)(err)}`);
        })
            .finally(() => cancelSub.dispose())
            .then(() => undefined);
    }
}
//# sourceMappingURL=testExplorer.js.map