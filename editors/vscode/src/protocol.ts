/**
 * The suspect-specific half of the LSP handshake.
 *
 * The client declares in `capabilities.experimental.suspect` which server
 * command ids it owns (it registers palette/editor handlers for them), and
 * the server leaves those out of `executeCommandProvider`, so the language
 * client does not register them a second time. The server answers with
 * `capabilities.experimental.suspect`: its custom requests and commands,
 * each with a version, so a feature can be checked instead of tried. The
 * client also reports its own version, and the server warns once when it
 * belongs to another release.
 */
import type { ClientCapabilities, FeatureState, LanguageClient, StaticFeature } from 'vscode-languageclient/node';
import { EDITOR_OWNED_COMMANDS } from './serverCommands';

/** What the client puts under `capabilities.experimental.suspect`. */
export interface ClientProtocol {
	clientCommands: string[];
	extensionVersion?: string;
}

/** The server's `capabilities.experimental.suspect`. */
export interface ServerProtocol {
	protocol: number;
	requests: Record<string, number>;
	commands: Record<string, number>;
}

export function clientProtocol(extensionVersion?: string): ClientProtocol {
	return { clientCommands: [...EDITOR_OWNED_COMMANDS], ...(extensionVersion && { extensionVersion }) };
}

/** Fills `experimental.suspect` in the client capabilities; register it before `start()`. */
export function suspectCapabilities(extensionVersion?: string): StaticFeature {
	return {
		fillClientCapabilities(capabilities: ClientCapabilities): void {
			const experimental = (capabilities.experimental ?? {}) as Record<string, unknown>;
			capabilities.experimental = { ...experimental, suspect: clientProtocol(extensionVersion) };
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

/**
 * The oldest CLI that has the grouped command tree this extension spawns
 * (`gen sdk`, `gen session`, `traffic gateway`, `test run`, ...).
 */
export const MIN_CLI_VERSION = '0.1.0';

function parseVersion(version: string): number[] | undefined {
	const match = /^v?(\d+)\.(\d+)\.(\d+)/.exec(version.trim());
	return match ? match.slice(1, 4).map(Number) : undefined;
}

/**
 * `true` when `version` parses and is older than `minimum`. A version that
 * does not parse is not reported: the check exists to name a known-old build.
 */
export function olderThan(version: string | undefined, minimum: string = MIN_CLI_VERSION): boolean {
	const have = version === undefined ? undefined : parseVersion(version);
	const need = parseVersion(minimum);
	if (!have || !need) return false;
	for (let i = 0; i < 3; i++) {
		if (have[i] !== need[i]) return have[i] < need[i];
	}
	return false;
}

/** The server's `serverInfo.version` when it is older than {@link MIN_CLI_VERSION}. */
export function tooOldServer(client: LanguageClient | undefined): string | undefined {
	const version = client?.initializeResult?.serverInfo?.version;
	return olderThan(version) ? version : undefined;
}
