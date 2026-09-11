const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { pathToFileURL } = require('node:url');
const { readSdkSessionIdentity, startSdkSession } = require('../dist/generation.js');
const { until } = require('./sdk-fixtures.cjs');

const binary = process.env.SUSPECT_TEST_BINARY;
if (!binary || !path.isAbsolute(binary)) throw new Error('Set SUSPECT_TEST_BINARY to an absolute path to the freshly built suspect CLI');

test('real CLI session preview/check/watch preserves disk, reuses planning, reports errors and recovers', { timeout: 20000 }, async () => {
	const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect editor session '));
	let watch;
	try {
		const config = path.join(root, 'config dir/sdk session.json');
		const spec = path.join(root, 'source dir/api input.json');
		const out = path.join(root, 'generated output');
		await fs.mkdir(path.dirname(config), { recursive: true });
		await fs.mkdir(path.dirname(spec), { recursive: true });
		const source = {
			openapi: '3.1.0', info: { title: 'Session editor fixture', version: '1' },
			servers: [{ url: 'https://example.com/api/v1' }],
			security: [{ apiKey: [] }],
			components: { securitySchemes: { apiKey: { type: 'http', scheme: 'bearer' } } },
			paths: {
				'/credits': { get: { operationId: 'getCredits', responses: {
					'200': { description: 'Balance', content: { 'application/json': { schema: { type: 'number' } } } },
				} } },
				'/health': { get: { operationId: 'getHealth', security: [], responses: { '204': { description: 'empty' } } } },
			},
		};
		await fs.writeFile(spec, JSON.stringify(source, null, 2));
		await fs.writeFile(config, JSON.stringify({
			spec: '../source dir/api input.json',
			targets: [{ backend: 'typescript-http', package_name: '@fixture/session-sdk', package_version: '1.2.3' }],
			operation_ids: ['getCredits', 'getHealth'], owner: 'fixture-editor-session', cache_entries: 4, cache_bytes: 16 * 1024 * 1024,
		}));
		const identity = await readSdkSessionIdentity(config, '../generated output');
		const preview = await startSdkSession(binary, identity, { preview: true }, () => {}).done;
		assert.equal(preview.status, 'drift');
		assert.equal(preview.success, false);
		assert.equal(preview.source, path.resolve(spec));
		assert.equal(preview.output, out);
		assert.deepEqual(preview.compatibilityProfiles, []);
		assert.equal(JSON.parse(preview.artifacts.find((file) => file.path === 'typescript/package.json').content).name, '@fixture/session-sdk');
		assert.deepEqual(JSON.parse(preview.artifacts.find((file) => file.path === 'typescript/http-manifest.json').content).operations
			.map((operation) => operation.operationId).sort(), ['getCredits', 'getHealth']);
		await assert.rejects(fs.stat(out), { code: 'ENOENT' });
		// This explicitly writable helper call establishes the canonical ownership baseline.
		const written = await startSdkSession(binary, identity, {}, () => {}).done;
		assert.equal(written.status, 'written');
		const artifact = path.join(out, 'typescript/operations.ts');
		const original = await fs.readFile(artifact, 'utf8');
		const before = await fs.stat(artifact);
		const current = await startSdkSession(binary, identity, { check: true }, () => {}).done;
		assert.equal(current.status, 'current');
		assert.equal(current.artifacts, undefined);
		assert.equal((await fs.stat(artifact)).mtimeMs, before.mtimeMs);

		const records = [];
		let transportError;
		watch = startSdkSession(binary, identity, { watch: true, preview: true }, (value) => records.push(value));
		watch.done.catch((error) => { transportError = error; });
		const waitFor = async (predicate) => {
			await until(() => { if (transportError) throw transportError; return records.some(predicate); }, 'canonical CLI did not emit the expected watch result');
			return records.find(predicate);
		};
		await waitFor((value) => value.status === 'current');
		await fs.writeFile(artifact, 'caller-owned edit');
		const conflict = await waitFor((value) => value.status === 'write-conflict');
		assert.equal(conflict.delta.compiles, 0);
		assert.equal(conflict.delta.renders, 0);
		assert.equal(conflict.delta.cache_hits, 1);
		assert.ok(conflict.diagnostics.some((diagnostic) => diagnostic.code === 'ownership-conflict'));
		assert.equal(conflict.artifacts.find((file) => file.path === 'typescript/operations.ts').content, original);
		assert.equal(await fs.readFile(artifact, 'utf8'), 'caller-owned edit');
		await fs.writeFile(artifact, original);
		await waitFor((value) => value.generation > conflict.generation && value.status === 'current');
		source.paths['/credits'].get.responses['200'].content['application/json'].schema = {
			type: 'object', properties: { balance: { type: 'number' } }, required: ['balance'],
		};
		await fs.writeFile(spec, JSON.stringify(source, null, 2));
		const changed = await waitFor((value) => value.status === 'drift');
		assert.equal(changed.delta.compiles, 1);
		assert.equal(changed.delta.renders, 1);
		assert.ok(changed.changedArtifacts.length > 0);
		assert.equal(await fs.readFile(artifact, 'utf8'), original, 'watch preview must not write the new plan');
		source.paths['/credits'].get.security = [{ missing: [] }];
		const invalidSource = JSON.stringify(source, null, 2);
		await fs.writeFile(spec, invalidSource);
		const failed = await waitFor((value) => value.status === 'planning-error');
		const diagnostic = failed.diagnostics.find((value) => value.code === 'http-security-scheme-unresolved');
		assert.ok(diagnostic, JSON.stringify(failed.diagnostics));
		assert.deepEqual(diagnostic.source, { document: pathToFileURL(spec).href, pointer: '/paths/~1credits/get/security/0/missing' });
		assert.ok(Number.isSafeInteger(diagnostic.range.start) && diagnostic.range.start > 0);
		assert.ok(Number.isSafeInteger(diagnostic.range.end) && diagnostic.range.end > diagnostic.range.start);
		assert.equal(Buffer.from(invalidSource).subarray(diagnostic.range.start, diagnostic.range.end).toString(), '[]');
		assert.equal(failed.artifacts, undefined);
		delete source.paths['/credits'].get.security;
		await fs.writeFile(spec, JSON.stringify(source, null, 2));
		const recovered = await waitFor((value) => value.generation > failed.generation && value.status === 'drift');
		assert.deepEqual(recovered.artifacts, changed.artifacts);
		assert.equal(recovered.revision, changed.revision);
		assert.deepEqual(recovered.delta, { compiles: 0, renders: 0, cache_hits: 1 });
		assert.equal(await fs.readFile(artifact, 'utf8'), original);
	} finally {
		watch?.dispose();
		if (watch) await watch.done.catch(() => undefined);
		await fs.rm(root, { recursive: true, force: true });
	}
});

