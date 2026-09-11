import * as cp from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import { Readable } from 'stream';
import { TextDecoder } from 'util';

export const SDK_PROFILE_DIRECTORIES = {
	'typescript-http': 'typescript',
	'rust-http': 'rust',
	'python-http': 'python',
	'go-http': 'go',
	'swift-http': 'swift',
	'java-http': 'java',
	'csharp-http': 'csharp',
	'kotlin-http': 'kotlin',
	'ruby-http': 'ruby',
	'php-http': 'php',
	'dart-http': 'dart',
	'cpp-http': 'cpp',
} as const;

export type SdkProfile = keyof typeof SDK_PROFILE_DIRECTORIES;

export function isSdkProfile(kind: string): kind is SdkProfile {
	return Object.hasOwn(SDK_PROFILE_DIRECTORIES, kind);
}

export const SDK_COMPATIBILITY_PROFILES = ['legacy-binary-string-v1'] as const;
export type SdkCompatibilityProfile = typeof SDK_COMPATIBILITY_PROFILES[number];

function isSdkCompatibilityProfiles(value: unknown): value is SdkCompatibilityProfile[] {
	return Array.isArray(value) && value.length <= 32 &&
		value.every((profile) => SDK_COMPATIBILITY_PROFILES.some((known) => profile === known));
}

/** Compatibility is an explicit, closed setting; source text and inventory prose cannot enable it. */
export function readSdkCompatibilityProfiles(value: unknown): SdkCompatibilityProfile[] {
	if (!isSdkCompatibilityProfiles(value)) {
		throw new Error(`Invalid SDK compatibility profiles; select only ${SDK_COMPATIBILITY_PROFILES.join(', ')}, or [] for ordinary OpenAPI semantics.`);
	}
	return [...value];
}

export type Generation =
	| { kind: SdkProfile; packageName: string; packageVersion: string; importName?: string; compatibilityProfiles?: readonly SdkCompatibilityProfile[]; operationIds: readonly string[]; check?: boolean }
	| { kind: 'docs-md' }
	| { kind: 'custom'; manifest: string };

/** Construct argv without shell parsing or normalizing exact operation selectors. */
export function generationArgs(spec: string, out: string, generation: Generation): string[] {
	const args = ['gen', spec, '--out', out];
	if (generation.kind === 'custom') return [...args, '--manifest', generation.manifest];
	if (generation.kind === 'docs-md') return [...args, '--preset', 'docs-md'];
	return [
		'codegen', spec, '--profile', generation.kind,
		'--package-name', generation.packageName, '--package-version', generation.packageVersion,
		'--out', out, '--format', 'json',
		...(generation.importName === undefined ? [] : ['--import-name', generation.importName]),
		...readSdkCompatibilityProfiles(generation.compatibilityProfiles ?? []).flatMap((profile) => ['--compatibility-profile', profile]),
		...generation.operationIds.flatMap((id) => ['--operation-id', id]),
		...(generation.check ? ['--check'] : []),
	];
}

/** One native profile advertised by the selected CLI build. */
export interface AvailableSdkProfile {
	profile: SdkProfile;
	directory: typeof SDK_PROFILE_DIRECTORIES[SdkProfile];
	description: string;
}

