const assert = require('node:assert/strict');
const cp = require('node:child_process');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { setTimeout: delay } = require('node:timers/promises');
const { test } = require('node:test');
const { generationArgs, startSdkSession, readSdkSessionIdentity, readCurrentSdkArtifact, SDK_RECORD_LIMIT } = require('../dist/generation.js');
const { MockCli, record } = require('./sdk-fixtures.cjs');

const identity = {
	configPath: '/workspace/config dir ;$(not-a-command)/sdk.json', configDirectory: '/workspace/config dir ;$(not-a-command)',
	outDirectory: '/workspace/output dir & data', sourcePath: '/workspace/spec dir/api.json', sourceRoot: '/workspace/spec dir',
};

test('one watch process receives byte-split/coalesced NDJSON, exact argv and the latest generation', async (t) => {
	const child = new MockCli();
	const calls = [];
	t.mock.method(cp, 'spawn', (...args) => { calls.push(args); return child; });
	const received = [];
	const handle = startSdkSession('/tools/CLI path/suspect', identity, { watch: true, preview: true }, (value) => received.push(value));
	const first = record();
	const bytes = Buffer.from(`${JSON.stringify(first)}\n`);
	const accent = bytes.indexOf(Buffer.from('é'));
	child.stdout.write(bytes.subarray(0, accent + 1));
	assert.equal(received.length, 0);
	child.stdout.write(bytes.subarray(accent + 1));
	child.stdout.write(`\n${JSON.stringify(record({ generation: 2, delta: { compiles: 0, renders: 0, cache_hits: 1 } }))}\r\n${JSON.stringify(record({ generation: 3 }))}\n`);
	child.close(1);
	assert.equal((await handle.done).generation, 3);
	assert.deepEqual(received.map((value) => value.generation), [1, 2, 3]);
	assert.equal(received[0].artifacts[0].content, first.artifacts[0].content);
	assert.equal(calls.length, 1);
	assert.deepEqual(calls[0], ['/tools/CLI path/suspect', [
		'codegen-session', '--config', identity.configPath, '--out', identity.outDirectory, '--watch', '--preview', '--format', 'json',
	], { cwd: identity.configDirectory, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }]);
	assert.deepEqual(child.signals, []);
});

test('one-shot check accepts canonical error exits, pretty JSON and full source diagnostics', async (t) => {
	const child = new MockCli();
	const spawn = t.mock.method(cp, 'spawn', () => child);
	const handle = startSdkSession('suspect', identity, { check: true }, () => {});
	const diagnostic = { code: 'http-security-scheme-unresolved', message: 'missing security scheme', source: { document: '/spec/api.json', pointer: '/paths/~1invalid/get/security/0/missing' }, range: { start: { line: 20, column: 3 } } };
	child.stdout.write(JSON.stringify(record({ status: 'planning-error', artifacts: undefined, diagnostics: [diagnostic], compatibilityProfiles: ['legacy-binary-string-v1'] }), null, 2));
	child.close(1);
	const result = await handle.done;
	assert.equal(result.status, 'planning-error');
	assert.deepEqual(result.diagnostics, [diagnostic]);
	assert.deepEqual(result.compatibilityProfiles, ['legacy-binary-string-v1']);
	assert.ok(spawn.mock.calls[0].arguments[1].includes('--check'));
	assert.ok(!spawn.mock.calls[0].arguments[1].includes('--preview'));
});

test('write conflicts retain planned contents and remain explicit', async (t) => {
	const child = new MockCli();
	t.mock.method(cp, 'spawn', () => child);
	const handle = startSdkSession('suspect', identity, { preview: true }, () => {});
	child.send(record({ status: 'write-conflict', diagnostics: [{ code: 'ownership-conflict', path: 'typescript/client.ts', message: 'preserving user changes' }] }));
	child.close(1);
	const result = await handle.done;
	assert.equal(result.status, 'write-conflict');
	assert.equal(result.success, false);
	assert.match(result.artifacts[0].content, /generated/);
});

