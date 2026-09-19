const assert = require('node:assert/strict');
const cp = require('node:child_process');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { promisify } = require('node:util');
const {
	FORMATS, PROFILE_INFO, CHECKS, TOOL_NAMES, InputError, parseInvocation, parsePins, verifyInputs, verifyUnchanged,
	validateInventory, requiredChecks, requiredScreenshots, assertCheckSet, verifyScreenshots, verifyInstalledSource, sourceIdentity, sha256,
} = require('./contract.cjs');

const execFile = promisify(cp.execFile);
const clone = (value) => JSON.parse(JSON.stringify(value));
const allProfiles = Object.keys(PROFILE_INFO);
const eightProfiles = allProfiles.slice(0, 8);

async function fixture(t) {
	const scratch = process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir();
	const root = await fs.mkdtemp(path.join(scratch, 'native-input-'));
	t.after(() => fs.rm(root, { recursive: true, force: true }));
	const source = path.join(root, 'repo/editors/vscode');
	const executable = process.platform === 'darwin' ? path.join(root, 'Code.app/Contents/MacOS/Code') : path.join(root, 'code-app/code');
	const appRoot = path.resolve(path.dirname(executable), process.platform === 'darwin' ? '../Resources/app' : 'resources/app');
	const tools = path.join(root, 'tools');
	const binary = path.join(root, 'versioned-cli');
	const archive = path.join(root, 'vscode.zip');
	const vsix = path.join(root, 'extension.vsix');
	const marker = path.join(root, 'unexpected-execution');
	for (const directory of [path.join(root, 'repo/.git'), path.join(source, 'src'), path.join(source, 'dist'), path.join(source, 'media'), path.dirname(executable), appRoot, tools]) await fs.mkdir(directory, { recursive: true });
	const manifest = { name: 'suspect-vscode', publisher: 'suspect', version: '0.1.0', main: './dist/extension.js',
		engines: { vscode: '^1.85.0' }, activationEvents: ['onLanguage:json'], contributes: { commands: [] }, dependencies: {} };
	await fs.writeFile(path.join(source, 'package.json'), JSON.stringify(manifest));
	await fs.writeFile(path.join(source, 'package-lock.json'), JSON.stringify({ lockfileVersion: 3, packages: { '': manifest } }));
	for (const name of ['extension', 'generation', 'runner']) {
		await fs.writeFile(path.join(source, 'src', `${name}.ts`), `export const ${name} = true;\n`);
		await fs.writeFile(path.join(source, 'dist', `${name}.js`), `exports.${name} = true;\n`);
	}
	await fs.writeFile(path.join(source, 'media/icon.svg'), '<svg/>');
	const cliBytes = `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(marker)}, 'unexpected');\n`;
	await fs.writeFile(binary, cliBytes, { mode: 0o755 });
	await fs.writeFile(executable, 'fake app executable for read-only input guards', { mode: 0o755 });
	await fs.writeFile(archive, 'fake archive input');
	await fs.writeFile(vsix, 'fake VSIX input');
	const appVersion = '1.137.0';
	const commit = 'b'.repeat(40);
	await fs.writeFile(path.join(appRoot, 'product.json'), JSON.stringify({ commit }));
	await fs.writeFile(path.join(appRoot, 'package.json'), JSON.stringify({ version: appVersion }));
	const packages = {};
	for (const name of TOOL_NAMES) {
		const directory = path.join(tools, 'node_modules', name);
		await fs.mkdir(directory, { recursive: true });
		await fs.writeFile(path.join(directory, 'package.json'), JSON.stringify({ name, version: '1.0.0' }));
		await fs.writeFile(path.join(directory, 'index.js'), 'module.exports = {};\n');
		packages[`node_modules/${name}`] = { version: '1.0.0', integrity: `sha512-${Buffer.alloc(64, 1).toString('base64')}` };
	}
	await fs.writeFile(path.join(tools, 'package-lock.json'), JSON.stringify({ lockfileVersion: 3, packages }));
	const fileHash = async (filename) => sha256(await fs.readFile(filename));
	const pins = {
		format: FORMATS.pins, cli: { sha256: await fileHash(binary), expectedProfiles: eightProfiles },
		vscode: { executable, executableSha256: await fileHash(executable), archive, archiveSha256: await fileHash(archive), version: appVersion, commit, platform: `${process.platform}-${process.arch}` },
		tools: { directory: tools, lockSha256: await fileHash(path.join(tools, 'package-lock.json')) },
		extension: { sourceDirectory: source, sourceSha256: (await sourceIdentity(source)).sha256, vsix: { path: vsix, sha256: await fileHash(vsix) } },
	};
	const pinsPath = path.join(root, 'pins.json');
	await fs.writeFile(pinsPath, JSON.stringify(pins));
	const env = { SUSPECT_NATIVE_PINS: pinsPath, SUSPECT_TEST_BINARY: binary, SUSPECT_NATIVE_OUT: path.join(root, 'fresh-out'), SUSPECT_NATIVE_SCRATCH: scratch, SUSPECT_NATIVE_SCENARIO: 'lifecycle' };
	return { root, source, tools, appRoot, binary, executable, archive, vsix, marker, pins, pinsPath, env,
		writePins: async (value) => fs.writeFile(pinsPath, JSON.stringify(value)) };
}

