/**
 * The server-side commands, given a VS Code surface: the language server
 * implements them; these handlers give them entry points the editor can
 * reach (palette, editor context) and results a human can read.
 */
import * as vscode from 'vscode';
import type { LanguageClient } from 'vscode-languageclient/node';

type ClientGetter = () => LanguageClient | undefined;

/** Sends one `workspace/executeCommand` to the running server. */
async function serverCommand(getClient: ClientGetter, command: string, args: unknown[]): Promise<Record<string, unknown> | undefined> {
	const client = getClient();
	if (!client) {
		void vscode.window.showWarningMessage('Suspect language server is not running.');
		return undefined;
	}
	return client.sendRequest<Record<string, unknown>>('workspace/executeCommand', { command, arguments: args });
}

/** The active editor's document, when it is a file the server can have. */
function activeFile(): vscode.TextDocument | undefined {
	const editor = vscode.window.activeTextEditor;
	if (!editor || editor.document.uri.scheme !== 'file') {
		return undefined;
	}
	return editor.document;
}

function requireActiveFile(): vscode.TextDocument | undefined {
	const doc = activeFile();
	if (!doc) {
		void vscode.window.showWarningMessage('Open a file the Suspect server can see.');
	}
	return doc;
}

/** Shows a JSON result the way a human reads it: message first, then the document. */
async function showResultJson(result: Record<string, unknown> | undefined, title: string): Promise<void> {
	if (!result) {
		return;
	}
	const doc = await vscode.workspace.openTextDocument({
		content: `${JSON.stringify(result, null, 2)}\n`,
		language: 'json',
	});
	void vscode.window.showTextDocument(doc, { preview: true });
	void vscode.window.setStatusBarMessage(`${title} — result opened`, 4000);
}