test('invalid, incompatible, unsafe or misidentified output terminates the session before delivery', async (t) => {
	const cases = [
		['incompatible version', record({ format: 'suspect.sdk.session.v999' }), /expected suspect.sdk.session.v1/],
		['missing full preview', record({ artifacts: undefined }), /complete artifacts array/],
		['misleading success', record({ status: 'planning-error', success: true }), /explicit status/],
		['write in a preview', record({ status: 'written' }), /Read-only/],
		['wrong output identity', record({ output: '/other/output' }), /output root/],
		['wrong config identity', record({ config: '/other/config.json' }), /config identity/],
		['null accepted source', record({ source: null }), /explicit status/],
		['invalid revision', record({ revision: 12 }), /explicit status/],
		['unknown compatibility profile', record({ compatibilityProfiles: ['future-profile'] }), /explicit status/],
		['non-array compatibility profiles', record({ compatibilityProfiles: 'legacy-binary-string-v1' }), /explicit status/],
		['duplicate files', record({ artifacts: [{ path: 'sdk/a.ts', content: 'a' }, { path: 'sdk/a.ts', content: 'b' }] }), /unique/],
		...['../outside', '/absolute', 'sdk/../../outside', 'C:\\outside', 'sdk\\outside', 'sdk/./a.ts', 'sdk//a.ts', 'command:run'].map((name) => [name, record({ artifacts: [{ path: name, content: 'secret' }] }), /portable/]),
	];
	for (const [name, input, expected] of cases) {
		await t.test(name, async (t) => {
			const child = new MockCli();
			t.mock.method(cp, 'spawn', () => child);
			const received = [];
			const handle = startSdkSession('suspect', identity, { watch: true, preview: true }, (value) => received.push(value));
			const rejected = assert.rejects(handle.done, expected);
			child.send(input);
			child.send(record({ generation: 2 }));
			await rejected;
			assert.deepEqual(received, []);
			assert.deepEqual(child.signals, ['SIGTERM']);
		});
	}
});

test('non-increasing generations and malformed JSON cannot replace the last good preview', async (t) => {
	for (const next of [JSON.stringify(record({ generation: 3 })), 'ordinary log output', '{truncated']) {
		await t.test(next.slice(0, 30), async (t) => {
			const child = new MockCli();
			t.mock.method(cp, 'spawn', () => child);
			const received = [];
			const handle = startSdkSession('suspect', identity, { watch: true, preview: true }, (value) => received.push(value));
			const rejected = assert.rejects(handle.done);
			child.send(record({ generation: 3 }));
			child.stdout.write(`${next}\n`);
			await rejected;
			assert.equal(received.length, 1);
			assert.deepEqual(child.signals, ['SIGTERM']);
		});
	}
});

test('oversized records stop the process and stderr is bounded', async (t) => {
	const child = new MockCli();
	t.mock.method(cp, 'spawn', () => child);
	const received = [];
	const handle = startSdkSession('suspect', identity, { watch: true, preview: true }, (value) => received.push(value));
	const rejected = assert.rejects(handle.done, (error) => {
		assert.match(error.message, /16 MiB preview limit/);
		assert.ok(Buffer.byteLength(error.message) < 8500);
		assert.ok(error.message.endsWith('stderr tail'));
		return true;
	});
	child.stderr.write(`${'unbounded diagnostic'.repeat(4000)}stderr tail`);
	child.stdout.write(Buffer.alloc(SDK_RECORD_LIMIT + 1, 32));
	child.send(record());
	await rejected;
	assert.deepEqual(received, []);
});

test('cancellation suppresses late records, is idempotent, and escalates for a stuck process', { timeout: 4000 }, async (t) => {
	const child = new MockCli({ closeOnKill: false });
	t.mock.method(cp, 'spawn', () => child);
	const received = [];
	const handle = startSdkSession('suspect', identity, { watch: true, preview: true }, (value) => received.push(value));
	child.send(record());
	handle.dispose();
	handle.dispose();
	child.send(record({ generation: 2 }));
	assert.deepEqual(child.signals, ['SIGTERM']);
	await delay(1100);
	assert.deepEqual(child.signals, ['SIGTERM', 'SIGKILL']);
	child.close(null, 'SIGKILL');
	assert.equal(await handle.done, undefined);
	assert.equal(received.length, 1);
});

test('spawn failures, missing final records and abnormal exits are transport errors', async (t) => {
	await t.test('executable missing', async (t) => {
		const child = new MockCli();
		child.pid = undefined;
		t.mock.method(cp, 'spawn', () => child);
		const handle = startSdkSession('suspect', identity, {}, () => {});
		const rejected = assert.rejects(handle.done, /ENOENT/);
		child.emit('error', new Error('spawn suspect ENOENT'));
		child.close(-2);
		await rejected;
	});
	for (const code of [0, 2]) {
		await t.test(`exit ${code}`, async (t) => {
			const child = new MockCli();
			t.mock.method(cp, 'spawn', () => child);
			const handle = startSdkSession('suspect', identity, {}, () => {});
			const rejected = assert.rejects(handle.done, /without a session record/);
			child.close(code);
			await rejected;
		});
	}
});

