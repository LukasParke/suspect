const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const fsp = require('node:fs/promises');
const path = require('node:path');

const FORMATS = Object.freeze({
	pins: 'suspect.editor.native-host.pins.v1',
	report: 'suspect.editor.native-host.run.v2',
	native: 'suspect.editor.native-host.observations.v2',
	error: 'suspect.editor.native-host.error.v1',
});
const PROFILE_INFO = Object.freeze({
	'typescript-http': ['typescript', 'TypeScript / JavaScript'],
	'rust-http': ['rust', 'Rust'],
	'python-http': ['python', 'Python'],
	'go-http': ['go', 'Go'],
	'swift-http': ['swift', 'Swift'],
	'ruby-http': ['ruby', 'Ruby'],
	'csharp-http': ['csharp', 'C#'],
	'java-http': ['java', 'Java'],
	'kotlin-http': ['kotlin', 'Kotlin'],
	'php-http': ['php', 'PHP'],
	'dart-http': ['dart', 'Dart'],
	'cpp-http': ['cpp', 'C++'],
});
const CHECKS = Object.freeze({
	activation: 'installed VSIX activates in the real desktop extension host and registers production commands',
	renderer: 'renderer automation attaches to the isolated native VS Code workbench',
	lsp: 'packaged language-client startup uses the exact canonical stdio argv and keeps one LSP process alive',
	discovery: 'native profile discovery picker displays exactly the pinned CLI inventory and cancellation launches no SDK writer',
	diff: 'production watch opens a native read-only disk/generated diff with lexical config/source/output identities',
	changed: 'saved referenced-source changes refresh the open native diff using one canonical watch process',
	reverted: 'saved A→B→A source reverts refresh the same native virtual documents without disk writes',
	removed: 'removing a target renders an empty desired side while preserving its owned disk snapshot',
	recovered: 'real planning errors invalidate both native diff documents and recover in the same watch process',
	stop: 'production stop terminates the real watch and invalidates already-open native documents',
	dialogCancel: 'native output-directory dialog cancellation leaves no session process or SDK writes',
	progressCancel: 'native progress cancellation terminates an in-flight real CLI preview',
	generate: 'actual profile/package/selector dialogs generate and open the selected native package README',
	shutdownReady: 'a final live native watch is handed to the real extension-host shutdown lifecycle',
	shutdown: 'native host shutdown terminates the final canonical watch process',
	preview: 'registered one-shot Preview resolves configured roots and opens a complete native read-only diff',
	showLatest: 'registered Show Latest opens another artifact from the completed native preview with no CLI process',
	checkCurrent: 'registered Check SDK Drift reports current through the native UI without opening a diff or writing',
	checkDrift: 'registered Check SDK Drift exposes saved-source drift in the native UI while preserving owned disk bytes',
	selectionProbe: 'explicit pinned CLI discovery reaches the installed native profile picker',
});
const STARTUP = [CHECKS.activation, CHECKS.renderer, CHECKS.lsp];
const REQUIRED_CHECKS = Object.freeze({
	lifecycle: [...STARTUP, CHECKS.discovery, CHECKS.diff, CHECKS.changed, CHECKS.reverted, CHECKS.removed,
		CHECKS.recovered, CHECKS.stop, CHECKS.dialogCancel, CHECKS.progressCancel, CHECKS.generate, CHECKS.shutdownReady, CHECKS.shutdown],
	commands: [...STARTUP, CHECKS.preview, CHECKS.showLatest, CHECKS.checkCurrent, CHECKS.checkDrift],
});
const REQUIRED_SCREENSHOTS = Object.freeze({
	lifecycle: ['01-native-activation.png', '02-native-profile-picker.png', '03-native-readonly-diff-A.png', '04-native-live-diff-B.png',
		'05-native-removed-diff.png', '06-native-planning-error.png', '07-native-recovered-diff.png', '08-native-progress-cancellation.png',
		'09-native-generated-python-readme.png', '10-native-watch-before-shutdown.png'],
	commands: ['01-native-activation.png', 'commands-01-one-shot-preview.png', 'commands-02-current-check.png', 'commands-03-drift-check.png'],
});
const TOOL_NAMES = ['@vscode/test-electron', '@vscode/vsce', 'playwright-core'];
const NATIVE_ENV = ['SUSPECT_NATIVE_PINS', 'SUSPECT_NATIVE_OUT', 'SUSPECT_NATIVE_SCRATCH', 'SUSPECT_NATIVE_SCENARIO', 'SUSPECT_NATIVE_MODE'];
const sha256 = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