test('pins accept an explicit twelve-backend contract and an explicit controlled eight-backend contract', async (t) => {
	const f = await fixture(t);
	for (const expectedProfiles of [allProfiles, eightProfiles]) {
		const pins = parsePins({ ...clone(f.pins), cli: { ...f.pins.cli, expectedProfiles: [...expectedProfiles] } });
		assert.deepEqual(pins.cli.expectedProfiles, expectedProfiles);
		assert.equal(Object.isFrozen(pins.cli.expectedProfiles), true);
	}
	const inputs = await verifyInputs(parseInvocation(f.env));
	assert.equal(inputs.cli.path, f.binary);
	assert.equal(inputs.extension.sha256, f.pins.extension.sourceSha256);
	assert.equal(inputs.vscode.commit, f.pins.vscode.commit);
	assert.deepEqual(Object.keys(inputs.tools.packages), TOOL_NAMES);
	assert.equal(inputs.extension.vsix.sha256, f.pins.extension.vsix.sha256);
	await assert.rejects(fs.stat(f.env.SUSPECT_NATIVE_OUT), { code: 'ENOENT' });
	await assert.rejects(fs.stat(f.marker), { code: 'ENOENT' });
});

test('unknown, incomplete, unsafe and unpinned input documents are rejected', async (t) => {
	const f = await fixture(t);
	const cases = [
		['unknown format', (pins) => { pins.format = 'suspect.editor.native-host.pins.v999'; }],
		['unknown root field', (pins) => { pins.historicalEvidence = '/old/report'; }],
		['unknown CLI field', (pins) => { pins.cli.path = '/implicit/binary'; }],
		['missing CLI hash', (pins) => { delete pins.cli.sha256; }],
		['invalid hash', (pins) => { pins.cli.sha256 = 'UNPINNED'; }],
		['missing backend IDs', (pins) => { delete pins.cli.expectedProfiles; }],
		['duplicate backend', (pins) => { pins.cli.expectedProfiles.push('typescript-http'); }],
		['unknown backend', (pins) => { pins.cli.expectedProfiles.push('terraform-http'); }],
		['missing native fixture backend', (pins) => { pins.cli.expectedProfiles = ['typescript-http', 'go-http', 'ruby-http']; }],
		['missing VS Code archive', (pins) => { delete pins.vscode.archive; }],
		['missing VS Code commit', (pins) => { delete pins.vscode.commit; }],
		['unpinned VS Code version', (pins) => { pins.vscode.version = 'latest'; }],
		['relative executable', (pins) => { pins.vscode.executable = 'Code'; }],
		['control character path', (pins) => { pins.vscode.archive += '\n'; }],
		['unknown platform', (pins) => { pins.vscode.platform = 'unknown-host'; }],
		['missing tool lock hash', (pins) => { delete pins.tools.lockSha256; }],
		['missing editor source hash', (pins) => { delete pins.extension.sourceSha256; }],
		['missing VSIX hash', (pins) => { delete pins.extension.vsix.sha256; }],
		['unknown VSIX switch', (pins) => { pins.extension.vsix.skipVerification = true; }],
	];
	for (const [name, mutate] of cases) await t.test(name, () => { const value = clone(f.pins); mutate(value); assert.throws(() => parsePins(value), InputError); });
});

