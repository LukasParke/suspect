import * as cp from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import * as settings from './settings.generated';

/**
 * NDJSON TestEvent stream emitted by `suspect test --report ndjson`.
 * Mirrors `suspect_test::exec::TestEvent` (serde tag "event", snake_case).
 */
export type SuspectEvent =
	| { event: 'wf_started'; id: string }
	| { event: 'step_started'; wf: string; step: string }
	| { event: 'request_sent'; wf: string; step: string; method: string; url: string }
	| { event: 'response_got'; wf: string; step: string; status: number; duration_ms: number }
	| { event: 'criterion_ok'; wf: string; step: string; crit: string }
	| { event: 'criterion_fail'; wf: string; step: string; crit: string; expected: string; actual: string }
	| { event: 'output_set'; wf: string; key: string; value: unknown }
	| { event: 'wf_done'; wf: string; passed: boolean }
	| { event: 'run_done'; passed: number; failed: number };

export function isSuspectEvent(value: unknown): value is SuspectEvent {
	if (typeof value !== 'object' || value === null) {
		return false;
	}
	const tag = (value as { event?: unknown }).event;
	return typeof tag === 'string' && tag in SUSPECT_EVENT_TAGS;
}

const SUSPECT_EVENT_TAGS: Record<string, true> = {
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

export interface RunTotals {
	passed: number;
	failed: number;
}

export interface SuspectRunHandle {
	done: Promise<RunTotals>;
	kill(): void;
}

/**
 * Settings that select the CLI binary: `suspect.basePath`, plus the
 * deprecated `suspect.server.path` from the retired legacy extension.
 */
export const BINARY_SETTINGS: readonly string[] = ['suspect.basePath', 'suspect.server.path'];

/**
 * The configured CLI location: `suspect.basePath` when set to anything but
 * its default, else the deprecated `suspect.server.path` alias, else bare
 * `suspect` (resolved from $PATH).
 */
function configuredBase(): string {
	const base = settings.basePath().trim();
	if (base && base !== 'suspect') return base;
	const legacy = settings.serverPath().trim();
	if (legacy) return legacy;
	return base || 'suspect';
}

/** Resolve the suspect CLI path from `suspect.basePath` (or its deprecated alias). */
export function suspectBinary(): string {
	const base = configuredBase();
	if (!base || base === 'suspect') {
		return 'suspect';
	}
	if (/[/\\]suspect$/.test(base)) {
		return base;
	}
	try {
		// Explicit executables may be versioned, renamed, or symlinks to a pinned
		// build. The setting accepts the file itself as well as a CLI directory.
		if (fs.statSync(base).isFile()) return base;
	} catch { /* Let spawning diagnose an unavailable CLI using the directory form. */ }
	return path.join(base, 'suspect');
}

/**
 * `suspect.test.baseUrl` (or the old `suspect.testBaseUrl`). `undefined`
 * leaves `--base-url` off, so the CLI applies `test.base_url` from the
 * committed `.suspect.yaml`.
 */
export function testBaseUrl(): string | undefined {
	return settings.testBaseUrl() ?? undefined;
}

/** `suspect.gateway.port` (or the old `suspect.gatewayPort`), else 8080. */
export function gatewayPort(): number {
	return settings.gatewayPort() ?? 8080;
}

/**
 * Spawn `suspect test <arazzo> [--filter <id>] --base-url <url> --report ndjson`
 * and stream parsed TestEvents to `onEvent`.
 *
 * Exit codes 0 (pass) and 1 (failures) resolve with the run_done totals;
 * anything else (spawn failure, usage error) rejects.
 */
export interface RunOptions {
	/** Base URL the suite runs against. Defaults to the editor setting, then the CLI's `test.base_url`. */
	baseUrl?: string;
	/** Recorded cassette: runs the suite offline, replaying the recording. */
	cassette?: string;
	/** Run only workflows whose id contains this substring. */
	filter?: string;
	/** Working directory for the run; relative `sourceDescriptions` resolve from the document regardless. */
	cwd?: string;
}

/**
 * The options-aware runner behind the Testing view: suites a manifest
 * declares pass its base URL and cassette so they run what the project
 * declared, not what the editor has configured.
 */
export function spawnSuspectRunWith(
	arazzoPath: string,
	options: RunOptions,
	onEvent: (event: SuspectEvent) => void,
	binary: string = suspectBinary(),
): SuspectRunHandle {
	const args = ['test', arazzoPath];
	const baseUrl = options.baseUrl ?? testBaseUrl();
	if (baseUrl !== undefined) {
		args.push('--base-url', baseUrl);
	}
	args.push('--report', 'ndjson');
	if (options.filter !== undefined) {
		args.push('--filter', options.filter);
	}
	if (options.cassette !== undefined) {
		args.push('--cassette', options.cassette);
	}

	let child: cp.ChildProcess | undefined;
	let killed = false;

	const done = new Promise<RunTotals>((resolve, reject) => {
		try {
			child = cp.spawn(binary, args, { stdio: ['ignore', 'pipe', 'pipe'], cwd: options.cwd });
		} catch (err) {
			reject(err instanceof Error ? err : new Error(String(err)));
			return;
		}
		let stderrTail = '';
		child.stderr?.on('data', (chunk: Buffer) => {
			stderrTail = (stderrTail + chunk.toString()).slice(-2000);
		});
		let buffer = '';
		let totals: RunTotals = { passed: 0, failed: 0 };
		child.stdout?.on('data', (chunk: Buffer) => {
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
				let event: unknown;
				try {
					event = JSON.parse(line);
				} catch {
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
				} catch {
					// consumer errors must not kill the stream
				}
			}
		});
		child.on('error', (err: Error) => reject(err));
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

/** The original signature: the editor's base URL, no cassette. */
export function spawnSuspectRun(
	arazzoPath: string,
	filter: string | undefined,
	onEvent: (event: SuspectEvent) => void,
): SuspectRunHandle {
	return spawnSuspectRunWith(arazzoPath, { filter }, onEvent);
}

export function errorMessage(err: unknown): string {
	return err instanceof Error ? err.message : String(err);
}
