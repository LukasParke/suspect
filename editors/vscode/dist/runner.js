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
exports.isSuspectEvent = isSuspectEvent;
exports.suspectBinary = suspectBinary;
exports.testBaseUrl = testBaseUrl;
exports.gatewayPort = gatewayPort;
exports.spawnSuspectRun = spawnSuspectRun;
exports.errorMessage = errorMessage;
const cp = __importStar(require("child_process"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
function isSuspectEvent(value) {
    if (typeof value !== 'object' || value === null) {
        return false;
    }
    const tag = value.event;
    return typeof tag === 'string' && tag in SUSPECT_EVENT_TAGS;
}
const SUSPECT_EVENT_TAGS = {
    wf_started: true,
    step_started: true,
    request_sent: true,
    response_got: true,
    criterion_ok: true,
    criterion_fail: true,
    output_set: true,
    wf_done: true,
    run_done: true,
};
function config() {
    return vscode.workspace.getConfiguration('suspect');
}
/** Resolve the suspect CLI path from `suspect.basePath`. */
function suspectBinary() {
    const base = (config().get('basePath') ?? 'suspect').trim();
    if (!base || base === 'suspect') {
        return 'suspect';
    }
    if (/[/\\]suspect$/.test(base)) {
        return base;
    }
    return path.join(base, 'suspect');
}
function testBaseUrl() {
    return config().get('testBaseUrl') ?? 'http://localhost:8080';
}
function gatewayPort() {
    return config().get('gatewayPort') ?? 8080;
}
/**
 * Spawn `suspect test <arazzo> [--filter <id>] --base-url <url> --report ndjson`
 * and stream parsed TestEvents to `onEvent`.
 *
 * Exit codes 0 (pass) and 1 (failures) resolve with the run_done totals;
 * anything else (spawn failure, usage error) rejects.
 */
function spawnSuspectRun(arazzoPath, filter, onEvent) {
    const args = ['test', arazzoPath, '--base-url', testBaseUrl(), '--report', 'ndjson'];
    if (filter !== undefined) {
        args.push('--filter', filter);
    }
    let child;
    let killed = false;
    const done = new Promise((resolve, reject) => {
        try {
            child = cp.spawn(suspectBinary(), args, { stdio: ['ignore', 'pipe', 'pipe'] });
        }
        catch (err) {
            reject(err instanceof Error ? err : new Error(String(err)));
            return;
        }
        let stderrTail = '';
        child.stderr?.on('data', (chunk) => {
            stderrTail = (stderrTail + chunk.toString()).slice(-2000);
        });
        let buffer = '';
        let totals = { passed: 0, failed: 0 };
        child.stdout?.on('data', (chunk) => {
            buffer += chunk.toString();
            for (;;) {
                const nl = buffer.indexOf('\n');
                if (nl < 0) {
                    break;
                }
                const line = buffer.slice(0, nl).trim();
                buffer = buffer.slice(nl + 1);
                if (!line) {
                    continue;
                }
                let event;
                try {
                    event = JSON.parse(line);
                }
                catch {
                    continue; // non-NDJSON output on stdout
                }
                if (!isSuspectEvent(event)) {
                    continue;
                }
                if (event.event === 'run_done') {
                    totals = {
                        passed: Number(event.passed ?? 0),
                        failed: Number(event.failed ?? 0),
                    };
                }
                try {
                    onEvent(event);
                }
                catch {
                    // consumer errors must not kill the stream
                }
            }
        });
        child.on('error', (err) => reject(err));
        child.on('exit', (code, signal) => {
            if (killed || code === 0 || code === 1) {
                resolve(totals);
                return;
            }
            const detail = stderrTail.trim();
            reject(new Error(`suspect test exited with code ${code}${signal ? ` (${signal})` : ''}${detail ? `: ${detail}` : ''}`));
        });
    });
    return {
        done,
        kill() {
            killed = true;
            child?.kill('SIGTERM');
        },
    };
}
function errorMessage(err) {
    return err instanceof Error ? err.message : String(err);
}
//# sourceMappingURL=runner.js.map