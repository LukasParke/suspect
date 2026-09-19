const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { pathToFileURL } = require('node:url');
const { generationArgs, runGeneration, readSdkSessionIdentity, startSdkSession } = require('../dist/generation.js');
const { until } = require('./sdk-fixtures.cjs');

const binary = process.env.SUSPECT_TEST_BINARY;
if (!binary || !path.isAbsolute(binary)) throw new Error('Set SUSPECT_TEST_BINARY to an absolute path to the freshly built suspect CLI');

async function failedReport(pending) {
	let report;
	await assert.rejects(pending, (error) => {
		report = JSON.parse(error.message.slice(error.message.indexOf('{\n'), error.message.lastIndexOf('}') + 1));
		return true;
	});
	return report;
}

for (const profile of ['typescript-http', 'python-http']) {
	test(`editor ${profile} compatibility is explicit in real CLI argv/reports and never inferred from binary source prose`, async () => {
		const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect editor compatibility '));
		try {
			const spec = path.join(root, 'binary input.json');
			const out = path.join(root, 'generated output');
			const source = JSON.stringify({
				openapi: '3.1.0', info: { title: 'legacy-binary-string-v1 is prose, not configuration', version: 'API-version' },
				servers: [{ url: 'https://example.com/v1' }],
				paths: { '/bytes': { post: { operationId: 'uploadBytes',
					requestBody: { required: true, content: { 'application/octet-stream': { schema: { type: 'string', format: 'binary' } } } },
					responses: { '200': { description: 'Bytes', content: { 'application/octet-stream': { schema: { type: 'string', format: 'binary' } } } } },
				} } },
			}, null, 2);
			await fs.writeFile(spec, source);
			const python = profile === 'python-http';
			const selection = { kind: profile, packageName: python ? 'fixture-binary-sdk' : '@fixture/binary-sdk', packageVersion: '1.2.3',
				...(python ? { importName: 'custom_binary_sdk' } : {}), operationIds: ['uploadBytes'] };
			const ordinary = await failedReport(runGeneration(binary, generationArgs(spec, out, selection), root));
			assert.equal(ordinary.status, 'failed');
			assert.deepEqual(ordinary.compatibilityProfiles, []);
			const diagnostic = ordinary.diagnostics.find((entry) => entry.code === 'http-binary-legacy-marker');
			assert.ok(diagnostic, JSON.stringify(ordinary.diagnostics));
			assert.equal(diagnostic.file, pathToFileURL(spec).href);
			assert.equal(diagnostic.pointer, '/paths/~1bytes/post/requestBody/content/application~1octet-stream/schema/format');
			assert.ok(diagnostic.line > 1 && diagnostic.col > 0);
			assert.equal(Buffer.from(source).subarray(diagnostic.range.start, diagnostic.range.end).toString(), '"binary"');
			await assert.rejects(fs.stat(out), { code: 'ENOENT' });

			const explicit = { ...selection, compatibilityProfiles: ['legacy-binary-string-v1'] };
			const preview = await failedReport(runGeneration(binary, generationArgs(spec, out, { ...explicit, check: true }), root));
			assert.equal(preview.status, 'drift');
			assert.equal(preview.profile, profile);
			assert.deepEqual(preview.compatibilityProfiles, ['legacy-binary-string-v1']);
			assert.deepEqual(preview.operations.map((operation) => operation.operationId), ['uploadBytes']);
			assert.ok(preview.artifacts.includes(`${python ? 'python' : 'typescript'}/README.md`));
			await assert.rejects(fs.stat(out), { code: 'ENOENT' });
			await runGeneration(binary, generationArgs(spec, out, explicit), root);
			const packageRoot = path.join(out, python ? 'python' : 'typescript');
			if (python) {
				assert.ok((await fs.stat(path.join(packageRoot, 'src/custom_binary_sdk/__init__.py'))).isFile());
				assert.match(await fs.readFile(path.join(packageRoot, 'pyproject.toml'), 'utf8'), /packages=\["src\/custom_binary_sdk"\]/);
			} else {
				const manifest = JSON.parse(await fs.readFile(path.join(packageRoot, 'http-manifest.json'), 'utf8'));
				assert.deepEqual(manifest.capabilities.profiles, ['legacy-binary-string-v1']);
				assert.match(await fs.readFile(path.join(packageRoot, 'operations.ts'), 'utf8'), /Uint8Array/);
			}
			const readme = path.join(packageRoot, 'README.md');
			const contents = await fs.readFile(readme, 'utf8');
			const before = await fs.stat(readme);
			await runGeneration(binary, generationArgs(spec, out, { ...explicit, check: true }), root);
			const defaultAgain = await failedReport(runGeneration(binary, generationArgs(spec, out, { ...selection, check: true }), root));
			assert.equal(defaultAgain.status, 'failed');
			assert.deepEqual(defaultAgain.compatibilityProfiles, []);
			assert.ok(defaultAgain.diagnostics.some((entry) => entry.code === 'http-binary-legacy-marker'));
			assert.equal(await fs.readFile(readme, 'utf8'), contents);
			assert.equal((await fs.stat(readme)).mtimeMs, before.mtimeMs);
			assert.equal((await fs.stat(readme)).ino, before.ino);
			assert.equal(await fs.readFile(spec, 'utf8'), source);
		} finally { await fs.rm(root, { recursive: true, force: true }); }
	});
}