test('real CLI preserves a symlink entry ref base and publishes cached A→B→A revisions', { timeout: 20000 }, async () => {
	const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect editor symlink '));
	let watch;
	try {
		const config = path.join(root, 'config dir/sdk.json');
		const entry = path.join(root, 'selected source/api alias.json');
		const physicalEntry = path.join(root, 'physical source/api.json');
		const selectedModel = path.join(path.dirname(entry), 'model.json');
		const physicalModel = path.join(path.dirname(physicalEntry), 'model.json');
		for (const directory of [path.dirname(config), path.dirname(entry), path.dirname(physicalEntry)]) await fs.mkdir(directory, { recursive: true });
		await fs.writeFile(physicalEntry, JSON.stringify({
			openapi: '3.1.0', info: { title: 'Lexical entry fixture', version: '1' },
			servers: [{ url: 'https://example.com/api/v1' }], security: [{ bearer: [] }],
			components: { securitySchemes: { bearer: { type: 'http', scheme: 'bearer' } } },
			paths: { '/balance': { get: { operationId: 'getBalance', responses: {
				'200': { description: 'Balance', content: { 'application/json': { schema: { $ref: './model.json#/$defs/Balance' } } } },
			} } } },
		}));
		const model = (value) => JSON.stringify({ $defs: { Balance: {
			type: 'object', properties: { origin: { type: 'string', enum: [value] } }, required: ['origin'], additionalProperties: false,
		} } });
		const a = model('selected-alias-A');
		const b = model('selected-alias-B');
		const physical = model('wrong-physical-ref-base');
		await fs.writeFile(selectedModel, a);
		await fs.writeFile(physicalModel, physical);
		await fs.symlink(physicalEntry, entry);
		const configuration = JSON.stringify({
			spec: '../selected source/api alias.json',
			targets: [{ backend: 'typescript-http', package_name: '@fixture/lexical-sdk', package_version: '1.0.0' }],
			operation_ids: ['getBalance'], cache_entries: 4,
		});
		await fs.writeFile(config, configuration);
		const identity = await readSdkSessionIdentity(config, '../preview output');
		assert.equal(identity.sourcePath, entry);
		assert.equal(identity.sourceRoot, path.dirname(entry));
		assert.notEqual(identity.sourcePath, physicalEntry);
		// Initial planning failures now explicitly include source:null. The same process recovers.
		await fs.writeFile(config, '{unfinished');
		const records = [];
		let transportError;
		watch = startSdkSession(binary, identity, { watch: true, preview: true }, (value) => records.push(value));
		watch.done.catch((error) => { transportError = error; });
		const waitFor = async (predicate) => {
			await until(() => { if (transportError) throw transportError; return records.some(predicate); }, 'missing lexical-source/revision result from real CLI');
			return records.find(predicate);
		};
		const invalid = await waitFor((value) => value.status === 'planning-error');
		assert.equal(invalid.source, null);
		assert.equal(invalid.config, config);
		await fs.writeFile(config, configuration);
		const first = await waitFor((value) => value.status === 'drift');
		assert.equal(first.source, entry);
		assert.ok(first.newDocuments.includes(entry));
		assert.ok(first.newDocuments.includes(selectedModel));
		assert.ok(!first.newDocuments.includes(physicalModel));
		assert.ok(!first.newDocuments.includes(physicalEntry));
		assert.match(first.revision, /^[a-f0-9]{64}$/);
		const generated = first.artifacts.map((file) => file.content).join('\n');
		assert.match(generated, /selected-alias-A/);
		assert.doesNotMatch(generated, /wrong-physical-ref-base/);
		await fs.writeFile(selectedModel, b);
		const changed = await waitFor((value) => value.generation > first.generation && value.status === 'drift');
		assert.notEqual(changed.revision, first.revision);
		assert.equal(changed.delta.compiles, 1);
		assert.equal(changed.delta.renders, 1);
		assert.equal(changed.source, entry);
		assert.match(changed.artifacts.map((file) => file.content).join('\n'), /selected-alias-B/);
		await fs.writeFile(selectedModel, a);
		const reverted = await waitFor((value) => value.generation > changed.generation && value.status === 'drift');
		assert.equal(reverted.source, entry);
		assert.equal(reverted.revision, first.revision);
		assert.deepEqual(reverted.artifacts, first.artifacts);
		assert.deepEqual(reverted.changedArtifacts, first.changedArtifacts);
		assert.equal(reverted.delta.compiles, 0);
		assert.equal(reverted.delta.renders, 0);
		assert.equal(reverted.delta.cache_hits, 1);
		assert.equal(await fs.readFile(physicalModel, 'utf8'), physical);
		assert.equal((await fs.lstat(entry)).isSymbolicLink(), true);
		await assert.rejects(fs.stat(identity.outDirectory), { code: 'ENOENT' });
	} finally {
		watch?.dispose();
		if (watch) await watch.done.catch(() => undefined);
		await fs.rm(root, { recursive: true, force: true });
	}
});
