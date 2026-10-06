"use strict";
// Workspace discovery: project manifests and Arazzo documents.
//
// Arazzo documents are recognised by content, not by file name. The
// `*.arazzo.yaml` convention is common but not required by the spec, and
// a project whose suites are `workflows/*.yaml` must not be invisible.
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
exports.isSkipped = isSkipped;
exports.findArazzoDocuments = findArazzoDocuments;
exports.findProjects = findProjects;
exports.affectsProject = affectsProject;
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const project_1 = require("./project");
const HEAD_BYTES = 2048;
/** Dependency trees and hidden directories (`.git`, `.suspect`, a tool's `.delta/worktrees`) are not the project. */
const SKIP = /(^|[\\/])(node_modules|\.[^\\/]+)[\\/]/;
/** True when the file sits in a skipped directory below its workspace folder. */
function isSkipped(uri) {
    return SKIP.test(vscode.workspace.asRelativePath(uri, false));
}
async function readHead(file) {
    const handle = await fs.promises.open(file, 'r');
    try {
        const buffer = Buffer.alloc(HEAD_BYTES);
        const { bytesRead } = await handle.read(buffer, 0, HEAD_BYTES, 0);
        return buffer.subarray(0, bytesRead).toString('utf8');
    }
    finally {
        await handle.close();
    }
}
/** Every Arazzo document in the workspace, sorted by path. Honours `files.exclude`. */
async function findArazzoDocuments() {
    const candidates = await vscode.workspace.findFiles('**/*.{yaml,yml}', undefined, 5000);
    const out = [];
    for (const uri of candidates) {
        if (isSkipped(uri))
            continue;
        try {
            if ((0, project_1.looksLikeArazzo)(await readHead(uri.fsPath)))
                out.push(uri);
        }
        catch {
            // unreadable between findFiles and read — skip
        }
    }
    return out.sort((a, b) => a.fsPath.localeCompare(b.fsPath));
}
/** Every readable `suspect.project.json` in the workspace, sorted by path. */
async function findProjects() {
    const uris = await vscode.workspace.findFiles('**/suspect.project.json', '**/node_modules/**');
    const out = [];
    for (const manifestUri of uris.sort((a, b) => a.fsPath.localeCompare(b.fsPath))) {
        if (isSkipped(manifestUri))
            continue;
        try {
            const manifest = (0, project_1.readProjectManifest)(await fs.promises.readFile(manifestUri.fsPath, 'utf8'));
            if (manifest !== undefined) {
                out.push({ manifestUri, dir: path.dirname(manifestUri.fsPath), manifest });
            }
        }
        catch {
            // unreadable — skip
        }
    }
    return out;
}
/** True when saving this document can change what the project or testing surfaces show. */
function affectsProject(document) {
    const file = document.uri.fsPath;
    if (/suspect\.project\.json$|[\\/]\.suspect\.ya?ml$/i.test(file))
        return true;
    return /\.ya?ml$/i.test(file) && (0, project_1.looksLikeArazzo)(document.getText(new vscode.Range(0, 0, 40, 0)));
}
//# sourceMappingURL=discover.js.map