class InputError extends Error {
	constructor(message, code = 'E_INPUT') { super(message); this.name = 'InputError'; this.code = code; this.exitCode = 2; }
}
function requireInput(condition, message, code) { if (!condition) throw new InputError(message, code); }
function keys(value, required, optional, label) {
	requireInput(isObject(value), `${label} must be an object`);
	for (const key of required) requireInput(Object.hasOwn(value, key), `${label}.${key} is required`);
	for (const key of Object.keys(value)) requireInput(required.includes(key) || optional.includes(key), `Unknown field ${label}.${key}`);
}
function absolute(value, label) {
	requireInput(typeof value === 'string' && value.length > 0 && value.length <= 16384 && path.isAbsolute(value) && !/[\u0000-\u001f\u007f]/.test(value), `${label} must be an absolute local path`);
	return path.resolve(value);
}
function hash(value, label) { requireInput(typeof value === 'string' && /^[a-f0-9]{64}$/.test(value), `${label} must be a lowercase SHA-256 digest`); }
function freeze(value) {
	if (value && typeof value === 'object') { for (const child of Object.values(value)) freeze(child); Object.freeze(value); }
	return value;
}
function parseInvocation(env, argv = []) {
	requireInput(argv.length === 0, 'This runner takes no argv options; use the documented environment contract');
	for (const key of Object.keys(env)) {
		if (key.startsWith('SUSPECT_NATIVE_')) requireInput(NATIVE_ENV.includes(key), `Unknown native host input ${key}`);
	}
	const invocation = {
		pinsPath: absolute(env.SUSPECT_NATIVE_PINS, 'SUSPECT_NATIVE_PINS'),
		binary: absolute(env.SUSPECT_TEST_BINARY, 'SUSPECT_TEST_BINARY'),
		out: absolute(env.SUSPECT_NATIVE_OUT, 'SUSPECT_NATIVE_OUT'),
		scratch: absolute(env.SUSPECT_NATIVE_SCRATCH, 'SUSPECT_NATIVE_SCRATCH'),
		scenario: env.SUSPECT_NATIVE_SCENARIO,
		mode: env.SUSPECT_NATIVE_MODE ?? 'run',
	};
	requireInput(Object.hasOwn(REQUIRED_CHECKS, invocation.scenario), 'SUSPECT_NATIVE_SCENARIO must be lifecycle or commands');
	requireInput(['run', 'selection-probe'].includes(invocation.mode), 'SUSPECT_NATIVE_MODE must be run or selection-probe');
	return freeze(invocation);
}
function parsePins(value) {
	keys(value, ['format', 'cli', 'vscode', 'tools', 'extension'], [], 'pins');
	requireInput(value.format === FORMATS.pins, `Expected ${FORMATS.pins}`);
	keys(value.cli, ['sha256', 'expectedProfiles'], [], 'pins.cli');
	hash(value.cli.sha256, 'pins.cli.sha256');
	const profiles = value.cli.expectedProfiles;
	requireInput(Array.isArray(profiles) && profiles.length >= 3 && profiles.length <= 12 &&
		profiles.every((profile) => typeof profile === 'string' && Object.hasOwn(PROFILE_INFO, profile)) && new Set(profiles).size === profiles.length,
		'pins.cli.expectedProfiles must contain unique known backend IDs');
	for (const profile of ['typescript-http', 'rust-http', 'python-http']) requireInput(profiles.includes(profile), `Native scenarios require ${profile}`);
	keys(value.vscode, ['executable', 'executableSha256', 'archive', 'archiveSha256', 'version', 'commit', 'platform'], [], 'pins.vscode');
	absolute(value.vscode.executable, 'pins.vscode.executable');
	absolute(value.vscode.archive, 'pins.vscode.archive');
	hash(value.vscode.executableSha256, 'pins.vscode.executableSha256');
	hash(value.vscode.archiveSha256, 'pins.vscode.archiveSha256');
	requireInput(typeof value.vscode.version === 'string' && /^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(value.vscode.version), 'pins.vscode.version must be an exact version');
	requireInput(typeof value.vscode.commit === 'string' && /^[a-f0-9]{40}$/.test(value.vscode.commit), 'pins.vscode.commit must be an exact commit');
	requireInput(['darwin-arm64', 'darwin-x64', 'linux-x64', 'linux-arm64'].includes(value.vscode.platform), 'Unsupported native host platform');
	keys(value.tools, ['directory', 'lockSha256'], [], 'pins.tools');
	absolute(value.tools.directory, 'pins.tools.directory');
	hash(value.tools.lockSha256, 'pins.tools.lockSha256');
	keys(value.extension, ['sourceDirectory', 'sourceSha256'], ['vsix'], 'pins.extension');
	absolute(value.extension.sourceDirectory, 'pins.extension.sourceDirectory');
	hash(value.extension.sourceSha256, 'pins.extension.sourceSha256');
	if (value.extension.vsix !== undefined) {
		keys(value.extension.vsix, ['path', 'sha256'], [], 'pins.extension.vsix');
		absolute(value.extension.vsix.path, 'pins.extension.vsix.path');
		hash(value.extension.vsix.sha256, 'pins.extension.vsix.sha256');
	}
	return freeze(value);
}
async function jsonFile(filename, maximum = 4 * 1024 * 1024) {
	const stat = await fsp.stat(filename);
	requireInput(stat.isFile() && stat.size <= maximum, `Expected bounded JSON file: ${filename}`);
	const bytes = await fsp.readFile(filename);
	requireInput(bytes.length <= maximum, `JSON file grew beyond its limit: ${filename}`);
	try { return { value: JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)), sha256: sha256(bytes) }; }
	catch { throw new InputError(`Invalid JSON file: ${filename}`); }
}
async function fileIdentity(filename, expected) {
	const stat = await fsp.stat(filename);
	requireInput(stat.isFile(), `Expected regular input file: ${filename}`);
	const value = crypto.createHash('sha256');
	for await (const bytes of fs.createReadStream(filename)) value.update(bytes);
	const digest = value.digest('hex');
	if (expected !== undefined) requireInput(digest === expected, `SHA-256 mismatch: ${filename}`, 'E_HASH');
	return { path: filename, realPath: await fsp.realpath(filename), sha256: digest, size: stat.size };
}
function within(parent, child) {
	const relative = path.relative(parent, child);
	return relative === '' || (!relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative));
}
async function hashTree(directory) {
	const hashes = {};
	const root = await fsp.realpath(directory);
	async function visit(current, prefix = '') {
		const entries = (await fsp.readdir(current, { withFileTypes: true })).sort((a, b) => a.name < b.name ? -1 : 1);
		for (const entry of entries) {
			const relative = prefix + entry.name;
			const filename = path.join(current, entry.name);
			if (entry.isDirectory()) await visit(filename, `${relative}/`);
			else if (entry.isFile()) hashes[relative] = (await fileIdentity(filename)).sha256;
			else if (entry.isSymbolicLink()) {
				requireInput(within(root, await fsp.realpath(filename)), `Input symlink escapes its tree: ${filename}`);
				hashes[relative] = `symlink:${await fsp.readlink(filename)}`;
			} else throw new InputError(`Non-regular input tree entry: ${filename}`);
		}
	}
	await visit(directory);
	return { sha256: sha256(JSON.stringify(hashes)), hashes };
}
async function sourceIdentity(directory) {
	const manifest = (await jsonFile(path.join(directory, 'package.json'))).value;
	requireInput(manifest.name === 'suspect-vscode' && manifest.publisher === 'suspect' && manifest.main === './dist/extension.js', 'sourceDirectory must contain the Suspect editor extension');
	requireInput(typeof manifest.version === 'string' && /^\d+\.\d+\.\d+(?:[-+][\w.+-]+)?$/.test(manifest.version), 'The editor source must declare an exact package version');
	const hashes = {};
	for (const filename of ['package.json', 'package-lock.json']) hashes[filename] = (await fileIdentity(path.join(directory, filename))).sha256;
	for (const subdirectory of ['src', 'dist', 'media']) {
		const subtree = await hashTree(path.join(directory, subdirectory));
		for (const [filename, digest] of Object.entries(subtree.hashes)) {
			requireInput(!digest.startsWith('symlink:'), 'Editor source/runtime files must be regular files');
			hashes[`${subdirectory}/${filename}`] = digest;
		}
	}
	for (const filename of ['src/extension.ts', 'src/generation.ts', 'src/runner.ts', 'dist/extension.js', 'dist/generation.js', 'dist/runner.js']) requireInput(Object.hasOwn(hashes, filename), `Required editor file is missing: ${filename}`);
	return { directory, id: `${manifest.publisher}.${manifest.name}`, version: manifest.version, manifest, hashes,
		runtimeHashes: Object.fromEntries(Object.entries(hashes).filter(([filename]) => filename.startsWith('dist/') || filename.startsWith('media/'))),
		sha256: sha256(JSON.stringify(hashes)) };
}
async function checkoutRoot(directory) {
	let current = await fsp.realpath(directory);
	while (true) {
		try { await fsp.stat(path.join(current, '.git')); return current; } catch (error) { if (error.code !== 'ENOENT') throw error; }
		const parent = path.dirname(current);
		if (parent === current) return undefined;
		current = parent;
	}
}
async function verifyInputs(invocation) {
	const pinsFile = await jsonFile(invocation.pinsPath, 1024 * 1024);
	const pins = parsePins(pinsFile.value);
	requireInput(pins.vscode.platform === `${process.platform}-${process.arch}`, 'Pinned VS Code platform does not match this host');
	const scratch = await fsp.realpath(invocation.scratch);
	requireInput((await fsp.stat(scratch)).isDirectory(), 'SUSPECT_NATIVE_SCRATCH must be an existing external directory');
	const sourceDirectory = await fsp.realpath(pins.extension.sourceDirectory);
	const checkout = await checkoutRoot(sourceDirectory);
	requireInput(!within(sourceDirectory, scratch) && (!checkout || !within(checkout, scratch)), 'Native scratch must be outside the source checkout');
	if (process.platform === 'darwin') requireInput(Buffer.byteLength(path.join(invocation.scratch, 'n-XXXXXX/u')) + 21 <= 103, 'Native scratch path is too long for macOS IPC; supply a shorter external parent');
	try { await fsp.lstat(invocation.out); throw new InputError('SUSPECT_NATIVE_OUT must not already exist', 'E_OUTPUT_EXISTS'); }
	catch (error) { if (error.code !== 'ENOENT') throw error; }
	const output = path.join(await fsp.realpath(path.dirname(invocation.out)), path.basename(invocation.out));
	const executable = await fileIdentity(pins.vscode.executable, pins.vscode.executableSha256);
	const appRoot = path.resolve(path.dirname(executable.realPath), process.platform === 'darwin' ? '../Resources/app' : 'resources/app');
	const appDirectory = process.platform === 'darwin' ? path.resolve(appRoot, '../../..') : path.dirname(executable.realPath);
	const toolsDirectory = await fsp.realpath(pins.tools.directory);
	for (const immutable of [sourceDirectory, toolsDirectory, appDirectory]) requireInput(!within(immutable, output) && !within(output, immutable), 'Native output must be separate from immutable source/app/tool trees');
	const cli = await fileIdentity(invocation.binary, pins.cli.sha256);
	await fsp.access(invocation.binary, fs.constants.X_OK);
	await fsp.access(pins.vscode.executable, fs.constants.X_OK);
	const archive = await fileIdentity(pins.vscode.archive, pins.vscode.archiveSha256);
	const product = await jsonFile(path.join(appRoot, 'product.json'));
	const appPackage = await jsonFile(path.join(appRoot, 'package.json'));
	requireInput(product.value.commit === pins.vscode.commit && appPackage.value.version === pins.vscode.version, 'VS Code app metadata does not match its version/commit pins');
	const lockPath = path.join(toolsDirectory, 'package-lock.json');
	const lockIdentity = await fileIdentity(lockPath, pins.tools.lockSha256);
	const lock = (await jsonFile(lockPath, 16 * 1024 * 1024)).value;
	requireInput(isObject(lock.packages), 'Native tools require a package-lock with packages metadata');
	const packages = {};
	for (const name of TOOL_NAMES) {
		const locked = lock.packages[`node_modules/${name}`];
		const installed = (await jsonFile(path.join(toolsDirectory, 'node_modules', name, 'package.json'))).value;
		requireInput(isObject(locked) && typeof locked.version === 'string' && typeof locked.integrity === 'string' && installed.version === locked.version,
			`Installed native tool does not match its lock: ${name}`);
		packages[name] = { version: locked.version, integrity: locked.integrity };
	}
	const toolTree = await hashTree(path.join(toolsDirectory, 'node_modules'));
	const extension = await sourceIdentity(pins.extension.sourceDirectory);
	requireInput(extension.sha256 === pins.extension.sourceSha256, 'Editor source SHA-256 does not match pins.extension.sourceSha256', 'E_HASH');
	const vsix = pins.extension.vsix ? await fileIdentity(pins.extension.vsix.path, pins.extension.vsix.sha256) : undefined;
	const launcher = await fileIdentity(process.execPath);
	return { pins, pinsFile: { path: invocation.pinsPath, sha256: pinsFile.sha256 }, invocation,
		cli: { ...cli, expectedProfiles: [...pins.cli.expectedProfiles] },
		vscode: { executable, archive, appRoot, version: pins.vscode.version, commit: pins.vscode.commit, platform: pins.vscode.platform,
			productSha256: product.sha256, packageSha256: appPackage.sha256 },
		tools: { directory: toolsDirectory, lock: lockIdentity, packages, treeSha256: toolTree.sha256 },
		extension: { ...extension, ...(vsix ? { vsix } : {}) }, launcher: { ...launcher, version: process.version },
	};
}
async function verifyUnchanged(inputs) {
	const { invocation, pins } = inputs;
	requireInput((await jsonFile(invocation.pinsPath)).sha256 === inputs.pinsFile.sha256, 'Pins file changed during the run', 'E_INPUT_CHANGED');
	await fileIdentity(invocation.binary, pins.cli.sha256);
	await fileIdentity(pins.vscode.executable, pins.vscode.executableSha256);
	await fileIdentity(pins.vscode.archive, pins.vscode.archiveSha256);
	requireInput((await jsonFile(path.join(inputs.vscode.appRoot, 'product.json'))).sha256 === inputs.vscode.productSha256 &&
		(await jsonFile(path.join(inputs.vscode.appRoot, 'package.json'))).sha256 === inputs.vscode.packageSha256, 'VS Code metadata changed during the run', 'E_INPUT_CHANGED');
	await fileIdentity(inputs.tools.lock.path, inputs.tools.lock.sha256);
	requireInput((await hashTree(path.join(inputs.tools.directory, 'node_modules'))).sha256 === inputs.tools.treeSha256, 'Native tool files changed during the run', 'E_INPUT_CHANGED');
	requireInput((await sourceIdentity(pins.extension.sourceDirectory)).sha256 === inputs.extension.sha256, 'Editor source changed during the run', 'E_INPUT_CHANGED');
	if (inputs.extension.vsix) await fileIdentity(inputs.extension.vsix.path, inputs.extension.vsix.sha256);
}
function validateInventory(value, expectedProfiles) {
	requireInput(isObject(value) && value.format === 'suspect.sdk.profiles.v1' && Array.isArray(value.profiles) && value.profiles.length <= 12, 'Invalid CLI profile inventory', 'E_INVENTORY');
	const actual = [];
	for (const entry of value.profiles) {
		requireInput(isObject(entry) && Object.hasOwn(PROFILE_INFO, entry.profile) && entry.directory === PROFILE_INFO[entry.profile][0] &&
			typeof entry.description === 'string' && entry.description.length <= 4096 && !actual.includes(entry.profile), 'Invalid CLI profile entry', 'E_INVENTORY');
		actual.push(entry.profile);
	}
	requireInput(JSON.stringify([...actual].sort()) === JSON.stringify([...expectedProfiles].sort()),
		`Advertised profiles differ from pins; missing=${expectedProfiles.filter((profile) => !actual.includes(profile)).join(',')} extra=${actual.filter((profile) => !expectedProfiles.includes(profile)).join(',')}`, 'E_INVENTORY');
	return value;
}
function requiredChecks(scenario, mode = 'run') { return mode === 'selection-probe' ? [CHECKS.selectionProbe] : [...REQUIRED_CHECKS[scenario]]; }
function requiredScreenshots(scenario, mode = 'run') { return mode === 'selection-probe' ? ['probe-01-profile-picker.png'] : [...REQUIRED_SCREENSHOTS[scenario]]; }
function assertCheckSet(report, setup) {
	assert.equal(report.format, FORMATS.native);
	assert.equal(report.runId, setup.runId);
	assert.equal(report.pinsSha256, setup.pinsSha256);
	assert.equal(report.mode, setup.mode);
	assert.equal(report.scenario, setup.scenario);
	assert.equal(report.status, 'passed');
	assert.deepEqual(report.requiredNativeChecks, requiredChecks(setup.scenario, setup.mode));
	assert.deepEqual(report.checks.map((check) => check.name), requiredChecks(setup.scenario, setup.mode));
	assert.ok(report.checks.every((check) => check.status === 'passed'), 'Every required native check must pass; skipped/partial records are failures');
}
async function verifyInstalledSource(directory, source) {
	const metadata = await jsonFile(path.join(directory, 'package.json'));
	for (const field of ['name', 'publisher', 'version', 'main', 'type', 'engines', 'activationEvents', 'contributes', 'dependencies', 'extensionDependencies', 'extensionPack']) {
		assert.deepEqual(metadata.value[field], source.manifest[field], `Installed VSIX manifest differs from source: ${field}`);
	}
	for (const [filename, expected] of Object.entries(source.runtimeHashes)) await fileIdentity(path.join(directory, filename), expected);
	return { directory, id: `${metadata.value.publisher}.${metadata.value.name}`, version: metadata.value.version, manifestSha256: metadata.sha256, runtimeFiles: Object.keys(source.runtimeHashes).length };
}
async function verifyScreenshots(report, setup) {
	assert.ok(Array.isArray(report.screenshots), 'Native screenshot records are required');
	const names = report.screenshots.map((image) => path.basename(image.file));
	assert.equal(new Set(names).size, names.length, 'Duplicate native screenshot names');
	for (const required of requiredScreenshots(setup.scenario, setup.mode)) assert.ok(names.includes(required), `Missing native screenshot: ${required}`);
	const root = await fsp.realpath(setup.attempt);
	for (const image of report.screenshots) {
		assert.ok(within(root, await fsp.realpath(image.file)), 'Native screenshot escapes its fresh output directory');
		assert.equal(image.source, 'official VS Code Electron renderer via CDP');
		assert.ok(image.dom?.width > 0 && image.dom?.height > 0, 'A native renderer viewport is required');
		await fileIdentity(image.file, image.sha256);
		const file = await fsp.open(image.file, 'r');
		try {
			const header = Buffer.alloc(8);
			await file.read(header, 0, 8, 0);
			assert.equal(header.toString('hex'), '89504e470d0a1a0a', 'Native screenshots must be PNG files');
		} finally { await file.close(); }
	}
}

module.exports = {
	FORMATS, PROFILE_INFO, CHECKS, REQUIRED_CHECKS, REQUIRED_SCREENSHOTS, TOOL_NAMES,
	InputError, parseInvocation, parsePins, verifyInputs, verifyUnchanged, validateInventory,
	requiredChecks, requiredScreenshots, assertCheckSet, verifyInstalledSource, verifyScreenshots, fileIdentity, sourceIdentity, sha256, within,
};
