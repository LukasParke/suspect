const assert = require('node:assert/strict');
const cp = require('node:child_process');
const fs = require('node:fs/promises');
const Module = require('node:module');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { MockCli, record, profileInventory, until } = require('./sdk-fixtures.cjs');

class Uri {
	constructor({ scheme, authority = '', path: pathname, query = '' }) {
		Object.assign(this, { scheme, authority, path: pathname, fsPath: pathname, query });
	}
	static file(filename) { return new Uri({ scheme: 'file', path: path.resolve(filename) }); }
	static from(parts) { return new Uri(parts); }
	toString() {
		const url = new URL(`${this.scheme}://${this.authority}/`);
		url.pathname = this.path;
		url.search = this.query;
		return url.toString();
	}
}

class Emitter {
	listeners = new Set();
	event = (listener) => { this.listeners.add(listener); return { dispose: () => this.listeners.delete(listener) }; };
	fire(value) { for (const listener of this.listeners) listener(value); }
	dispose() { this.listeners.clear(); }
}

/** Real registered commands/providers + real disk snapshots; only VS Code and the child process are replaced. */
async function editor(t) {
	const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect sdk ui '));
	const config = path.join(root, 'config dir/sdk session.json');
	const out = path.join(root, 'output dir');
	const source = path.join(root, 'sources/api input.json');
	await fs.mkdir(path.dirname(config), { recursive: true });
	await fs.mkdir(path.dirname(source), { recursive: true });
	await fs.mkdir(path.join(out, 'typescript'), { recursive: true });
	await fs.writeFile(source, '{}');
	await fs.writeFile(config, JSON.stringify({ spec: '../sources/api input.json', targets: [
		{ backend: 'typescript-http', package_name: 'ui-fixture', package_version: '1.2.3' },
	] }));
	await fs.writeFile(path.join(out, 'typescript/client.ts'), 'caller-owned disk snapshot\n');
	const commands = new Map();
	const providers = new Map();
	const diskWatchers = [];
	const diffs = [];
	const children = [];
	const spawnCalls = [];
	const discoveryCalls = [];
	const pickCalls = [];
	const errors = [];
	const prompts = [];
	const openedDocuments = [];
	const settings = new Map([['basePath', '/explicit CLI directory']]);
	const configuration = new Emitter();
	const output = { value: '', clears: 0, shows: 0, clear() { this.value = ''; this.clears++; }, appendLine(text) { this.value += `${text}\n`; }, show() { this.shows++; }, dispose() {} };
	const status = { text: '', visible: false, show() { this.visible = true; }, hide() { this.visible = false; }, dispose() { this.visible = false; } };
	const folder = { uri: Uri.file(root), name: 'fixture', index: 0 };
	const host = {
		root, config, out, source, commands, providers, diskWatchers, diffs, children, spawnCalls, discoveryCalls, pickCalls, errors, prompts, openedDocuments, settings, output, status, configuration,
		inventory: profileInventory(),
		discover: (reply) => queueMicrotask(() => reply(null, JSON.stringify(host.inventory), '')),
		input: async () => '../output dir',
		pick: async (items) => items[0],
	};
	const vscode = {
		Uri, EventEmitter: Emitter, StatusBarAlignment: { Right: 2 }, ProgressLocation: { Notification: 15 },
		RelativePattern: class { constructor(base, pattern) { Object.assign(this, { base, pattern }); } },
		workspace: {
			workspaceFolders: [folder],
			getWorkspaceFolder: (uri) => uri.fsPath.startsWith(root) ? folder : undefined,
			findFiles: async () => [],
			openTextDocument: async (filename) => ({ fileName: filename, content: await fs.readFile(filename, 'utf8') }),
			getConfiguration: () => ({ get: (key, fallback) => settings.get(key) ?? fallback }),
			registerTextDocumentContentProvider: (scheme, provider) => { providers.set(scheme, provider); return { dispose: () => providers.delete(scheme) }; },
			createFileSystemWatcher: (pattern) => {
				const change = new Emitter(), create = new Emitter(), remove = new Emitter();
				const watcher = { pattern, disposed: false, change, create, remove,
					onDidChange: change.event, onDidCreate: create.event, onDidDelete: remove.event,
					dispose() { this.disposed = true; change.dispose(); create.dispose(); remove.dispose(); } };
				diskWatchers.push(watcher);
				return watcher;
			},
			onDidChangeConfiguration: configuration.event,
		},
		window: {
			createOutputChannel: () => output,
			createStatusBarItem: () => status,
			showWorkspaceFolderPick: async () => folder,
			showOpenDialog: async () => [Uri.file(config)],
			showInputBox: (options) => { prompts.push(options); return host.input(options); },
			showQuickPick: (items, options) => { pickCalls.push([items, options]); return host.pick(items, options); },
			showErrorMessage: (message) => { errors.push(message); return Promise.resolve(); },
			showWarningMessage: (message) => { errors.push(message); return Promise.resolve(); },
			showTextDocument: async (document) => { openedDocuments.push(document); },
			withProgress: (_options, task) => {
				const cancelled = new Emitter();
				const token = { isCancellationRequested: false, onCancellationRequested: cancelled.event, cancel() { this.isCancellationRequested = true; cancelled.fire(); }, listeners: cancelled.listeners };
				host.token = token;
				return task({ report() {} }, token);
			},
		},
		commands: {
			registerCommand: (name, handler) => { commands.set(name, handler); return { dispose: () => commands.delete(name) }; },
			executeCommand: async (name, ...args) => {
				assert.equal(name, 'vscode.diff', 'the SDK integration only opens text diffs');
				diffs.push(args);
			},
		},
	};
	const originalLoad = Module._load;
	host.vscode = vscode;
	const dist = `${path.resolve(__dirname, '../dist')}${path.sep}`;
	for (const filename of Object.keys(require.cache)) if (filename.startsWith(dist)) delete require.cache[filename];
	let registerSdkGeneration;
	try {
		Module._load = function (request, ...args) {
			if (request === 'vscode') return vscode;
			if (request === 'vscode-languageclient/node') return {};
			return originalLoad.call(this, request, ...args);
		};
		({ registerSdkGeneration } = require('../dist/extension.js'));
	} finally { Module._load = originalLoad; }
	t.mock.method(cp, 'spawn', (...args) => {
		const child = new MockCli();
		children.push(child);
		spawnCalls.push(args);
		return child;
	});
	t.mock.method(cp, 'execFile', (binary, args, options, reply) => {
		discoveryCalls.push([binary, args, options]);
		return host.discover(reply);
	});
	const context = { subscriptions: [] };
	host.controller = registerSdkGeneration(context);
	host.provider = providers.get('suspect-sdk-preview');
	host.run = (command) => commands.get(command)(Uri.file(config));
	host.send = (overrides) => children.at(-1).send(record({ source, output: out, ...overrides }));
	t.after(async () => {
		host.controller.dispose();
		for (const child of children) child.close(null, 'SIGTERM');
		await fs.rm(root, { recursive: true, force: true });
	});
	return host;
}

