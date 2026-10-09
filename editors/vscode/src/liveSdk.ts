/**
 * Live SDK preview: renders a native SDK from the current buffer through
 * the server's `suspect/renderSdk` request, and re-renders on every
 * document change (debounced) — the server's incremental reparse keeps
 * its tree current on every keystroke, so each refresh costs the compile,
 * not the keystrokes that preceded it.
 */
import * as path from 'path';
import * as vscode from 'vscode';
import type { LanguageClient } from 'vscode-languageclient/node';
import { availableSdkProfiles, SdkProfile } from './generation';
import { errorMessage, suspectBinary } from './runner';

type ClientGetter = () => LanguageClient | undefined;

interface RenderedArtifact {
	path: string;
	content: string;
}

interface RenderSdkResult {
	rendered: boolean;
	artifacts: RenderedArtifact[];
	diagnostics: { code: string; message: string }[];
	operations: number;
}

const LIVE_SDK_SCHEME = 'suspect-live-sdk';
const REFRESH_DEBOUNCE_MS = 400;

/** One open live preview. Refreshes render through the shared provider. */
class LiveSdkPreview implements vscode.TextDocumentContentProvider {
	readonly onDidChange: vscode.Event<vscode.Uri>;
	private readonly artifacts = new Map<string, string>();
	private watcher: vscode.Disposable | undefined;
	private pending: NodeJS.Timeout | undefined;
	private lastOpened: string | undefined;

	constructor(
		private readonly getClient: ClientGetter,
		private readonly emitter: vscode.EventEmitter<vscode.Uri>,
		private readonly context: {
			uri: vscode.Uri;
			profile: SdkProfile;
			packageName: string;
			packageVersion: string;
			importName?: string;
			operationIds: readonly string[];
		},
	) {
		this.onDidChange = emitter.event;
	}

	provideTextDocumentContent(uri: vscode.Uri): string {
		return this.artifacts.get(uri.path) ?? `No artifact at ${uri.path}. Use Suspect: Live SDK Preview.`;
	}

	/** One render pass; artifacts land in the provider, open documents refresh. */
	async refresh(): Promise<void> {
		const client = this.getClient();
		if (!client) return;
		try {
			const result = await client.sendRequest<RenderSdkResult>('suspect/renderSdk', {
				uri: this.context.uri.toString(),
				profile: this.context.profile,
				packageName: this.context.packageName,
				packageVersion: this.context.packageVersion,
				importName: this.context.importName,
				operationIds: this.context.operationIds,
			});
			if (!result.rendered) {
				const first = result.diagnostics[0]?.message ?? 'the contract refused to plan';
				void vscode.window.showWarningMessage(`Suspect live SDK: ${first}`, 'Show All').then((choice) => {
					if (choice === 'Show All') {
						const output = vscode.window.createOutputChannel('Suspect Live SDK');
						output.appendLine(result.diagnostics.map((d) => `${d.code}: ${d.message}`).join('\n'));
						output.show(true);
					}
				});
				return;
			}
			const previous = new Map(this.artifacts);
			this.artifacts.clear();
			for (const artifact of result.artifacts) {
				this.artifacts.set(`/${artifact.path}`, artifact.content);
			}
			// Refresh every open preview tab whose content changed.
			for (const open of this.artifacts) {
				if (previous.get(open[0]) !== open[1]) {
					this.emitter.fire(this.uriFor(open[0].slice(1)));
				}
			}
			// First render: open the README (or first artifact).
			if (!this.lastOpened) {
				const lead =
					result.artifacts.find((a) => path.basename(a.path).toLowerCase() === 'readme.md') ??
					result.artifacts[0];
				if (lead) {
					this.lastOpened = `/${lead.path}`;
					void vscode.window.showTextDocument(this.uriFor(lead.path), { preview: true });
				}
			}
			void vscode.window.setStatusBarMessage(
				`Suspect live SDK: ${result.operations} operations · ${result.artifacts.length} artifacts`,
				4000,
			);
		} catch (error) {
			void vscode.window.showWarningMessage(`Suspect live SDK: ${errorMessage(error)}`);
		}
	}

	private uriFor(artifactPath: string): vscode.Uri {
		return vscode.Uri.from({ scheme: LIVE_SDK_SCHEME, path: `/${artifactPath}` });
	}

	/** Follows the source document: every change re-renders, debounced. */
	start(): void {
		this.watcher?.dispose();
		this.watcher = vscode.workspace.onDidChangeTextDocument((event) => {
			if (event.document.uri.toString() !== this.context.uri.toString()) return;
			if (this.pending) clearTimeout(this.pending);
			this.pending = setTimeout(() => {
				this.pending = undefined;
				void this.refresh();
			}, REFRESH_DEBOUNCE_MS);
			this.pending.unref?.();
		});
		void this.refresh();
	}

	dispose(): void {
		this.watcher?.dispose();
		if (this.pending) clearTimeout(this.pending);
		this.artifacts.clear();
	}
}

let active: LiveSdkPreview | undefined;

/** Registers the live-preview commands and the scheme they serve. */
export function registerLiveSdkPreview(getClient: ClientGetter): vscode.Disposable[] {
	const emitter = new vscode.EventEmitter<vscode.Uri>();
	const provider = vscode.workspace.registerTextDocumentContentProvider(LIVE_SDK_SCHEME, {
		onDidChange: emitter.event,
		provideTextDocumentContent: (uri: vscode.Uri) =>
			active?.provideTextDocumentContent(uri) ?? 'Start Suspect: Live SDK Preview to see artifacts.',
	});
	return [
		provider,
		vscode.commands.registerCommand('suspect.liveSdkPreview', async () => {
			const editor = vscode.window.activeTextEditor;
			if (!editor || editor.document.uri.scheme !== 'file') {
				void vscode.window.showWarningMessage('Open the specification to preview its SDK.');
				return;
			}
			let profiles;
			try {
				profiles = await availableSdkProfiles(suspectBinary());
			} catch (error) {
				void vscode.window.showErrorMessage(`Suspect profile discovery failed: ${errorMessage(error)}`);
				return;
			}
			const pick = await vscode.window.showQuickPick(
				profiles.map((p) => ({ label: p.profile, description: p.description })),
				{ placeHolder: 'Suspect: live SDK preview — choose a profile' },
			);
			if (!pick) return;
			const config = vscode.workspace.getConfiguration('suspect', editor.document.uri);
			const packageName = config.get<string>('sdk.packageName', '');
			if (!packageName) {
				void vscode.window.showWarningMessage('Set suspect.sdk.packageName to identify the preview package.');
				return;
			}
			active?.dispose();
			active = new LiveSdkPreview(getClient, emitter, {
				uri: editor.document.uri,
				profile: pick.label as SdkProfile,
				packageName,
				packageVersion: config.get<string>('sdk.packageVersion', '0.1.0'),
				importName: config.get<string>('sdk.importNames') || undefined,
				operationIds: config.get<string[]>('sdk.operationIds', []),
			});
			active.start();
		}),
		vscode.commands.registerCommand('suspect.stopLiveSdkPreview', () => {
			active?.dispose();
			active = undefined;
		}),
	];
}