/** Discover compiled profiles from the real CLI instead of assuming that a menu label is supported. */
export function availableSdkProfiles(binary: string, cwd?: string): Promise<AvailableSdkProfile[]> {
	return new Promise((resolve, reject) => {
		cp.execFile(binary, ['codegen-profiles', '--format', 'json'], {
			cwd, encoding: 'utf8', timeout: 5000, maxBuffer: 256 * 1024, windowsHide: true, shell: false, killSignal: 'SIGKILL',
		}, (error, stdout) => {
			if (error) { reject(new Error(`Unable to read SDK profiles from ${binary}: ${error.message}`)); return; }
			try {
				const value: unknown = JSON.parse(stdout);
				if (!isObject(value) || value.format !== 'suspect.sdk.profiles.v1' ||
					!Array.isArray(value.profiles) || value.profiles.length > 32) throw new Error('Invalid SDK profile inventory');
				const seen = new Set<string>();
				const profiles = value.profiles.map((item: unknown): AvailableSdkProfile => {
					if (!isObject(item) || typeof item.profile !== 'string' || !isSdkProfile(item.profile) ||
						item.directory !== SDK_PROFILE_DIRECTORIES[item.profile] || seen.has(item.profile) ||
						typeof item.description !== 'string' || item.description.length > 4096) throw new Error('Invalid SDK profile entry');
					seen.add(item.profile);
					return { profile: item.profile, directory: SDK_PROFILE_DIRECTORIES[item.profile], description: item.description };
				});
				resolve(profiles);
			} catch (error) { reject(error); }
		});
	});
}