test('real CLI preserves unnamed operation provenance and saved compatibility revisions through rejection and cached recovery', { timeout: 20000 }, async () => {
	const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect editor compatibility session '));
	let watch;
	try {
		const spec = path.join(root, 'api input.json');
		const config = path.join(root, 'SDK config.json');
		const out = path.join(root, 'read-only output');
		const source = JSON.stringify({
			openapi: '3.1.0', info: { title: 'Explicit session semantics', version: '1' },
			servers: [{ url: 'https://example.com/v1' }],
			paths: { '/health': { get: { responses: { '204': { description: 'empty' } } } } },
		}, null, 2);
		await fs.writeFile(spec, source);
		const selection = { kind: 'typescript-http', packageName: '@fixture/session-options', packageVersion: '1.2.3', operationIds: [], check: true };
		const all = await failedReport(runGeneration(binary, generationArgs(spec, out, selection), root));
		assert.equal(all.status, 'drift');
		assert.deepEqual(all.compatibilityProfiles, []);
		assert.deepEqual(all.operations.map(({ operationId, method, path: template }) => ({ operationId, method, path: template })), [
			{ operationId: null, method: 'GET', path: '/health' },
		]);
		assert.equal(all.operations[0].document, pathToFileURL(spec).href);
		assert.equal(all.operations[0].pointer, '/paths/~1health/get');
		const configuration = {
			spec: 'api input.json', targets: [{ backend: 'typescript-http', package_name: '@fixture/session-options', package_version: '1.2.3' }],
			operation_ids: [], compatibility_profiles: [], cache_entries: 4,
		};
		const ordinary = JSON.stringify(configuration, null, 2);
		await fs.writeFile(config, ordinary);
		const identity = await readSdkSessionIdentity(config, out);
		assert.equal(identity.sourcePath, spec);
		assert.equal(await fs.readFile(config, 'utf8'), ordinary, 'identity reads retain the entire saved config');
		const records = [];
		let transportError;
		watch = startSdkSession(binary, identity, { watch: true, preview: true }, (record) => records.push(record));
		watch.done.catch((error) => { transportError = error; });
		const waitFor = async (predicate) => {
			await until(() => { if (transportError) throw transportError; return records.some(predicate); }, 'missing compatibility revision from canonical session');
			return records.find(predicate);
		};
		const a = await waitFor((record) => record.status === 'drift');
		assert.deepEqual(a.compatibilityProfiles, []);
		const manifest = (record) => JSON.parse(record.artifacts.find((file) => file.path === 'typescript/http-manifest.json').content);
		assert.deepEqual(manifest(a).capabilities.profiles, []);
		await fs.writeFile(config, JSON.stringify({ ...configuration, compatibility_profiles: ['legacy-binary-string-v1'] }, null, 2));
		const b = await waitFor((record) => record.generation > a.generation && record.status === 'drift');
		assert.deepEqual(b.compatibilityProfiles, ['legacy-binary-string-v1']);
		assert.deepEqual(manifest(b).capabilities.profiles, ['legacy-binary-string-v1']);
		assert.notEqual(b.revision, a.revision);
		assert.equal(b.delta.renders, 1);
		await fs.writeFile(config, JSON.stringify({ ...configuration, compatibility_profiles: ['future-profile'] }, null, 2));
		const invalid = await waitFor((record) => record.generation > b.generation && record.status === 'planning-error');
		assert.equal(invalid.artifacts, undefined);
		assert.ok(invalid.diagnostics.some((diagnostic) => /future-profile/.test(diagnostic.message)), JSON.stringify(invalid.diagnostics));
		await fs.writeFile(config, ordinary);
		const recovered = await waitFor((record) => record.generation > invalid.generation && record.status === 'drift');
		assert.deepEqual(recovered.compatibilityProfiles, []);
		assert.equal(recovered.revision, a.revision);
		assert.deepEqual(recovered.artifacts, a.artifacts);
		assert.deepEqual(recovered.delta, { compiles: 0, renders: 0, cache_hits: 1 });
		await assert.rejects(fs.stat(out), { code: 'ENOENT' });
		assert.equal(await fs.readFile(spec, 'utf8'), source);
		assert.equal(await fs.readFile(config, 'utf8'), ordinary);
	} finally {
		watch?.dispose();
		if (watch) await watch.done.catch(() => undefined);
		await fs.rm(root, { recursive: true, force: true });
	}
});