/** Registers every server-command surface the extension owns. */
export function registerServerCommands(getClient: ClientGetter): vscode.Disposable[] {
	return [
		// --- workspace reports ---
		vscode.commands.registerCommand('suspect.showRefGraph', async () => {
			const result = await serverCommand(getClient, 'suspect.showRefGraph', []);
			const mermaid = result?.mermaid;
			if (typeof mermaid !== 'string') {
				return;
			}
			const doc = await vscode.workspace.openTextDocument({ content: `\`\`\`mermaid\n${mermaid}\n\`\`\`\n`, language: 'markdown' });
			void vscode.window.showTextDocument(doc, { preview: true });
		}),
		vscode.commands.registerCommand('suspect.breakingChanges', async () => {
			const result = await serverCommand(getClient, 'suspect.breakingChanges', []);
			const count = result?.breaking_changes;
			if (count === undefined) {
				return;
			}
			const message = `Suspect: ${count} breaking change(s) vs ${process.env.SUSPECT_GIT_BASE ?? 'HEAD~1'}. Details in the Suspect output.`;
			if ((count as number) > 0) {
				void vscode.window.showWarningMessage(message);
			} else {
				void vscode.window.showInformationMessage(message);
			}
		}),
		vscode.commands.registerCommand('suspect.contractCoverage', async () => {
			const result = await serverCommand(getClient, 'suspect.contractCoverage', []);
			if (result?.operations === undefined) {
				return;
			}
			const { operations, uncovered } = result as { operations: number; uncovered: number };
			const covered = operations - uncovered;
			void vscode.window.showInformationMessage(
				`Suspect: ${covered}/${operations} operations covered by workflows; ${uncovered} uncovered.`,
			);
		}),
		vscode.commands.registerCommand('suspect.runService', async () => {
			const result = await serverCommand(getClient, 'suspect.runService', []);
			await showResultJson(result, 'Service gate');
		}),

		// --- per-document checks ---
		vscode.commands.registerCommand('suspect.verifyContract', async () => {
			const doc = requireActiveFile();
			if (!doc) {
				return;
			}
			const result = await serverCommand(getClient, 'suspect.verifyContract', [doc.uri.fsPath]);
			if (result?.current === undefined) {
				return;
			}
			if (result.current) {
				void vscode.window.showInformationMessage('Suspect: contract package is current.');
			} else {
				void vscode.window
					.showWarningMessage('Suspect: contract package is stale — run `suspect contract`.', 'Run Now')
					.then((choice) => {
						if (choice === 'Run Now') {
							const terminal = vscode.window.createTerminal('suspect contract');
							terminal.sendText('suspect contract');
							terminal.show();
						}
					});
			}
		}),
		vscode.commands.registerCommand('suspect.changeImpact', async () => {
			const editor = vscode.window.activeTextEditor;
			if (!editor) {
				return;
			}
			const offset = docOffset(editor);
			const result = await serverCommand(getClient, 'suspect.changeImpact', [
				editor.document.uri.toString(),
				offset,
			]);
			const summary = result?.summary;
			if (typeof summary !== 'string') {
				return;
			}
			void vscode.window.showInformationMessage(`Suspect: ${summary}`, 'Full Report').then((choice) => {
				if (choice === 'Full Report') {
					void showResultJson(result, 'Change impact');
				}
			});
		}),

		// --- refactors: the server applies the edits ---
		vscode.commands.registerCommand('suspect.generateExample', async () => {
			const editor = vscode.window.activeTextEditor;
			if (!editor) {
				return;
			}
			const result = await serverCommand(getClient, 'suspect.generateExample', [
				editor.document.uri.toString(),
				editor.selection.active.line,
				editor.selection.active.character,
			]);
			if (result && typeof result.yaml === 'string') {
				void vscode.window.setStatusBarMessage('Suspect: example inserted', 4000);
			}
		}),
		vscode.commands.registerCommand('suspect.extractSchema', async () => {
			const editor = vscode.window.activeTextEditor;
			if (!editor) {
				return;
			}
			const name = await vscode.window.showInputBox({
				prompt: 'Component schema name (blank: derive from the selection)',
				placeHolder: 'PetSchema',
			});
			if (name === undefined) {
				return;
			}
			const args = [editor.document.uri.toString(), docOffset(editor)];
			if (name.length > 0) {
				args.push(name);
			}
			await serverCommand(getClient, 'suspect.extractSchema', args);
		}),
		vscode.commands.registerCommand('suspect.inlineSchema', async () => {
			const editor = vscode.window.activeTextEditor;
			if (!editor) {
				return;
			}
			await serverCommand(getClient, 'suspect.inlineSchema', [
				editor.document.uri.toString(),
				docOffset(editor),
			]);
		}),
		vscode.commands.registerCommand('suspect.renderPreview', async () => {
			const doc = requireActiveFile();
			if (!doc) {
				return;
			}
			const preset = await vscode.window.showQuickPick(
				[{ label: 'docs-md', description: 'Markdown documentation' }],
				{ placeHolder: 'Preview preset' },
			);
			if (!preset) {
				return;
			}
			await serverCommand(getClient, 'suspect.renderPreview', [doc.uri.toString(), preset.label]);
		}),
	];
}

/** UTF-16 offset of the cursor — the position the server's refactor plans anchor on. */
function docOffset(editor: vscode.TextEditor): number {
	return editor.document.offsetAt(editor.selection.active);
}

/**
 * The admission review for the document about to be generated from:
 * `suspect/generationContract` on the server, mapped to a go/no-go.
 */
export async function generationContract(
	getClient: ClientGetter,
	uri: vscode.Uri,
): Promise<{ admissible: boolean; refusals: number; findings: unknown[] } | undefined> {
	const client = getClient();
	if (!client) {
		return undefined;
	}
	const result = await client.sendRequest<{ admissible?: boolean; refusals?: number; findings?: unknown[] }>(
		'suspect/generationContract',
		{ uri: uri.toString() },
	);
	if (result?.admissible === undefined) {
		return undefined;
	}
	return {
		admissible: result.admissible,
		refusals: result.refusals ?? 0,
		findings: result.findings ?? [],
	};
}
