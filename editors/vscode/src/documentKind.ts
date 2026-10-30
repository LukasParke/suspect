/**
 * The `suspect.documentKind` context key: what the server takes the active
 * document to be (`openapi`, `fragment`, `none`, ...), from its
 * `suspect/documentKind` request. Menu `when` clauses use it so spec-only
 * entries stay off `docker-compose.yml` and `package.json`. The server
 * gates its own features by the same value, so the two agree.
 */
import * as vscode from 'vscode';
import type { LanguageClient } from 'vscode-languageclient/node';
import { requestVersion } from './protocol';

export const DOCUMENT_KIND_REQUEST = 'suspect/documentKind';
export const DOCUMENT_KIND_CONTEXT = 'suspect.documentKind';

type ClientGetter = () => LanguageClient | undefined;

/** The server's kind for `uri`; `undefined` when there is no server, it predates the request, or the request fails. */
export async function documentKind(client: LanguageClient | undefined, uri: vscode.Uri): Promise<string | undefined> {
	if (!client || uri.scheme !== 'file' || requestVersion(client, DOCUMENT_KIND_REQUEST) === undefined) {
		return undefined;
	}
	try {
		const result = await client.sendRequest<{ kind?: unknown }>(DOCUMENT_KIND_REQUEST, { uri: uri.toString() });
		return typeof result?.kind === 'string' ? result.kind : undefined;
	} catch {
		return undefined;
	}
}

export interface DocumentKindTracker {
	disposables: vscode.Disposable[];
	/** Re-reads the active editor's kind (after the client starts). */
	refresh(): Promise<void>;
}

/** Keeps the context key on the active editor's kind: on editor switches and saves. */
export function trackDocumentKind(getClient: ClientGetter): DocumentKindTracker {
	let latest = 0;
	const refresh = async (): Promise<void> => {
		const ticket = ++latest;
		const document = vscode.window.activeTextEditor?.document;
		const kind = document ? await documentKind(getClient(), document.uri) : undefined;
		// A slower answer for an editor the user already left must not win.
		if (ticket === latest) {
			await vscode.commands.executeCommand('setContext', DOCUMENT_KIND_CONTEXT, kind);
		}
	};
	return {
		disposables: [
			vscode.window.onDidChangeActiveTextEditor(() => void refresh()),
			// Typing `openapi:` into a new file changes its kind; a save is
			// when the server's view of it settles.
			vscode.workspace.onDidSaveTextDocument((saved) => {
				if (saved === vscode.window.activeTextEditor?.document) void refresh();
			}),
		],
		refresh,
	};
}
