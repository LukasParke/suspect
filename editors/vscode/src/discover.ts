// Workspace discovery: project manifests and Arazzo documents.
//
// Arazzo documents are recognised by content, not by file name. The
// `*.arazzo.yaml` convention is common but not required by the spec, and
// a project whose suites are `workflows/*.yaml` must not be invisible.

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { looksLikeArazzo, ProjectManifest, readProjectManifest } from './project';

const HEAD_BYTES = 2048;
/** Dependency trees and hidden directories (`.git`, `.suspect`, a tool's `.delta/worktrees`) are not the project. */
const SKIP = /(^|[\\/])(node_modules|\.[^\\/]+)[\\/]/;

/** True when the file sits in a skipped directory below its workspace folder. */
export function isSkipped(uri: vscode.Uri): boolean {
	return SKIP.test(vscode.workspace.asRelativePath(uri, false));
}

async function readHead(file: string): Promise<string> {
	const handle = await fs.promises.open(file, 'r');
	try {
		const buffer = Buffer.alloc(HEAD_BYTES);
		const { bytesRead } = await handle.read(buffer, 0, HEAD_BYTES, 0);
		return buffer.subarray(0, bytesRead).toString('utf8');
	} finally {
		await handle.close();
	}
}

/** Every Arazzo document in the workspace, sorted by path. Honours `files.exclude`. */
export async function findArazzoDocuments(): Promise<vscode.Uri[]> {
	const candidates = await vscode.workspace.findFiles('**/*.{yaml,yml}', undefined, 5000);
	const out: vscode.Uri[] = [];
	for (const uri of candidates) {
		if (isSkipped(uri)) continue;
		try {
			if (looksLikeArazzo(await readHead(uri.fsPath))) out.push(uri);
		} catch {
			// unreadable between findFiles and read — skip
		}
	}
	return out.sort((a, b) => a.fsPath.localeCompare(b.fsPath));
}

export interface DiscoveredProject {
	manifestUri: vscode.Uri;
	dir: string;
	manifest: ProjectManifest;
}

/** Every readable `suspect.project.json` in the workspace, sorted by path. */
export async function findProjects(): Promise<DiscoveredProject[]> {
	const uris = await vscode.workspace.findFiles('**/suspect.project.json', '**/node_modules/**');
	const out: DiscoveredProject[] = [];
	for (const manifestUri of uris.sort((a, b) => a.fsPath.localeCompare(b.fsPath))) {
		if (isSkipped(manifestUri)) continue;
		try {
			const manifest = readProjectManifest(await fs.promises.readFile(manifestUri.fsPath, 'utf8'));
			if (manifest !== undefined) {
				out.push({ manifestUri, dir: path.dirname(manifestUri.fsPath), manifest });
			}
		} catch {
			// unreadable — skip
		}
	}
	return out;
}

/** True when saving this document can change what the project or testing surfaces show. */
export function affectsProject(document: vscode.TextDocument): boolean {
	const file = document.uri.fsPath;
	if (/suspect\.project\.json$|[\\/]\.suspect\.ya?ml$/i.test(file)) return true;
	return /\.ya?ml$/i.test(file) && looksLikeArazzo(document.getText(new vscode.Range(0, 0, 40, 0)));
}