test('config-relative identity and disk snapshots preserve files and refuse symlink/traversal reads', async () => {
	const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect sdk identity '));
	try {
		const configDirectory = path.join(root, 'config ; $(prose)');
		await fs.mkdir(configDirectory);
		const configPath = path.join(configDirectory, 'sdk.json');
		const source = { spec: '../sources/api input.json', targets: [
			{ backend: 'python-http', package_name: 'example-sdk', package_version: '1.2.3', import_name: 'example_sdk' },
			{ backend: 'go-http', package_name: 'example.com/sdk', package_version: '1.2.3' },
			{ backend: 'swift-http', package_name: 'ExampleSDK', package_version: '1.2.3' },
		], operation_ids: [' --exact ID; $(no execution)'], compatibility_profiles: ['legacy-binary-string-v1'] };
		await fs.writeFile(configPath, JSON.stringify(source));
		const resolved = await readSdkSessionIdentity(configPath, '../output ; data');
		assert.equal(resolved.configDirectory, configDirectory);
		assert.equal(resolved.sourceRoot, path.join(root, 'sources'));
		assert.equal(resolved.sourcePath, path.join(root, 'sources/api input.json'));
		assert.equal(resolved.outDirectory, path.join(root, 'output ; data'));
		assert.deepEqual(await readCurrentSdkArtifact(resolved.outDirectory, 'python/models.py'), { content: '', exists: false });
		await fs.mkdir(path.join(resolved.outDirectory, 'python'), { recursive: true });
		const artifact = path.join(resolved.outDirectory, 'python/models.py');
		await fs.writeFile(artifact, 'caller-owned text\n');
		const before = await fs.stat(artifact);
		assert.deepEqual(await readCurrentSdkArtifact(resolved.outDirectory, 'python/models.py'), { content: 'caller-owned text\n', exists: true });
		assert.equal((await fs.stat(artifact)).mtimeMs, before.mtimeMs);
		await fs.symlink(configDirectory, path.join(resolved.outDirectory, 'linked'));
		await assert.rejects(readCurrentSdkArtifact(resolved.outDirectory, 'linked/sdk.json'), /symlink artifact/);
		await assert.rejects(readCurrentSdkArtifact(resolved.outDirectory, '../config/sdk.json'), /Unsafe/);
		await assert.rejects(readCurrentSdkArtifact(resolved.outDirectory, 'python'), /Not a regular text file/);
		const large = await fs.open(path.join(resolved.outDirectory, 'python/large.py'), 'w');
		try { await large.truncate(SDK_RECORD_LIMIT + 1); } finally { await large.close(); }
		await assert.rejects(readCurrentSdkArtifact(resolved.outDirectory, 'python/large.py'), /editor read limit/);
		assert.deepEqual(JSON.parse(await fs.readFile(configPath, 'utf8')), source);
	} finally { await fs.rm(root, { recursive: true, force: true }); }
});

test('generation argv requires explicit closed compatibility profiles and preserves repeated flags alongside import identity', () => {
	const generation = { kind: 'python-http', packageName: 'fixture-sdk', packageVersion: '1.2.3', importName: 'custom_sdk', operationIds: [' --exact ID; $(prose)'] };
	const ordinary = generationArgs('/source dir/api.json', '/output dir', generation);
	assert.ok(!ordinary.includes('--compatibility-profile'));
	assert.deepEqual(generationArgs('/source dir/api.json', '/output dir', {
		...generation, compatibilityProfiles: ['legacy-binary-string-v1', 'legacy-binary-string-v1'],
	}), [
		'codegen', '/source dir/api.json', '--profile', 'python-http', '--package-name', 'fixture-sdk', '--package-version', '1.2.3',
		'--out', '/output dir', '--format', 'json', '--import-name', 'custom_sdk',
		'--compatibility-profile', 'legacy-binary-string-v1', '--compatibility-profile', 'legacy-binary-string-v1',
		'--operation-id', ' --exact ID; $(prose)',
	]);
	for (const compatibilityProfiles of [['future-profile'], ['legacy-binary-string-v1', '$(touch sentinel)'], 'legacy-binary-string-v1', [null]]) {
		assert.throws(() => generationArgs('/source', '/output', { ...generation, compatibilityProfiles }), /Invalid SDK compatibility profiles/);
	}
});