test('invocations require all explicit paths/scenario and reject obsolete fallbacks and unknown modes', async (t) => {
	const f = await fixture(t);
	for (const key of Object.keys(f.env)) await t.test(`missing ${key}`, () => {
		const env = { ...f.env }; delete env[key]; assert.throws(() => parseInvocation(env), InputError);
	});
	for (const key of ['SUSPECT_NATIVE_EVIDENCE', 'SUSPECT_NATIVE_VSIX', 'SUSPECT_NATIVE_SETUP', 'SUSPECT_NATIVE_UNKNOWN']) await t.test(key, () => assert.throws(() => parseInvocation({ ...f.env, [key]: '/old/value' }), /Unknown native host input/));
	assert.throws(() => parseInvocation({ ...f.env, SUSPECT_TEST_BINARY: 'suspect' }), /absolute local path/);
	assert.throws(() => parseInvocation({ ...f.env, SUSPECT_NATIVE_SCENARIO: 'all' }), /lifecycle or commands/);
	assert.throws(() => parseInvocation({ ...f.env, SUSPECT_NATIVE_MODE: 'skip' }), /run or selection-probe/);
	assert.throws(() => parseInvocation(f.env, ['--skip-native']), /takes no argv options/);
	assert.equal(parseInvocation({ ...f.env, SUSPECT_NATIVE_MODE: 'selection-probe' }).mode, 'selection-probe');
});

test('file guards reject mismatched identities before any CLI or native host execution', async (t) => {
	const cases = [
		['CLI bytes', async (f) => fs.appendFile(f.binary, '\nchanged')],
		['VS Code executable bytes', async (f) => fs.appendFile(f.executable, 'changed')],
		['VS Code archive bytes', async (f) => fs.appendFile(f.archive, 'changed')],
		['tool lock bytes', async (f) => fs.appendFile(path.join(f.tools, 'package-lock.json'), ' ')],
		['editor source bytes', async (f) => fs.appendFile(path.join(f.source, 'src/runner.ts'), 'changed')],
		['VSIX bytes', async (f) => fs.appendFile(f.vsix, 'changed')],
		['VS Code commit metadata', async (f) => fs.writeFile(path.join(f.appRoot, 'product.json'), JSON.stringify({ commit: 'c'.repeat(40) }))],
		['VS Code version metadata', async (f) => fs.writeFile(path.join(f.appRoot, 'package.json'), JSON.stringify({ version: '9.9.9' }))],
		['installed tool version', async (f) => fs.writeFile(path.join(f.tools, 'node_modules/@vscode/vsce/package.json'), JSON.stringify({ version: '9.9.9' }))],
		['missing tool', async (f) => fs.unlink(path.join(f.tools, 'node_modules/playwright-core/package.json'))],
	];
	for (const [name, mutate] of cases) await t.test(name, async (t) => {
		const f = await fixture(t); await mutate(f);
		await assert.rejects(verifyInputs(parseInvocation(f.env)));
		await assert.rejects(fs.stat(f.marker), { code: 'ENOENT' });
		await assert.rejects(fs.stat(f.env.SUSPECT_NATIVE_OUT), { code: 'ENOENT' });
	});
});