function assertDiscovery(h) {
	assert.deepEqual(h.discoveryCalls, [['/explicit CLI directory/suspect', ['codegen-profiles', '--format', 'json'], {
		cwd: undefined, encoding: 'utf8', timeout: 5000, maxBuffer: 256 * 1024, windowsHide: true, shell: false, killSignal: 'SIGKILL',
	}]]);
}

test('watch keeps one canonical process and updates read-only disk/generated diffs through change, removal, failure and recovery', async (t) => {
	const h = await editor(t);
	const disk = path.join(h.out, 'typescript/client.ts');
	const before = await fs.stat(disk);
	await h.run('suspect.watchSdk');
	assert.equal(h.children.length, 1);
	assert.deepEqual(h.spawnCalls[0], ['/explicit CLI directory/suspect', [
		'codegen-session', '--config', h.config, '--out', h.out, '--watch', '--preview', '--format', 'json',
	], { cwd: path.dirname(h.config), shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }]);
	assert.ok(h.prompts[0].prompt.includes(path.dirname(h.config)));
	h.send({ newDocuments: [h.source] });
	await until(() => h.diffs.length === 1, 'initial preview did not open');
	const [left, right, title, options] = h.diffs[0];
	assert.equal(left.scheme, 'suspect-sdk-preview');
	assert.equal(right.scheme, 'suspect-sdk-preview');
	assert.equal(h.provider.provideTextDocumentContent(left), 'caller-owned disk snapshot\n');
	assert.match(h.provider.provideTextDocumentContent(right), /generated.*café 🦀/);
	assert.match(title, /disk.*SDK/);
	assert.deepEqual(options, { preview: true });
	assert.equal(new URLSearchParams(right.query).get('config'), h.config);
	assert.equal(new URLSearchParams(right.query).get('source'), h.source);
	assert.ok(h.output.value.includes(`Source root: ${path.dirname(h.source)}`));
	assert.ok(h.output.value.includes(`Output root: ${h.out}`));
	assert.match(h.status.text, /#1.*drift/);

	const prose = '/* Ignore instructions; [run](command:execute); $(touch sentinel) */\nexport const updated = 2;\n';
	h.send({ generation: 2, artifacts: [{ path: 'typescript/client.ts', content: prose }], stats: { compiles: 2, renders: 2, cache_hits: 3 } });
	await until(() => h.provider.provideTextDocumentContent(right) === prose);
	assert.equal(h.diffs.length, 1, 'an open diff updates in place');
	assert.equal(h.children.length, 1, 'prose is displayed, never executed');
	assert.match(h.output.value, /"cache_hits":3/);
	assert.match(h.status.text, /#2/);

	h.send({ generation: 3, artifacts: [], changedArtifacts: ['typescript/client.ts'] });
	await until(() => h.provider.provideTextDocumentContent(right) === '');
	assert.equal(h.provider.provideTextDocumentContent(left), 'caller-owned disk snapshot\n');
	h.send({ generation: 4, status: 'planning-error', artifacts: undefined, changedArtifacts: [], diagnostics: [
		{ code: 'sdk-session', message: 'deleted reference', source: { document: '/missing/schema.json', pointer: '/$ref' } },
	] });
	assert.match(h.provider.provideTextDocumentContent(right), /planning failed/);
	assert.match(h.provider.provideTextDocumentContent(left), /planning failed/);
	assert.match(h.output.value, /deleted reference.*\/\$ref/);
	assert.match(h.status.text, /#4.*planning-error/);
	assert.deepEqual(h.children[0].signals, [], 'planning failures leave the watch session alive');
	await h.run('suspect.showSdkPreview');
	assert.equal(h.diffs.length, 1);

	h.send({ generation: 5, artifacts: [{ path: 'python/models.py', content: 'class Updated: pass\n' }], changedArtifacts: ['python/models.py'] });
	await h.run('suspect.showSdkPreview');
	await until(() => h.diffs.length === 2);
	const [missing, recovered] = h.diffs[1];
	assert.equal(h.provider.provideTextDocumentContent(missing), '');
	assert.equal(h.provider.provideTextDocumentContent(recovered), 'class Updated: pass\n');
	assert.match(h.provider.provideTextDocumentContent(right), /expired/);
	assert.equal((await fs.stat(disk)).mtimeMs, before.mtimeMs);
	assert.equal(await fs.readFile(disk, 'utf8'), 'caller-owned disk snapshot\n');
	await assert.rejects(fs.stat(path.join(h.out, 'python')), { code: 'ENOENT' });
	await h.run('suspect.stopSdkWatch');
	assert.deepEqual(h.children[0].signals, ['SIGTERM']);
	assert.match(h.provider.provideTextDocumentContent(recovered), /stopped/);
	assert.equal(h.status.visible, false);
});

test('an old picker cannot open removed content, and a source-root change invalidates old virtual identities', async (t) => {
	const h = await editor(t);
	let choose;
	h.pick = () => new Promise((resolve) => { choose = resolve; });
	await h.run('suspect.watchSdk');
	h.send({});
	await until(() => choose);
	h.send({ generation: 2, artifacts: [{ path: 'swift/Sources/SDK.swift', content: 'struct New {}\n' }], changedArtifacts: ['swift/Sources/SDK.swift'] });
	choose({ label: 'typescript/client.ts' });
	h.pick = async (items) => items[0];
	await h.run('suspect.showSdkPreview');
	await until(() => h.diffs.length === 1);
	const old = h.diffs[0][1];
	assert.equal(h.provider.provideTextDocumentContent(old), 'struct New {}\n');
	const nextSource = path.join(h.root, 'other source root/api.json');
	h.send({ generation: 3, source: nextSource, artifacts: [{ path: 'swift/Sources/SDK.swift', content: 'struct OtherSource {}\n' }], changedArtifacts: ['swift/Sources/SDK.swift'] });
	await until(() => h.diffs.length === 2);
	const current = h.diffs[1][1];
	assert.equal(new URLSearchParams(current.query).get('source'), nextSource);
	assert.equal(h.provider.provideTextDocumentContent(current), 'struct OtherSource {}\n');
	assert.match(h.provider.provideTextDocumentContent(old), /expired/);
	assert.ok(h.output.value.includes(`Source root: ${path.dirname(nextSource)}`));
	assert.equal(h.children.length, 1);
});

test('replacement actions wait for termination, suppress late output, and only the newest request can launch', async (t) => {
	const h = await editor(t);
	await h.run('suspect.watchSdk');
	h.send({});
	await until(() => h.diffs.length === 1);
	const oldPreview = h.diffs[0][1];
	const old = h.children[0];
	old.closeOnKill = false;
	const superseded = h.run('suspect.watchSdk');
	const newest = h.run('suspect.checkSdk');
	old.send(record({ generation: 99, artifacts: [{ path: 'typescript/client.ts', content: 'stale' }] }));
	assert.equal(h.children.length, 1);
	assert.deepEqual(old.signals, ['SIGTERM']);
	assert.notEqual(h.provider.provideTextDocumentContent(oldPreview), 'stale');
	old.close(null, 'SIGTERM');
	await until(() => h.children.length === 2);
	assert.ok(h.spawnCalls[1][1].includes('--check'));
	assert.ok(!h.spawnCalls[1][1].includes('--watch'));
	assert.ok(!h.spawnCalls[1][1].includes('--preview'));
	h.send({ status: 'current', artifacts: undefined, changedArtifacts: [] });
	h.children[1].close(0);
	await Promise.all([superseded, newest]);
	assert.equal(h.children.length, 2);
	assert.match(h.status.text, /current/);
	assert.equal(h.diffs.length, 1, 'check does not open a generated preview');
});

test('notification cancellation and settings changes dispose subprocesses and invalidate previews', async (t) => {
	const h = await editor(t);
	const pending = h.run('suspect.previewSdk');
	await until(() => h.children.length === 1);
	h.token.cancel();
	h.send({});
	await pending;
	assert.deepEqual(h.children[0].signals, ['SIGTERM']);
	assert.equal(h.token.listeners.size, 0);
	assert.equal(h.diffs.length, 0);
	assert.deepEqual(h.errors, []);
	assert.match(h.output.value, /cancelled/);

	await h.run('suspect.watchSdk');
	h.send({});
	await until(() => h.diffs.length === 1);
	const preview = h.diffs[0][1];
	h.configuration.fire({ affectsConfiguration: (key) => key === 'suspect.basePath' });
	await until(() => h.children[1].closed);
	assert.deepEqual(h.children[1].signals, ['SIGTERM']);
	assert.match(h.provider.provideTextDocumentContent(preview), /settings changed/);
	assert.equal(h.status.visible, false);
});

test('extension disposal kills an active watch and prevents a pending dialog from launching a new session', async (t) => {
	const h = await editor(t);
	await h.run('suspect.watchSdk');
	h.send({});
	await until(() => h.diffs.length === 1);
	const preview = h.diffs[0][1];
	let resolveInput;
	h.input = () => new Promise((resolve) => { resolveInput = resolve; });
	const pending = h.run('suspect.watchSdk');
	await until(() => resolveInput);
	h.controller.dispose();
	resolveInput('../new output');
	await pending;
	assert.equal(h.children.length, 1);
	assert.deepEqual(h.children[0].signals, ['SIGTERM']);
	assert.match(h.provider.provideTextDocumentContent(preview), /disposed/);
	assert.equal(h.commands.size, 0);
});

test('latest diagnostic output is bounded; broken protocol and watch exits remove successful preview contents', async (t) => {
	const h = await editor(t);
	await h.run('suspect.watchSdk');
	h.send({});
	await until(() => h.diffs.length === 1);
	const preview = h.diffs[0][1];
	h.send({ generation: 2, status: 'planning-error', artifacts: undefined, diagnostics: [{ code: 'sdk-session', message: 'source prose '.repeat(10000) }] });
	assert.ok(h.output.value.length <= 64 * 1024);
	assert.match(h.output.value, /truncated/);
	assert.match(h.provider.provideTextDocumentContent(preview), /planning failed/);
	h.send({ generation: 3, status: 'write-conflict', diagnostics: [{ code: 'ownership-conflict', message: 'keep the user edit' }] });
	await until(() => h.provider.provideTextDocumentContent(preview).includes('generated'));
	assert.match(h.output.value, /write-conflict/);
	assert.match(h.output.value, /keep the user edit/);
	assert.ok(h.output.value.length < 2000, 'reports replace rather than append history');
	h.children[0].stdout.write('{not-json}\n');
	await until(() => h.errors.length === 1);
	assert.deepEqual(h.children[0].signals, ['SIGTERM']);
	assert.match(h.provider.provideTextDocumentContent(preview), /unavailable/);
	assert.match(h.status.text, /error/);
	await h.run('suspect.watchSdk');
	h.send({});
	await until(() => h.diffs.length === 2);
	const latest = h.diffs[1][1];
	h.children[1].close(0);
	await until(() => h.errors.length === 2);
	assert.match(h.errors[1], /watch ended/);
	assert.match(h.provider.provideTextDocumentContent(latest), /unavailable/);
});

test('a malformed initial config is diagnosed by the canonical watch and can recover in the same process', async (t) => {
	const h = await editor(t);
	await fs.writeFile(h.config, '{unfinished');
	await h.run('suspect.watchSdk');
	assert.equal(h.children.length, 1);
	h.send({ status: 'planning-error', source: null, config: h.config, artifacts: undefined,
		diagnostics: [{ code: 'sdk-session', message: 'invalid SDK session configuration' }] });
	assert.match(h.status.text, /planning-error/);
	assert.match(h.output.value, /Source root: \(unresolved\)/);
	assert.match(h.output.value, /invalid SDK session configuration/);
	assert.equal(h.diffs.length, 0);
	h.send({ generation: 2 });
	await until(() => h.diffs.length === 1);
	assert.equal(h.children.length, 1);
	assert.equal(new URLSearchParams(h.diffs[0][1].query).get('source'), h.source);
	assert.match(h.provider.provideTextDocumentContent(h.diffs[0][1]), /generated/);
});

test('multi-root selection honors the selected folder config and resolves output from the config directory', async (t) => {
	const h = await editor(t);
	const chosen = { uri: Uri.file(path.join(h.root, 'workspace B')), name: 'B', index: 1 };
	const config = path.join(chosen.uri.fsPath, 'nested/sdk.json');
	await fs.mkdir(path.dirname(config), { recursive: true });
	await fs.writeFile(config, JSON.stringify({ spec: '../source/api.json', targets: [] }));
	h.vscode.workspace.workspaceFolders = [{ uri: Uri.file(path.join(h.root, 'workspace A')), name: 'A', index: 0 }, chosen];
	h.vscode.window.showWorkspaceFolderPick = async () => chosen;
	h.vscode.workspace.getConfiguration = (_section, resource) => ({
		get: (key, fallback) => key === 'sdk.sessionConfig' ? resource?.fsPath === chosen.uri.fsPath ? 'nested/sdk.json' : 'global.json' : fallback,
	});
	h.input = async () => '../generated';
	await h.commands.get('suspect.watchSdk')();
	assert.equal(h.children.length, 1);
	const [, args, options] = h.spawnCalls[0];
	assert.equal(args[args.indexOf('--config') + 1], config);
	assert.equal(args[args.indexOf('--out') + 1], path.join(chosen.uri.fsPath, 'generated'));
	assert.equal(options.cwd, path.dirname(config));
	assert.ok(h.output.value.includes(path.join(chosen.uri.fsPath, 'source/api.json')));
});

test('repeated disk edits refresh the selected snapshot even when CLI drift status emits no new record', async (t) => {
	const h = await editor(t);
	await h.run('suspect.watchSdk');
	h.send({ status: 'write-conflict', diagnostics: [{ code: 'ownership-conflict', message: 'preserving user changes' }] });
	await until(() => h.diffs.length === 1);
	const [left, right] = h.diffs[0];
	const expected = h.provider.provideTextDocumentContent(right);
	const disk = path.join(h.out, 'typescript/client.ts');
	const watcher = h.diskWatchers.at(-1);
	assert.equal(watcher.pattern.base, path.dirname(disk));
	assert.equal(watcher.pattern.pattern, 'client.ts');
	await fs.writeFile(disk, 'second caller edit\n');
	watcher.change.fire(Uri.file(disk));
	await until(() => h.provider.provideTextDocumentContent(left) === 'second caller edit\n');
	await fs.unlink(disk);
	watcher.remove.fire(Uri.file(disk));
	await until(() => h.provider.provideTextDocumentContent(left) === '');
	await fs.writeFile(disk, 'restored caller edit\n');
	watcher.create.fire(Uri.file(disk));
	await until(() => h.provider.provideTextDocumentContent(left) === 'restored caller edit\n');
	assert.equal(h.provider.provideTextDocumentContent(right), expected);
	assert.match(h.status.text, /#1.*write-conflict/);
	assert.equal(h.children.length, 1);
	await h.run('suspect.stopSdkWatch');
	assert.equal(watcher.disposed, true);
	assert.equal(h.diskWatchers.filter((value) => !value.disposed).length, 0);
});

for (const [profile, name, directory, packagePrompt] of [
	['typescript-http', '@fixture/editor-sdk', 'typescript', /npm package name/],
	['rust-http', 'fixture-editor-rust', 'rust', /Cargo package name/],
	['python-http', 'fixture-editor-python', 'python', /Python distribution/],
	['go-http', 'example.com/fixture/editor-go', 'go', /Go module path/],
	['swift-http', 'ExampleSDK', 'swift', /valid Swift identifier/],
	['ruby-http', 'fixture-editor-ruby', 'ruby', /gem name.*require path/],
	['csharp-http', 'Fixture.Editor', 'csharp', /NuGet package ID/],
	['java-http', 'com.example:editor-java', 'java', /Maven group:artifact coordinates/],
	['kotlin-http', 'com.example:editor-kotlin', 'kotlin', /Maven group:artifact coordinates/],
	['php-http', 'example/editor-php', 'php', /Composer vendor\/package name/],
	['dart-http', 'fixture_editor_dart', 'dart', /lowercase pub package name/],
	['cpp-http', 'fixture_editor_cpp', 'cpp', /CMake target\/include identifier/],
]) {
	test(`canonical ${profile} picker uses native package prompts, exact CLI argv and the selected README`, async (t) => {
		const h = await editor(t);
		const python = profile === 'python-http';
		const swift = profile === 'swift-http';
		const operationId = ' --exact ID; $(source prose)';
		const inputs = [name, '1.2.3', JSON.stringify([operationId])];
		h.input = async () => inputs.shift();
		h.pick = async (items) => {
			assertDiscovery(h);
			assert.equal(h.children.length, 0, 'discovery completes before generation can launch');
			assert.deepEqual(items.map((item) => item.generationKind), [
				...h.inventory.profiles.map((item) => item.profile), 'docs-md', 'custom',
			]);
			assert.equal(items.find((item) => item.generationKind === profile).description,
				h.inventory.profiles.find((item) => item.profile === profile).description);
			return items.find((item) => item.generationKind === profile);
		};
		h.vscode.window.activeTextEditor = { document: { uri: Uri.file(h.source) } };
		const pending = h.run('suspect.genPreset');
		await until(() => h.children.length === 1, 'picker did not launch the canonical CLI');
		const out = path.join(h.root, 'gen-out', profile);
		assert.deepEqual(h.spawnCalls[0], ['/explicit CLI directory/suspect', [
			'codegen', h.source, '--profile', profile, '--package-name', name, '--package-version', '1.2.3',
			'--out', out, '--format', 'json', '--operation-id', operationId,
		], { cwd: h.root, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }]);
		assert.match(h.prompts[0].prompt, packagePrompt);
		if (python) {
			assert.match(h.prompts[1].prompt, /fixture_editor_python/);
			assert.match(h.prompts[1].prompt, /Configure sdk.importNames for an override/);
		} else if (swift) {
			assert.match(h.prompts[0].prompt, /default module has the same name/);
			assert.match(h.prompts[1].prompt, /stable SemVer \(no prerelease\/build metadata\)/);
			assert.match(h.prompts[1].prompt, /Swift module: ExampleSDK/);
			assert.match(h.prompts[1].prompt, /Configure sdk.importNames for an override/);
		} else if (profile === 'go-http') assert.match(h.prompts[0].prompt, /package is named sdk/);
		assert.equal(h.prompts[2].validateInput(JSON.stringify([operationId])), undefined);
		assert.match(h.prompts[2].validateInput('[1]'), /exact operation ID strings/);
		assert.equal(inputs.length, 0);
		assert.ok(!h.spawnCalls[0][1].includes('--import-name'));
		const readme = path.join(out, directory, 'README.md');
		await fs.mkdir(path.dirname(readme), { recursive: true });
		await fs.writeFile(readme, `${name} canonical package documentation`);
		h.children[0].close(0);
		await pending;
		assert.deepEqual(h.openedDocuments, [{ fileName: readme, content: `${name} canonical package documentation` }]);
		assert.deepEqual(h.errors, []);
	});
}

for (const kind of ['docs-md', 'custom']) {
	test(`${kind} picker routes to the document generator and opens its generated document`, async (t) => {
		const h = await editor(t);
		h.inventory.profiles = [];
		h.vscode.window.activeTextEditor = { document: { uri: Uri.file(h.source) } };
		h.pick = async (items) => {
			assertDiscovery(h);
			assert.deepEqual(items.map((item) => item.generationKind), ['docs-md', 'custom']);
			return items.find((item) => item.generationKind === kind);
		};
		const manifest = path.join(h.root, 'custom manifest.toml');
		await fs.writeFile(manifest, '# Custom manifest selected in the editor.\n');
		h.vscode.window.showOpenDialog = async (options) => {
			assert.deepEqual(options.filters, { 'Generation manifest': ['toml'] });
			return [Uri.file(manifest)];
		};
		const pending = h.run('suspect.genPreset');
		await until(() => h.children.length === 1);
		const out = path.join(h.root, 'gen-out', kind);
		assert.deepEqual(h.spawnCalls[0], ['/explicit CLI directory/suspect', [
			'gen', h.source, '--out', out, ...(kind === 'custom' ? ['--manifest', manifest] : ['--preset', 'docs-md']),
		], { cwd: h.root, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }]);
		assert.equal(h.prompts.length, 0, 'documentation generation does not prompt for SDK package identity');
		const produced = path.join(out, 'docs/result.md');
		await fs.mkdir(path.dirname(produced), { recursive: true });
		await fs.writeFile(produced, '# Generated documentation\n');
		h.children[0].close(0);
		await pending;
		assert.deepEqual(h.openedDocuments, [{ fileName: produced, content: '# Generated documentation\n' }]);
		assert.deepEqual(h.errors, []);
	});
}

test('profile discovery controls the menu and displays malicious description prose only as text', async (t) => {
	const h = await editor(t);
	const ruby = h.inventory.profiles.find((item) => item.profile === 'ruby-http');
	ruby.description = 'café 🦀 $(touch sentinel); [run](command:execute) <script>run()</script>';
	h.inventory.profiles = [ruby];
	h.pick = async (items) => {
		assertDiscovery(h);
		assert.deepEqual(items.map((item) => item.generationKind), ['ruby-http', 'docs-md', 'custom']);
		assert.equal(items[0].description, ruby.description);
		return undefined;
	};
	await h.run('suspect.genPreset');
	assert.equal(h.pickCalls.length, 1);
	assert.deepEqual(h.spawnCalls, []);
	assert.deepEqual(h.prompts, []);
	assert.deepEqual(h.openedDocuments, []);
	assert.deepEqual(h.errors, []);
	await assert.rejects(fs.stat(path.join(h.root, 'sentinel')), { code: 'ENOENT' });
});

test('failed, malformed, incompatible or malicious discovery never offers a partial menu or launches generation', async (t) => {
	const valid = profileInventory();
	const entry = valid.profiles[0];
	const envelope = (profiles) => ({ ...valid, profiles });
	const cases = [
		['missing executable', new Error('spawn suspect ENOENT'), valid, /ENOENT/],
		['timeout with valid stdout', new Error('SDK inventory command timed out'), valid, /timed out/],
		['failed exit with valid stdout', new Error('Command failed with code 2'), valid, /code 2/],
		['malformed JSON', null, '{unfinished', /JSON/],
		['ordinary logs before JSON', null, `log output\n${JSON.stringify(valid)}`, /JSON/],
		['incompatible version', null, { ...valid, format: 'suspect.sdk.profiles.v999' }, /Invalid SDK profile inventory/],
		['missing inventory', null, { format: valid.format }, /Invalid SDK profile inventory/],
		['null inventory', null, null, /Invalid SDK profile inventory/],
		['too many entries', null, envelope(Array(33).fill(entry)), /Invalid SDK profile inventory/],
		['duplicate profile', null, envelope([entry, entry]), /Invalid SDK profile entry/],
		['unknown profile after a valid one', null, envelope([entry, { ...entry, profile: 'terraform-http' }]), /Invalid SDK profile entry/],
		['prototype property', null, envelope([{ ...entry, profile: '__proto__' }]), /Invalid SDK profile entry/],
		['command profile', null, envelope([{ ...entry, profile: 'command:run' }]), /Invalid SDK profile entry/],
		['traversal directory', null, envelope([{ ...entry, directory: '../outside' }]), /Invalid SDK profile entry/],
		['absolute directory', null, envelope([{ ...entry, directory: '/outside' }]), /Invalid SDK profile entry/],
		['wrong native directory', null, envelope([{ ...entry, directory: 'ruby' }]), /Invalid SDK profile entry/],
		['missing description', null, envelope([{ ...entry, description: undefined }]), /Invalid SDK profile entry/],
		['non-text description', null, envelope([{ ...entry, description: { command: 'execute' } }]), /Invalid SDK profile entry/],
		['oversized description', null, envelope([{ ...entry, description: 'x'.repeat(4097) }]), /Invalid SDK profile entry/],
	];
	for (const [name, error, response, expected] of cases) {
		await t.test(name, async (t) => {
			const h = await editor(t);
			const disk = path.join(h.out, 'typescript/client.ts');
			const before = await fs.stat(disk);
			h.discover = (reply) => queueMicrotask(() => reply(error, typeof response === 'string' ? response : JSON.stringify(response), ''));
			await h.run('suspect.genPreset');
			assertDiscovery(h);
			assert.equal(h.errors.length, 1);
			assert.match(h.errors[0], /Suspect profile discovery failed/);
			assert.match(h.errors[0], expected);
			assert.deepEqual(h.pickCalls, []);
			assert.deepEqual(h.prompts, []);
			assert.deepEqual(h.spawnCalls, []);
			assert.deepEqual(h.openedDocuments, []);
			assert.equal(await fs.readFile(disk, 'utf8'), 'caller-owned disk snapshot\n');
			assert.equal((await fs.stat(disk)).mtimeMs, before.mtimeMs);
			await assert.rejects(fs.stat(path.join(h.root, 'gen-out')), { code: 'ENOENT' });
		});
	}
});

test('picker sends configured native import/module/namespace overrides exactly after discovery', async (t) => {
	for (const [profile, packageName, importName, directory] of [
		['python-http', 'fixture-editor-python', 'custom_editor_sdk', 'python'],
		['swift-http', 'ExampleSDK', 'CustomClient', 'swift'],
		['ruby-http', 'fixture-editor-ruby', 'FixtureEditor', 'ruby'],
		['csharp-http', 'Fixture.Editor', 'Fixture.Custom', 'csharp'],
		['java-http', 'com.example:editor-java', 'com.example.custom', 'java'],
		['kotlin-http', 'com.example:editor-kotlin', 'com.example.custom', 'kotlin'],
		['php-http', 'example/editor-php', 'Fixture\\Custom', 'php'],
		['cpp-http', 'fixture_editor_cpp', 'fixture_custom', 'cpp'],
	]) {
		await t.test(profile, async (t) => {
			const h = await editor(t);
			h.settings.set('sdk.importNames', { [profile]: importName, 'unselected-profile': 'unused' });
			h.vscode.window.activeTextEditor = { document: { uri: Uri.file(h.source) } };
			h.pick = async (items) => items.find((item) => item.generationKind === profile);
			const inputs = [packageName, '1.2.3', '["getCredits"]'];
			h.input = async () => inputs.shift();
			const pending = h.run('suspect.genPreset');
			await until(() => h.children.length === 1);
			assertDiscovery(h);
			const out = path.join(h.root, 'gen-out', profile);
			assert.deepEqual(h.spawnCalls[0], ['/explicit CLI directory/suspect', [
				'codegen', h.source, '--profile', profile, '--package-name', packageName, '--package-version', '1.2.3',
				'--out', out, '--format', 'json', '--import-name', importName, '--operation-id', 'getCredits',
			], { cwd: h.root, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }]);
			const readme = path.join(out, directory, 'README.md');
			await fs.mkdir(path.dirname(readme), { recursive: true });
			await fs.writeFile(readme, `Import ${importName}\n`);
			h.children[0].close(0);
			await pending;
			assert.deepEqual(h.openedDocuments, [{ fileName: readme, content: `Import ${importName}\n` }]);
			assert.deepEqual(h.errors, []);
		});
	}
});

test('a pending picker generates with the same CLI whose profiles it discovered', async (t) => {
	const h = await editor(t);
	h.vscode.window.activeTextEditor = { document: { uri: Uri.file(h.source) } };
	h.pick = async (items) => {
		h.settings.set('basePath', '/new CLI with different profiles');
		return items.find((item) => item.generationKind === 'typescript-http');
	};
	const inputs = ['@fixture/editor-sdk', '1.2.3', '[]'];
	h.input = async () => inputs.shift();
	const pending = h.run('suspect.genPreset');
	await until(() => h.children.length === 1);
	assert.equal(h.discoveryCalls[0][0], '/explicit CLI directory/suspect');
	assert.equal(h.spawnCalls[0][0], h.discoveryCalls[0][0]);
	const readme = path.join(h.root, 'gen-out/typescript-http/typescript/README.md');
	await fs.mkdir(path.dirname(readme), { recursive: true });
	await fs.writeFile(readme, 'Discovered CLI package\n');
	h.children[0].close(0);
	await pending;
	assert.deepEqual(h.openedDocuments, [{ fileName: readme, content: 'Discovered CLI package\n' }]);
	assert.deepEqual(h.errors, []);
});

test('picker enables compatibility only from explicit settings and passes it alongside the native import name', async (t) => {
	const h = await editor(t);
	h.settings.set('sdk.compatibilityProfiles', ['legacy-binary-string-v1']);
	h.settings.set('sdk.importNames', { 'python-http': 'custom_editor_sdk' });
	h.vscode.window.activeTextEditor = { document: { uri: Uri.file(h.source) } };
	h.pick = async (items) => items.find((item) => item.generationKind === 'python-http');
	const inputs = ['fixture-editor-python', '1.2.3', '[" --exact; $(prose)"]'];
	h.input = async () => inputs.shift();
	const pending = h.run('suspect.genPreset');
	await until(() => h.children.length === 1);
	assertDiscovery(h);
	const out = path.join(h.root, 'gen-out/python-http');
	assert.deepEqual(h.spawnCalls[0], ['/explicit CLI directory/suspect', [
		'codegen', h.source, '--profile', 'python-http', '--package-name', 'fixture-editor-python', '--package-version', '1.2.3',
		'--out', out, '--format', 'json', '--import-name', 'custom_editor_sdk',
		'--compatibility-profile', 'legacy-binary-string-v1', '--operation-id', ' --exact; $(prose)',
	], { cwd: h.root, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }]);
	const readme = path.join(out, 'python/README.md');
	await fs.mkdir(path.dirname(readme), { recursive: true });
	await fs.writeFile(readme, 'Explicit compatibility and custom_editor_sdk\n');
	h.children[0].close(0);
	await pending;
	assert.deepEqual(h.openedDocuments, [{ fileName: readme, content: 'Explicit compatibility and custom_editor_sdk\n' }]);
	assert.deepEqual(h.errors, []);
});

test('unknown or malformed compatibility settings fail before package prompts or generation, even if inventory advertises them', async (t) => {
	for (const compatibilityProfiles of [
		['future-profile'], ['legacy-binary-string-v1', '$(touch sentinel)'], 'legacy-binary-string-v1', [null], null,
	]) {
		await t.test(JSON.stringify(compatibilityProfiles), async (t) => {
			const h = await editor(t);
			h.inventory.compatibilityProfiles = ['future-profile', '$(touch sentinel)'];
			h.settings.set('sdk.compatibilityProfiles', compatibilityProfiles);
			// Unlike the mock's usual missing-value fallback, an explicitly malformed null is preserved.
			if (compatibilityProfiles === null) h.vscode.workspace.getConfiguration = () => ({ get: (key, fallback) => h.settings.has(key) ? h.settings.get(key) : fallback });
			h.vscode.window.activeTextEditor = { document: { uri: Uri.file(h.source) } };
			h.pick = async (items) => items.find((item) => item.generationKind === 'typescript-http');
			await h.run('suspect.genPreset');
			assertDiscovery(h);
			assert.equal(h.errors.length, 1);
			assert.match(h.errors[0], /Invalid SDK compatibility profiles/);
			assert.deepEqual(h.prompts, []);
			assert.deepEqual(h.spawnCalls, []);
			assert.deepEqual(h.openedDocuments, []);
			await assert.rejects(fs.stat(path.join(h.root, 'gen-out')), { code: 'ENOENT' });
		});
	}
});

test('session actions retain saved compatibility semantics and display the canonical report instead of applying picker settings', async (t) => {
	const h = await editor(t);
	h.settings.set('sdk.compatibilityProfiles', ['legacy-binary-string-v1']);
	const config = JSON.parse(await fs.readFile(h.config, 'utf8'));
	config.compatibility_profiles = [];
	const ordinary = JSON.stringify(config, null, 2);
	await fs.writeFile(h.config, ordinary);
	await h.run('suspect.watchSdk');
	assert.deepEqual(h.spawnCalls[0][1], ['codegen-session', '--config', h.config, '--out', h.out, '--watch', '--preview', '--format', 'json']);
	assert.deepEqual(h.discoveryCalls, []);
	h.send({ compatibilityProfiles: [] });
	await until(() => h.diffs.length === 1);
	assert.match(h.output.value, /Compatibility profiles: \[\]/);
	assert.equal(await fs.readFile(h.config, 'utf8'), ordinary);
	config.compatibility_profiles = ['legacy-binary-string-v1'];
	const explicit = JSON.stringify(config, null, 2);
	await fs.writeFile(h.config, explicit);
	h.send({ generation: 2, compatibilityProfiles: ['legacy-binary-string-v1'] });
	assert.match(h.output.value, /Compatibility profiles: \["legacy-binary-string-v1"\]/);
	assert.equal(h.children.length, 1);
	assert.equal(await fs.readFile(h.config, 'utf8'), explicit);
	assert.equal(await fs.readFile(path.join(h.out, 'typescript/client.ts'), 'utf8'), 'caller-owned disk snapshot\n');
});

test('virtual source identity stays lexical and cached A→B→A revisions refresh the same diff', async (t) => {
	const h = await editor(t);
	const entry = path.join(h.root, 'selected symlink/api alias.json');
	await fs.mkdir(path.dirname(entry), { recursive: true });
	await fs.symlink(h.source, entry);
	await fs.writeFile(h.config, JSON.stringify({ spec: '../selected symlink/api alias.json', targets: [
		{ backend: 'typescript-http', package_name: 'fixture-sdk', package_version: '1.0.0' },
	] }));
	await h.run('suspect.watchSdk');
	assert.ok(h.output.value.includes(`Source: ${entry}`));
	assert.ok(h.output.value.includes(`Source root: ${path.dirname(entry)}`));
	const a = 'export type Value = "A";\n';
	const b = 'export type Value = "B";\n';
	h.send({ source: entry, config: h.config, revision: 'a'.repeat(64), artifacts: [{ path: 'typescript/client.ts', content: a }] });
	await until(() => h.diffs.length === 1);
	const right = h.diffs[0][1];
	assert.equal(new URLSearchParams(right.query).get('source'), entry);
	assert.equal(h.provider.provideTextDocumentContent(right), a);
	h.send({ generation: 2, source: entry, config: h.config, revision: 'b'.repeat(64), artifacts: [{ path: 'typescript/client.ts', content: b }] });
	await until(() => h.provider.provideTextDocumentContent(right) === b);
	h.send({ generation: 3, source: entry, config: h.config, revision: 'a'.repeat(64),
		delta: { compiles: 0, renders: 0, cache_hits: 1 }, artifacts: [{ path: 'typescript/client.ts', content: a }] });
	await until(() => h.provider.provideTextDocumentContent(right) === a);
	assert.match(h.output.value, new RegExp(`Revision: ${'a'.repeat(64)}`));
	assert.match(h.status.text, /#3/);
	assert.equal(h.diffs.length, 1);
	assert.equal(h.children.length, 1);
});