/** Execute the real CLI and retain canonical stdout diagnostics as well as stderr. */
export function runGeneration(binary: string, args: readonly string[], cwd?: string): Promise<void> {
	return new Promise((resolve, reject) => {
		const child = cp.spawn(binary, args, { cwd, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
		let stdout = '';
		let stderr = '';
		child.stdout.on('data', (chunk: Buffer) => { stdout = (stdout + chunk.toString()).slice(-16000); });
		child.stderr.on('data', (chunk: Buffer) => { stderr = (stderr + chunk.toString()).slice(-4000); });
		child.on('error', reject);
		child.on('close', (code, signal) => {
			if (code === 0) resolve();
			else reject(new Error(`suspect ${args[0]} exited ${signal ? `on ${signal}` : `with code ${code}`}\n${stdout.trim()}\n${stderr.trim()}`.trim()));
		});
	});
}

export const SDK_SESSION_FORMAT = 'suspect.sdk.session.v1';
export const SDK_RECORD_LIMIT = 16 * 1024 * 1024;
export const SDK_ARTIFACT_LIMIT = 4096;
const SDK_CONFIG_LIMIT = 4 * 1024 * 1024;
const SDK_STDERR_LIMIT = 8192;

export interface SdkSessionIdentity {
	configPath: string;
	configDirectory: string;
	outDirectory: string;
	sourcePath?: string;
	sourceRoot?: string;
}

export interface SdkSessionOptions {
	watch?: boolean;
	check?: boolean;
	preview?: boolean;
}

export interface SdkSessionCounts {
	compiles: number;
	renders: number;
	cache_hits: number;
}

export interface SdkSessionDiagnostic {
	message: string;
	code?: string;
	[key: string]: unknown;
}

export interface SdkSessionRecord {
	format: typeof SDK_SESSION_FORMAT;
	success: boolean;
	status: 'current' | 'drift' | 'planning-error' | 'write-conflict' | 'written';
	generation: number;
	/** Lexical absolute retrieval path, preserving a selected symlink's relative-ref base. */
	source?: string | null;
	output?: string;
	config?: string;
	/** Opaque content/configuration identity; cached reverts may repeat a prior revision. */
	revision?: string;
	/** Explicit interpretation recorded by the CLI from the saved session config. */
	compatibilityProfiles?: SdkCompatibilityProfile[];
	changedArtifacts: string[];
	newDocuments: string[];
	delta: SdkSessionCounts;
	stats: SdkSessionCounts;
	diagnostics: SdkSessionDiagnostic[];
	/** Complete desired artifact set, including unchanged files, in preview mode. */
	artifacts?: { path: string; content: string }[];
}

export interface SdkSessionHandle {
	/** Drift and planning errors are records, not transport failures. Disposal resolves undefined. */
	done: Promise<SdkSessionRecord | undefined>;
	dispose(): void;
}

/** Both arguments are explicit paths. No source prose, selector, or generated code is executed. */
export function sdkSessionArgs(configPath: string, outDirectory: string, options: SdkSessionOptions): string[] {
	return [
		'codegen-session', '--config', configPath, '--out', outDirectory,
		...(options.watch ? ['--watch'] : []),
		...(options.check ? ['--check'] : []),
		...(options.preview ? ['--preview'] : []),
		'--format', 'json',
	];
}

/** Only read enough configuration to label its identity; the CLI owns SDK validation. */
export async function readSdkSessionIdentity(configPath: string, outDirectory: string): Promise<SdkSessionIdentity> {
	configPath = path.resolve(configPath);
	const configDirectory = path.dirname(configPath);
	const identity = { configPath, configDirectory, outDirectory: path.resolve(configDirectory, outDirectory) };
	try {
		const config: unknown = JSON.parse(await readBoundedText(configPath, SDK_CONFIG_LIMIT));
		const configuredSource = isObject(config) ? config.spec ?? (isObject(config.pins) ? config.pins.manifest : undefined) : undefined;
		if (typeof configuredSource === 'string' && configuredSource.length &&
			!/[\u0000-\u001f]/.test(configuredSource) && !/^[a-z][a-z\d+.-]*:/i.test(configuredSource)) {
			// Normalize path segments without following entry/config symlinks. Their lexical
			// locations, rather than realpath targets, determine relative-reference bases.
			const sourcePath = path.resolve(configDirectory, configuredSource);
			return { ...identity, sourcePath, sourceRoot: path.dirname(sourcePath) };
		}
	} catch {
		// The canonical CLI reports malformed/missing config and can recover in watch mode.
	}
	return identity;
}

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isCount(value: unknown): value is number {
	return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

function isCounts(value: unknown): value is SdkSessionCounts {
	return isObject(value) && isCount(value.compiles) && isCount(value.renders) && isCount(value.cache_hits);
}

/** Artifact names are portable, relative paths, never file/command URIs or traversal. */
function isArtifactPath(value: unknown): value is string {
	return typeof value === 'string' && value.length > 0 && value.length <= 4096 &&
		!/[\\:\u0000-\u001f\u007f]/.test(value) &&
		value.split('/').every((part) => part.length > 0 && part !== '.' && part !== '..');
}

function isPaths(value: unknown, artifacts: boolean): value is string[] {
	return Array.isArray(value) && value.length <= SDK_ARTIFACT_LIMIT &&
		value.every((entry) => artifacts ? isArtifactPath(entry) : typeof entry === 'string' && entry.length <= 16384);
}

/** Fail closed on incompatible output instead of presenting partial or obsolete artifacts. */
export function parseSdkSessionRecord(text: string, preview: boolean): SdkSessionRecord {
	const value: unknown = JSON.parse(text);
	if (!isObject(value) || value.format !== SDK_SESSION_FORMAT || typeof value.success !== 'boolean' ||
		!['current', 'drift', 'planning-error', 'write-conflict', 'written'].includes(value.status as string) || !isCount(value.generation) ||
		(value.source === null && value.status !== 'planning-error') ||
		(value.source !== undefined && value.source !== null && (typeof value.source !== 'string' || value.source.length > 16384 || !path.isAbsolute(value.source) || value.source.includes('\0'))) ||
		(value.output !== undefined && (typeof value.output !== 'string' || value.output.length > 16384 || !path.isAbsolute(value.output) || value.output.includes('\0'))) ||
		(value.config !== undefined && (typeof value.config !== 'string' || value.config.length > 16384 || !path.isAbsolute(value.config) || value.config.includes('\0'))) ||
		(value.revision !== undefined && (typeof value.revision !== 'string' || !value.revision.length || value.revision.length > 1024)) ||
		(value.compatibilityProfiles !== undefined && !isSdkCompatibilityProfiles(value.compatibilityProfiles)) ||
		!isPaths(value.changedArtifacts, true) || !isPaths(value.newDocuments, false) ||
		!isCounts(value.delta) || !isCounts(value.stats) || !Array.isArray(value.diagnostics) || value.diagnostics.length > SDK_ARTIFACT_LIMIT ||
		!value.diagnostics.every((entry) => isObject(entry) && typeof entry.message === 'string' &&
			(entry.code === undefined || typeof entry.code === 'string')) ||
		value.success !== (value.status === 'current' || value.status === 'written')) {
		throw new Error(`Invalid SDK session record; expected ${SDK_SESSION_FORMAT} with explicit status and diagnostics.`);
	}
	if (value.artifacts !== undefined) {
		if (!Array.isArray(value.artifacts) || value.artifacts.length > SDK_ARTIFACT_LIMIT ||
			!value.artifacts.every((file) => isObject(file) && isArtifactPath(file.path) && typeof file.content === 'string') ||
			new Set(value.artifacts.map((file) => file.path)).size !== value.artifacts.length) {
			throw new Error('Invalid SDK session artifacts: expected unique, portable relative paths and text contents.');
		}
	} else if (preview && value.status !== 'planning-error') {
		throw new Error('SDK preview record is missing its complete artifacts array.');
	}
	return value as unknown as SdkSessionRecord;
}

/**
 * One persistent CLI process. Watch stdout is NDJSON; one-shot JSON may be pretty-printed.
 * Only one incomplete record and the latest result are retained. Cancellation and invalid
 * output stop delivery immediately, terminate the process, and escalate after one second.
 */
export function startSdkSession(
	binary: string,
	identity: SdkSessionIdentity,
	options: SdkSessionOptions,
	onRecord: (record: SdkSessionRecord) => void,
): SdkSessionHandle {
	let child: cp.ChildProcessByStdio<null, Readable, Readable> | undefined;
	let disposed = false;
	let closed = false;
	let failure: Error | undefined;
	let killTimer: NodeJS.Timeout | undefined;
	let latest: SdkSessionRecord | undefined;
	let parts: Buffer[] = [];
	let bytes = 0;
	let stderr = Buffer.alloc(0);
	const args = sdkSessionArgs(identity.configPath, identity.outDirectory, options);
	const terminate = () => {
		if (!child?.pid || closed || child.exitCode !== null || child.signalCode !== null || killTimer) return;
		child.kill('SIGTERM');
		if (closed) return;
		killTimer = setTimeout(() => {
			if (!closed && child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
		}, 1000);
		killTimer.unref();
	};
	const fail = (error: unknown) => {
		failure ??= error instanceof Error ? error : new Error(String(error));
		parts = [];
		bytes = 0;
		terminate();
	};
	const done = new Promise<SdkSessionRecord | undefined>((resolve, reject) => {
		const consume = () => {
			const text = new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(parts, bytes));
			parts = [];
			bytes = 0;
			if (!text.trim()) return;
			const record = parseSdkSessionRecord(text, options.preview === true);
			if ((options.preview || options.check) && record.status === 'written') {
				throw new Error('Read-only SDK session unexpectedly reported a write.');
			}
			if (record.status !== 'planning-error' && !record.source && !identity.sourcePath) {
				throw new Error('SDK session did not report its source identity.');
			}
			if (record.output !== undefined && path.resolve(record.output) !== identity.outDirectory) {
				throw new Error('SDK session output root does not match the requested output directory.');
			}
			if (record.config !== undefined && path.resolve(record.config) !== identity.configPath) {
				throw new Error('SDK session config identity does not match the requested config.');
			}
			if (latest && record.generation <= latest.generation) {
				throw new Error('SDK session generations must increase; refusing an obsolete result.');
			}
			latest = record;
			onRecord(record);
		};
		const append = (chunk: Buffer) => {
			bytes += chunk.length;
			if (bytes > SDK_RECORD_LIMIT) throw new Error(`SDK session record exceeds the ${SDK_RECORD_LIMIT / 1024 / 1024} MiB preview limit.`);
			if (chunk.length) parts.push(chunk);
		};
		try {
			child = cp.spawn(binary, args, {
				cwd: identity.configDirectory, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
			});
		} catch (error) {
			closed = true;
			reject(error);
			return;
		}
		child.stdout.on('data', (chunk: Buffer) => {
			if (disposed || failure) return;
			try {
				if (!options.watch) {
					append(chunk);
					return;
				}
				let start = 0;
				for (let end = chunk.indexOf(10); end !== -1; end = chunk.indexOf(10, start)) {
					append(chunk.subarray(start, end));
					consume();
					if (disposed) return;
					start = end + 1;
				}
				append(chunk.subarray(start));
			} catch (error) { fail(error); }
		});
		child.stderr.on('data', (chunk: Buffer) => {
			if (!disposed) stderr = Buffer.concat([stderr, chunk.subarray(-SDK_STDERR_LIMIT)]).subarray(-SDK_STDERR_LIMIT);
		});
		child.on('error', (error: Error) => {
			failure ??= error;
			// A spawn failure has no running process and is followed by close.
			if (!child?.pid) {
				if (disposed) resolve(undefined);
				else reject(error);
			} else terminate();
		});
		child.on('close', (code, signal) => {
			closed = true;
			if (killTimer) clearTimeout(killTimer);
			if (disposed) { resolve(undefined); return; }
			if (!failure) {
				try { if (bytes) consume(); } catch (error) { fail(error); }
			}
			const detail = stderr.toString('utf8').trim();
			if (failure) reject(new Error(`${failure.message}${detail ? `\n${detail}` : ''}`));
			else if (signal || (code !== 0 && code !== 1) || !latest || (code === 1 && latest.success)) {
				reject(new Error(`suspect codegen-session exited ${signal ? `on ${signal}` : `with code ${code}`}${!latest ? ' without a session record' : ''}${detail ? `\n${detail}` : ''}`));
			} else resolve(latest);
		});
	});
	return {
		done,
		dispose() {
			if (disposed || closed) return;
			disposed = true;
			parts = [];
			bytes = 0;
			latest = undefined;
			terminate();
		},
	};
}

async function readBoundedText(filename: string, limit: number): Promise<string> {
	const stat = await fs.promises.stat(filename);
	if (!stat.isFile()) throw new Error(`Not a regular text file: ${filename}`);
	if (stat.size > limit) throw new Error(`${filename} exceeds the ${limit} byte editor read limit.`);
	const stream = fs.createReadStream(filename);
	const chunks: Buffer[] = [];
	let bytes = 0;
	try {
		for await (const chunk of stream) {
			const buffer = chunk as Buffer;
			bytes += buffer.length;
			if (bytes > limit) throw new Error(`${filename} exceeds the ${limit} byte editor read limit.`);
			chunks.push(buffer);
		}
		return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks, bytes));
	} finally { stream.destroy(); }
}

/** Read a current disk snapshot without traversing a generated artifact symlink. Missing is empty. */
export async function readCurrentSdkArtifact(outDirectory: string, artifactPath: string): Promise<{ content: string; exists: boolean }> {
	if (!isArtifactPath(artifactPath)) throw new Error('Unsafe SDK artifact path.');
	try {
		// An explicitly selected output root may itself be a symlink. Descendants may not.
		let current = await fs.promises.realpath(outDirectory);
		for (const part of artifactPath.split('/')) {
			current = path.join(current, part);
			const stat = await fs.promises.lstat(current);
			if (stat.isSymbolicLink()) throw new Error(`Cannot preview a symlink artifact: ${artifactPath}`);
		}
		return { content: await readBoundedText(current, SDK_RECORD_LIMIT), exists: true };
	} catch (error) {
		if ((error as NodeJS.ErrnoException).code === 'ENOENT') return { content: '', exists: false };
		throw error;
	}
}