test('fresh output and short external scratch guards preserve existing files', async (t) => {
	const f = await fixture(t);
	await fs.mkdir(f.env.SUSPECT_NATIVE_OUT);
	const sentinel = path.join(f.env.SUSPECT_NATIVE_OUT, 'keep.txt');
	await fs.writeFile(sentinel, 'caller-owned evidence');
	await assert.rejects(verifyInputs(parseInvocation(f.env)), /must not already exist/);
	assert.equal(await fs.readFile(sentinel, 'utf8'), 'caller-owned evidence');
	const inside = path.join(f.root, 'repo/scratch');
	await fs.mkdir(inside);
	await assert.rejects(verifyInputs(parseInvocation({ ...f.env, SUSPECT_NATIVE_OUT: path.join(f.root, 'new-out'), SUSPECT_NATIVE_SCRATCH: inside })), /outside the source checkout/);
	await assert.rejects(verifyInputs(parseInvocation({ ...f.env, SUSPECT_NATIVE_OUT: path.join(f.tools, 'new-evidence') })), /separate from immutable/);
	if (process.platform === 'darwin') {
		const long = path.join(f.root, 'too-long-for-ipc-'.repeat(5));
		await fs.mkdir(long);
		await assert.rejects(verifyInputs(parseInvocation({ ...f.env, SUSPECT_NATIVE_OUT: path.join(f.root, 'new-out'), SUSPECT_NATIVE_SCRATCH: long })), /too long for macOS IPC/);
	}
});

test('the CLI entrypoint rejects a bad pin with exit 2 without executing the supplied binary', async (t) => {
	const f = await fixture(t);
	await f.writePins({ ...f.pins, cli: { ...f.pins.cli, sha256: '0'.repeat(64) } });
	const cleanEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('SUSPECT_NATIVE_')));
	await assert.rejects(execFile(process.execPath, [path.join(__dirname, 'run.cjs')], { env: { ...cleanEnv, ...f.env } }), (error) => {
		assert.equal(error.code, 2);
		const result = JSON.parse(error.stderr);
		assert.equal(result.format, FORMATS.error);
		assert.equal(result.status, 'failed');
		assert.equal(result.code, 'E_HASH');
		return true;
	});
	await assert.rejects(fs.stat(f.marker), { code: 'ENOENT' });
	await assert.rejects(fs.stat(f.env.SUSPECT_NATIVE_OUT), { code: 'ENOENT' });
});

test('inventory comparison is exact, order independent, and refuses an eight-profile CLI for twelve-profile pins', () => {
	const inventory = (profiles) => ({ format: 'suspect.sdk.profiles.v1', profiles: profiles.map((profile) => ({ profile, directory: PROFILE_INFO[profile][0], description: `${profile} description` })) });
	assert.equal(validateInventory(inventory(allProfiles), [...allProfiles].reverse()).profiles.length, 12);
	assert.equal(validateInventory(inventory(eightProfiles), eightProfiles).profiles.length, 8);
	assert.throws(() => validateInventory(inventory(eightProfiles), allProfiles), /missing=kotlin-http,php-http,dart-http,cpp-http/);
	assert.throws(() => validateInventory(inventory(allProfiles), eightProfiles), /extra=/);
	const duplicate = inventory(eightProfiles); duplicate.profiles.push(duplicate.profiles[0]);
	assert.throws(() => validateInventory(duplicate, eightProfiles), /Invalid CLI profile entry/);
	const bad = inventory(eightProfiles); bad.profiles[0].directory = '../outside';
	assert.throws(() => validateInventory(bad, eightProfiles), /Invalid CLI profile entry/);
	assert.throws(() => validateInventory({ format: 'legacy', profiles: [] }, eightProfiles), /Invalid CLI profile inventory/);
});

test('input/source/tool mutations after selection are detected', async (t) => {
	for (const [name, mutate] of [
		['source', (f) => fs.appendFile(path.join(f.source, 'dist/extension.js'), '\nchanged')],
		['tool with unchanged lock', (f) => fs.appendFile(path.join(f.tools, 'node_modules/playwright-core/index.js'), '\nchanged')],
		['VSIX', (f) => fs.appendFile(f.vsix, 'changed')],
		['pins serialization', (f) => fs.appendFile(f.pinsPath, '\n')],
	]) await t.test(name, async (t) => {
		const f = await fixture(t);
		const selected = await verifyInputs(parseInvocation(f.env));
		await mutate(f);
		await assert.rejects(verifyUnchanged(selected));
	});
});

