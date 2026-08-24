import * as cp from 'child_process';
import * as path from 'path';
import * as vscode from 'vscode';

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

function config(): vscode.WorkspaceConfiguration {
	return vscode.workspace.getConfiguration('suspect');
}

/** Resolve the suspect CLI path from `suspect.basePath`. */
export function suspectBinary(): string {
	const base = (config().get<string>('basePath') ?? 'suspect').trim();
	if (!base || base === 'suspect') {
		return 'suspect';
	}
	if (/[/\\]suspect$/.test(base)) {
		return base;
	}
	return path.join(base, 'suspect');
}

export function testBaseUrl(): string {
	return config().get<string>('testBaseUrl') ?? 'http://localhost:8080';
}

export function gatewayPort(): number {
	return config().get<number>('gatewayPort') ?? 8080;
}

/**
 * Spawn `suspect test <arazzo> [--filter <id>] --base-url <url> --report ndjson`
 * and stream parsed TestEvents to `onEvent`.
 *
 * Exit codes 0 (pass) and 1 (failures) resolve with the run_done totals;
 * anything else (spawn failure, usage error) rejects.
 */
export function spawnSuspectRun(
	arazzoPath: string,
	filter: string | undefined,
	onEvent: (event: SuspectEvent) => void,
): SuspectRunHandle {
	const args = ['test', arazzoPath, '--base-url', testBaseUrl(), '--report', 'ndjson'];
	if (filter !== undefined) {
		args.push('--filter', filter);
	}

	let child: cp.ChildProcess | undefined;
	let killed = false;

	const done = new Promise<RunTotals>((resolve, reject) => {
		try {
			child = cp.spawn(suspectBinary(), args, { stdio: ['ignore', 'pipe', 'pipe'] });
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

export function errorMessage(err: unknown): string {
	return err instanceof Error ? err.message : String(err);
}
