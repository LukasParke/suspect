"use strict";
// The findings/status surface: one status-bar item summarising the
// specification's health, one place to jump from it.
//
// The counts come from the language client's published diagnostics, so
// the item is live: it moves as findings are fixed, and clicking it opens
// the Overview view where each severity is navigable.
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
exports.SEVERITY_ORDER = void 0;
exports.countSeverities = countSeverities;
exports.statusText = statusText;
exports.registerStatus = registerStatus;
exports.updateStatus = updateStatus;
const vscode = __importStar(require("vscode"));
exports.SEVERITY_ORDER = [
    { key: 'error', label: 'Errors', icon: '$(error)' },
    { key: 'warning', label: 'Warnings', icon: '$(warning)' },
    { key: 'information', label: 'Information', icon: '$(info)' },
    { key: 'hint', label: 'Hints', icon: '$(lightbulb)' },
];
/** Counts a diagnostic list by severity. Pure: unit-tested without VS Code. */
function countSeverities(diagnostics) {
    const out = { error: 0, warning: 0, information: 0, hint: 0 };
    for (const diagnostic of diagnostics) {
        switch (diagnostic.severity) {
            case 0:
                out.error += 1;
                break;
            case 1:
                out.warning += 1;
                break;
            case 2:
                out.information += 1;
                break;
            case 3:
                out.hint += 1;
                break;
            default: break;
        }
    }
    return out;
}
/** The status-bar text for a count: leading with what blocks work first. */
function statusText(counts) {
    const parts = [];
    if (counts.error > 0)
        parts.push(`$(error) ${counts.error}`);
    if (counts.warning > 0)
        parts.push(`$(warning) ${counts.warning}`);
    if (parts.length === 0)
        return `$(check) 0 problems`;
    return `Suspect ${parts.join('  ')}`;
}
let item;
function registerStatus(context) {
    item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 95);
    item.name = 'Suspect Findings';
    item.command = 'suspect.showOverview';
    item.tooltip = 'Suspect findings — click to open the Overview';
    context.subscriptions.push(item);
    item.show();
    const refresh = () => updateStatus();
    context.subscriptions.push(vscode.languages.onDidChangeDiagnostics(refresh));
    refresh();
}
function updateStatus() {
    const all = vscode.languages.getDiagnostics();
    let flat = [];
    for (const [, list] of all) {
        for (const d of list) {
            // Only what the suspect server published: markers carry the
            // source we set on every finding.
            if (d.source === 'suspect' || d.source === 'suspect-lint') {
                flat.push(d);
            }
        }
    }
    const counts = countSeverities(flat);
    if (item !== undefined) {
        item.text = statusText(counts);
        item.tooltip = new vscode.MarkdownString(`**Suspect** — ${counts.error} errors, ${counts.warning} warnings, ${counts.information} info, ${counts.hint} hints\n\nClick to open the Overview`);
    }
    return counts;
}
//# sourceMappingURL=status.js.map