test('installed package verification covers every runtime file and runtime manifest fields', async (t) => {
	const f = await fixture(t);
	const source = await sourceIdentity(f.source);
	const installed = path.join(f.root, 'installed');
	await fs.cp(f.source, installed, { recursive: true });
	await verifyInstalledSource(installed, source);
	await fs.appendFile(path.join(installed, 'media/icon.svg'), 'tampered');
	await assert.rejects(verifyInstalledSource(installed, source), /SHA-256 mismatch/);
	await fs.copyFile(path.join(f.source, 'media/icon.svg'), path.join(installed, 'media/icon.svg'));
	await fs.writeFile(path.join(installed, 'package.json'), JSON.stringify({ ...source.manifest, main: './different.js' }));
	await assert.rejects(verifyInstalledSource(installed, source), /manifest differs from source/);
});

test('required native check sets reject legacy, partial, duplicate, skipped and relabeled results', () => {
	for (const scenario of ['lifecycle', 'commands']) {
		const setup = { runId: 'test-run', pinsSha256: 'd'.repeat(64), scenario, mode: 'run' };
		const good = { format: FORMATS.native, ...setup, status: 'passed', requiredNativeChecks: requiredChecks(scenario), checks: requiredChecks(scenario).map((name) => ({ name, status: 'passed' })) };
		assertCheckSet(good, setup);
		for (const mutate of [
			(value) => { value.format = 'suspect.editor.native-host.v1'; },
			(value) => { value.checks.pop(); },
			(value) => { value.checks.push(value.checks[0]); },
			(value) => { value.checks[0].status = 'skipped'; },
			(value) => { value.checks[0].name = 'equivalent enough'; },
			(value) => { value.requiredNativeChecks = []; },
			(value) => { value.mode = 'selection-probe'; },
			(value) => { value.runId = 'old-run'; },
		]) { const value = clone(good); mutate(value); assert.throws(() => assertCheckSet(value, setup)); }
	}
	assert.deepEqual(requiredChecks('lifecycle', 'selection-probe'), [CHECKS.selectionProbe]);
	assert.notDeepEqual(requiredChecks('lifecycle', 'selection-probe'), requiredChecks('lifecycle'));
});

test('required screenshot names, containment, PNG bytes and digests are verified', async (t) => {
	const f = await fixture(t);
	const attempt = path.join(f.root, 'images');
	await fs.mkdir(attempt);
	const setup = { attempt, scenario: 'commands', mode: 'run' };
	const bytes = Buffer.from('89504e470d0a1a0a00000000', 'hex');
	const screenshots = [];
	for (const filename of requiredScreenshots(setup.scenario)) {
		const file = path.join(attempt, filename); await fs.writeFile(file, bytes);
		screenshots.push({ file, sha256: sha256(bytes), source: 'official VS Code Electron renderer via CDP', dom: { width: 1440, height: 900 } });
	}
	await verifyScreenshots({ screenshots }, setup);
	await assert.rejects(verifyScreenshots({ screenshots: screenshots.slice(1) }, setup), /Missing native screenshot/);
	await assert.rejects(verifyScreenshots({ screenshots: [...screenshots, screenshots[0]] }, setup), /Duplicate/);
	const outside = path.join(f.root, path.basename(screenshots[0].file)); await fs.writeFile(outside, bytes);
	await assert.rejects(verifyScreenshots({ screenshots: [{ ...screenshots[0], file: outside }, ...screenshots.slice(1)] }, setup), /escapes/);
	await fs.writeFile(screenshots[0].file, 'not a PNG');
	await assert.rejects(verifyScreenshots({ screenshots: [{ ...screenshots[0], sha256: sha256('not a PNG') }, ...screenshots.slice(1)] }, setup), /PNG files/);
});
