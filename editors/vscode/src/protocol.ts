/**
 * The suspect-specific half of the LSP handshake.
 *
 * The client declares in `capabilities.experimental.suspect` which server
 * command ids it owns (it registers palette/editor handlers for them), and
 * the server leaves those out of `executeCommandProvider`, so the language
 * client does not register them a second time. The server answers with
 * `capabilities.experimental.suspect`: its custom requests and commands,
 * each with a version, so a feature can be checked instead of tried.
 */
import type { ClientCapabilities, FeatureState, LanguageClient, StaticFeature } from 'vscode-languageclient/node';
import { EDITOR_OWNED_COMMANDS } from './serverCommands';

/** What the client puts under `capabilities.experimental.suspect`. */
export interface ClientProtocol {
	clientCommands: string[];
}

/** The server's `capabilities.experimental.suspect`. */
export interface ServerProtocol {
	protocol: number;
	requests: Record<string, number>;
	commands: Record<string, number>;
}

export function clientProtocol(): ClientProtocol {
	return { clientCommands: [...EDITOR_OWNED_COMMANDS] };
}

/** Fills `experimental.suspect` in the client capabilities; register it before `start()`. */
export function suspectCapabilities(): StaticFeature {
	return {
		fillClientCapabilities(capabilities: ClientCapabilities): void {
			const experimental = (capabilities.experimental ?? {}) as Record<string, unknown>;
			capabilities.experimental = { ...experimental, suspect: clientProtocol() };
		},
		initialize(): void {},
		getState: (): FeatureState => ({ kind: 'static' }),
		clear(): void {},
	};
}

/**
 * The server's advertisement, or `undefined` for a server that predates it
 * (protocol 0: `suspect/runWorkflow`, `suspect/generationContract` and
 * `suspect/renderSdk`, nothing newer).
 */
export function serverProtocol(client: LanguageClient | undefined): ServerProtocol | undefined {
	const experimental = client?.initializeResult?.capabilities.experimental as { suspect?: ServerProtocol } | undefined;
	return experimental?.suspect;
}

/** The advertised version of a custom request, or `undefined` when the server does not offer it. */
export function requestVersion(client: LanguageClient | undefined, method: string): number | undefined {
	return serverProtocol(client)?.requests?.[method];
}
