"use strict";
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
exports.SDK_ARTIFACT_LIMIT = exports.SDK_RECORD_LIMIT = exports.SDK_SESSION_FORMAT = exports.SDK_COMPATIBILITY_PROFILES = exports.SDK_PROFILE_DIRECTORIES = void 0;
exports.isSdkProfile = isSdkProfile;
exports.readSdkCompatibilityProfiles = readSdkCompatibilityProfiles;
exports.generationArgs = generationArgs;
exports.availableSdkProfiles = availableSdkProfiles;
exports.runGeneration = runGeneration;
exports.sdkSessionArgs = sdkSessionArgs;
exports.readSdkSessionIdentity = readSdkSessionIdentity;
exports.parseSdkSessionRecord = parseSdkSessionRecord;
exports.startSdkSession = startSdkSession;
exports.readCurrentSdkArtifact = readCurrentSdkArtifact;
const cp = __importStar(require("child_process"));
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const util_1 = require("util");
exports.SDK_PROFILE_DIRECTORIES = {
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
};
function isSdkProfile(kind) {
    return Object.hasOwn(exports.SDK_PROFILE_DIRECTORIES, kind);
}
exports.SDK_COMPATIBILITY_PROFILES = ['legacy-binary-string-v1'];
function isSdkCompatibilityProfiles(value) {
    return Array.isArray(value) && value.length <= 32 &&
        value.every((profile) => exports.SDK_COMPATIBILITY_PROFILES.some((known) => profile === known));
}
/** Compatibility is an explicit, closed setting; source text and inventory prose cannot enable it. */
function readSdkCompatibilityProfiles(value) {
    if (!isSdkCompatibilityProfiles(value)) {
        throw new Error(`Invalid SDK compatibility profiles; select only ${exports.SDK_COMPATIBILITY_PROFILES.join(', ')}, or [] for ordinary OpenAPI semantics.`);
    }
    return [...value];
}
/** Construct argv without shell parsing or normalizing exact operation selectors. */
function generationArgs(spec, out, generation) {
    const args = ['gen', spec, '--out', out];
    if (generation.kind === 'custom')
        return [...args, '--manifest', generation.manifest];
    if (generation.kind === 'docs-md')
        return [...args, '--preset', 'docs-md'];
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
/** Discover compiled profiles from the real CLI instead of assuming that a menu label is supported. */
function availableSdkProfiles(binary, cwd) {
    return new Promise((resolve, reject) => {
        cp.execFile(binary, ['codegen-profiles', '--format', 'json'], {
            cwd, encoding: 'utf8', timeout: 5000, maxBuffer: 256 * 1024, windowsHide: true, shell: false, killSignal: 'SIGKILL',
        }, (error, stdout) => {
            if (error) {
                reject(new Error(`Unable to read SDK profiles from ${binary}: ${error.message}`));
                return;
            }
            try {
                const value = JSON.parse(stdout);
                if (!isObject(value) || value.format !== 'suspect.sdk.profiles.v1' ||
                    !Array.isArray(value.profiles) || value.profiles.length > 32)
                    throw new Error('Invalid SDK profile inventory');
                const seen = new Set();
                const profiles = value.profiles.map((item) => {
                    if (!isObject(item) || typeof item.profile !== 'string' || !isSdkProfile(item.profile) ||
                        item.directory !== exports.SDK_PROFILE_DIRECTORIES[item.profile] || seen.has(item.profile) ||
                        typeof item.description !== 'string' || item.description.length > 4096)
                        throw new Error('Invalid SDK profile entry');
                    seen.add(item.profile);
                    return { profile: item.profile, directory: exports.SDK_PROFILE_DIRECTORIES[item.profile], description: item.description };
                });
                resolve(profiles);
            }
            catch (error) {
                reject(error);
            }
        });
    });
}
/** Execute the real CLI and retain canonical stdout diagnostics as well as stderr. */
function runGeneration(binary, args, cwd) {
    return new Promise((resolve, reject) => {
        const child = cp.spawn(binary, args, { cwd, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
        let stdout = '';
        let stderr = '';
        child.stdout.on('data', (chunk) => { stdout = (stdout + chunk.toString()).slice(-16000); });
        child.stderr.on('data', (chunk) => { stderr = (stderr + chunk.toString()).slice(-4000); });
        child.on('error', reject);
        child.on('close', (code, signal) => {
            if (code === 0)
                resolve();
            else
                reject(new Error(`suspect ${args[0]} exited ${signal ? `on ${signal}` : `with code ${code}`}\n${stdout.trim()}\n${stderr.trim()}`.trim()));
        });
    });
}
exports.SDK_SESSION_FORMAT = 'suspect.sdk.session.v1';
exports.SDK_RECORD_LIMIT = 16 * 1024 * 1024;
exports.SDK_ARTIFACT_LIMIT = 4096;
const SDK_CONFIG_LIMIT = 4 * 1024 * 1024;
const SDK_STDERR_LIMIT = 8192;
/** Both arguments are explicit paths. No source prose, selector, or generated code is executed. */
function sdkSessionArgs(configPath, outDirectory, options) {
    return [
        'codegen-session', '--config', configPath, '--out', outDirectory,
        ...(options.watch ? ['--watch'] : []),
        ...(options.check ? ['--check'] : []),
        ...(options.preview ? ['--preview'] : []),
        '--format', 'json',
    ];
}
/** Only read enough configuration to label its identity; the CLI owns SDK validation. */
async function readSdkSessionIdentity(configPath, outDirectory) {
    configPath = path.resolve(configPath);
    const configDirectory = path.dirname(configPath);
    const identity = { configPath, configDirectory, outDirectory: path.resolve(configDirectory, outDirectory) };
    try {
        const config = JSON.parse(await readBoundedText(configPath, SDK_CONFIG_LIMIT));
        const configuredSource = isObject(config) ? config.spec ?? (isObject(config.pins) ? config.pins.manifest : undefined) : undefined;
        if (typeof configuredSource === 'string' && configuredSource.length &&
            !/[\u0000-\u001f]/.test(configuredSource) && !/^[a-z][a-z\d+.-]*:/i.test(configuredSource)) {
            // Normalize path segments without following entry/config symlinks. Their lexical
            // locations, rather than realpath targets, determine relative-reference bases.
            const sourcePath = path.resolve(configDirectory, configuredSource);
            return { ...identity, sourcePath, sourceRoot: path.dirname(sourcePath) };
        }
    }
    catch {
        // The canonical CLI reports malformed/missing config and can recover in watch mode.
    }
    return identity;
}
function isObject(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function isCount(value) {
    return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}
function isCounts(value) {
    return isObject(value) && isCount(value.compiles) && isCount(value.renders) && isCount(value.cache_hits);
}
/** Artifact names are portable, relative paths, never file/command URIs or traversal. */
function isArtifactPath(value) {
    return typeof value === 'string' && value.length > 0 && value.length <= 4096 &&
        !/[\\:\u0000-\u001f\u007f]/.test(value) &&
        value.split('/').every((part) => part.length > 0 && part !== '.' && part !== '..');
}
function isPaths(value, artifacts) {
    return Array.isArray(value) && value.length <= exports.SDK_ARTIFACT_LIMIT &&
        value.every((entry) => artifacts ? isArtifactPath(entry) : typeof entry === 'string' && entry.length <= 16384);
}
/** Fail closed on incompatible output instead of presenting partial or obsolete artifacts. */
function parseSdkSessionRecord(text, preview) {
    const value = JSON.parse(text);
    if (!isObject(value) || value.format !== exports.SDK_SESSION_FORMAT || typeof value.success !== 'boolean' ||
        !['current', 'drift', 'planning-error', 'write-conflict', 'written'].includes(value.status) || !isCount(value.generation) ||
        (value.source === null && value.status !== 'planning-error') ||
        (value.source !== undefined && value.source !== null && (typeof value.source !== 'string' || value.source.length > 16384 || !path.isAbsolute(value.source) || value.source.includes('\0'))) ||
        (value.output !== undefined && (typeof value.output !== 'string' || value.output.length > 16384 || !path.isAbsolute(value.output) || value.output.includes('\0'))) ||
        (value.config !== undefined && (typeof value.config !== 'string' || value.config.length > 16384 || !path.isAbsolute(value.config) || value.config.includes('\0'))) ||
        (value.revision !== undefined && (typeof value.revision !== 'string' || !value.revision.length || value.revision.length > 1024)) ||
        (value.compatibilityProfiles !== undefined && !isSdkCompatibilityProfiles(value.compatibilityProfiles)) ||
        !isPaths(value.changedArtifacts, true) || !isPaths(value.newDocuments, false) ||
        !isCounts(value.delta) || !isCounts(value.stats) || !Array.isArray(value.diagnostics) || value.diagnostics.length > exports.SDK_ARTIFACT_LIMIT ||
        !value.diagnostics.every((entry) => isObject(entry) && typeof entry.message === 'string' &&
            (entry.code === undefined || typeof entry.code === 'string')) ||
        value.success !== (value.status === 'current' || value.status === 'written')) {
        throw new Error(`Invalid SDK session record; expected ${exports.SDK_SESSION_FORMAT} with explicit status and diagnostics.`);
    }
    if (value.artifacts !== undefined) {
        if (!Array.isArray(value.artifacts) || value.artifacts.length > exports.SDK_ARTIFACT_LIMIT ||
            !value.artifacts.every((file) => isObject(file) && isArtifactPath(file.path) && typeof file.content === 'string') ||
            new Set(value.artifacts.map((file) => file.path)).size !== value.artifacts.length) {
            throw new Error('Invalid SDK session artifacts: expected unique, portable relative paths and text contents.');
        }
    }
    else if (preview && value.status !== 'planning-error') {
        throw new Error('SDK preview record is missing its complete artifacts array.');
    }
    return value;
}
/**
 * One persistent CLI process. Watch stdout is NDJSON; one-shot JSON may be pretty-printed.
 * Only one incomplete record and the latest result are retained. Cancellation and invalid
 * output stop delivery immediately, terminate the process, and escalate after one second.
 */
function startSdkSession(binary, identity, options, onRecord) {
    let child;
    let disposed = false;
    let closed = false;
    let failure;
    let killTimer;
    let latest;
    let parts = [];
    let bytes = 0;
    let stderr = Buffer.alloc(0);
    const args = sdkSessionArgs(identity.configPath, identity.outDirectory, options);
    const terminate = () => {
        if (!child?.pid || closed || child.exitCode !== null || child.signalCode !== null || killTimer)
            return;
        child.kill('SIGTERM');
        if (closed)
            return;
        killTimer = setTimeout(() => {
            if (!closed && child?.exitCode === null && child.signalCode === null)
                child.kill('SIGKILL');
        }, 1000);
        killTimer.unref();
    };
    const fail = (error) => {
        failure ??= error instanceof Error ? error : new Error(String(error));
        parts = [];
        bytes = 0;
        terminate();
    };
    const done = new Promise((resolve, reject) => {
        const consume = () => {
            const text = new util_1.TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(parts, bytes));
            parts = [];
            bytes = 0;
            if (!text.trim())
                return;
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
        const append = (chunk) => {
            bytes += chunk.length;
            if (bytes > exports.SDK_RECORD_LIMIT)
                throw new Error(`SDK session record exceeds the ${exports.SDK_RECORD_LIMIT / 1024 / 1024} MiB preview limit.`);
            if (chunk.length)
                parts.push(chunk);
        };
        try {
            child = cp.spawn(binary, args, {
                cwd: identity.configDirectory, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
            });
        }
        catch (error) {
            closed = true;
            reject(error);
            return;
        }
        child.stdout.on('data', (chunk) => {
            if (disposed || failure)
                return;
            try {
                if (!options.watch) {
                    append(chunk);
                    return;
                }
                let start = 0;
                for (let end = chunk.indexOf(10); end !== -1; end = chunk.indexOf(10, start)) {
                    append(chunk.subarray(start, end));
                    consume();
                    if (disposed)
                        return;
                    start = end + 1;
                }
                append(chunk.subarray(start));
            }
            catch (error) {
                fail(error);
            }
        });
        child.stderr.on('data', (chunk) => {
            if (!disposed)
                stderr = Buffer.concat([stderr, chunk.subarray(-SDK_STDERR_LIMIT)]).subarray(-SDK_STDERR_LIMIT);
        });
        child.on('error', (error) => {
            failure ??= error;
            // A spawn failure has no running process and is followed by close.
            if (!child?.pid) {
                if (disposed)
                    resolve(undefined);
                else
                    reject(error);
            }
            else
                terminate();
        });
        child.on('close', (code, signal) => {
            closed = true;
            if (killTimer)
                clearTimeout(killTimer);
            if (disposed) {
                resolve(undefined);
                return;
            }
            if (!failure) {
                try {
                    if (bytes)
                        consume();
                }
                catch (error) {
                    fail(error);
                }
            }
            const detail = stderr.toString('utf8').trim();
            if (failure)
                reject(new Error(`${failure.message}${detail ? `\n${detail}` : ''}`));
            else if (signal || (code !== 0 && code !== 1) || !latest || (code === 1 && latest.success)) {
                reject(new Error(`suspect codegen-session exited ${signal ? `on ${signal}` : `with code ${code}`}${!latest ? ' without a session record' : ''}${detail ? `\n${detail}` : ''}`));
            }
            else
                resolve(latest);
        });
    });
    return {
        done,
        dispose() {
            if (disposed || closed)
                return;
            disposed = true;
            parts = [];
            bytes = 0;
            latest = undefined;
            terminate();
        },
    };
}
async function readBoundedText(filename, limit) {
    const stat = await fs.promises.stat(filename);
    if (!stat.isFile())
        throw new Error(`Not a regular text file: ${filename}`);
    if (stat.size > limit)
        throw new Error(`${filename} exceeds the ${limit} byte editor read limit.`);
    const stream = fs.createReadStream(filename);
    const chunks = [];
    let bytes = 0;
    try {
        for await (const chunk of stream) {
            const buffer = chunk;
            bytes += buffer.length;
            if (bytes > limit)
                throw new Error(`${filename} exceeds the ${limit} byte editor read limit.`);
            chunks.push(buffer);
        }
        return new util_1.TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks, bytes));
    }
    finally {
        stream.destroy();
    }
}
/** Read a current disk snapshot without traversing a generated artifact symlink. Missing is empty. */
async function readCurrentSdkArtifact(outDirectory, artifactPath) {
    if (!isArtifactPath(artifactPath))
        throw new Error('Unsafe SDK artifact path.');
    try {
        // An explicitly selected output root may itself be a symlink. Descendants may not.
        let current = await fs.promises.realpath(outDirectory);
        for (const part of artifactPath.split('/')) {
            current = path.join(current, part);
            const stat = await fs.promises.lstat(current);
            if (stat.isSymbolicLink())
                throw new Error(`Cannot preview a symlink artifact: ${artifactPath}`);
        }
        return { content: await readBoundedText(current, exports.SDK_RECORD_LIMIT), exists: true };
    }
    catch (error) {
        if (error.code === 'ENOENT')
            return { content: '', exists: false };
        throw error;
    }
}
//# sourceMappingURL=generation.js.map