const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { pathToFileURL } = require('node:url');
const { generationArgs, runGeneration, readSdkSessionIdentity, startSdkSession } = require('../dist/generation.js');

const binary = process.env.SUSPECT_TEST_BINARY;
if (!binary || !path.isAbsolute(binary)) throw new Error('Set SUSPECT_TEST_BINARY to an absolute path to the freshly built suspect CLI');

function reportFrom(error) {
	return JSON.parse(error.message.slice(error.message.indexOf('{\n'), error.message.lastIndexOf('}') + 1));
}

test('editor emits the canonical Swift package and previews a custom module without writes', async () => {
	const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect editor swift-http '));
	try {
		const spec = path.join(root, 'api input.json');
		const out = path.join(root, 'generated output');
		await fs.writeFile(spec, JSON.stringify({
			openapi: '3.1.0', info: { title: 'Independent API identity', version: 'api-version' },
			servers: [{ url: 'https://example.com/api/v1' }], security: [{ bearer: [] }],
			components: { securitySchemes: { bearer: { type: 'http', scheme: 'bearer' } } },
			paths: { '/credits': { get: { operationId: 'getCredits', responses: {
				'200': { description: 'Balance', content: { 'application/json': { schema: { type: 'number' } } } },
			} } } },
		}));
		await runGeneration(binary, generationArgs(spec, out, {
			kind: 'swift-http', packageName: 'ExampleSDK', packageVersion: '1.2.3', operationIds: ['getCredits'],
		}));
		const config = path.join(root, 'swift session.json');
		await fs.writeFile(config, JSON.stringify({ spec: 'api input.json', operation_ids: ['getCredits'], targets: [
			{ backend: 'swift-http', package_name: 'ExampleSDK', package_version: '1.2.3', import_name: 'CustomClient' },
		] }));
		const identity = await readSdkSessionIdentity(config, 'custom module preview');
		const preview = await startSdkSession(binary, identity, { preview: true }, () => {}).done;
		assert.equal(preview.status, 'drift');
		const files = new Map(preview.artifacts.map((file) => [file.path, file.content]));
		await assert.rejects(fs.stat(identity.outDirectory), { code: 'ENOENT' });
		const written = await fs.readdir(out);
		assert.ok(written.includes('swift') && files.has('swift/Package.swift'),
			`Expected canonical swift/ artifact root. Written: ${written.join(', ')}. Preview: ${[...files.keys()].slice(0, 8).join(', ')}`);
		const manifest = JSON.parse(await fs.readFile(path.join(out, 'swift/sdk-manifest.json'), 'utf8'));
		assert.equal(manifest.package, 'ExampleSDK');
		assert.equal(manifest.module, 'ExampleSDK');
		assert.equal(manifest.version, '1.2.3');
		assert.equal(manifest.operations[0].operationId, 'getCredits');
		assert.match(await fs.readFile(path.join(out, 'swift/Package.swift'), 'utf8'), /\.library\(name: "ExampleSDK", targets: \["ExampleSDK"\]\)/);
		assert.match(await fs.readFile(path.join(out, 'swift/README.md'), 'utf8'), /ExampleSDK/);
		assert.match(files.get('swift/Package.swift'), /\.library\(name: "ExampleSDK", targets: \["CustomClient"\]\)/);
		assert.equal(JSON.parse(files.get('swift/sdk-manifest.json')).module, 'CustomClient');
		assert.ok(files.has('swift/Sources/CustomClient/Client.swift'));
		assert.ok(files.has('swift/README.md'));
	} finally { await fs.rm(root, { recursive: true, force: true }); }
});

for (const profile of ['python-http', 'go-http']) {
	test(`editor routes expanded ${profile} operations through canonical generation with truthful package identity and ownership`, async () => {
		const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), `suspect editor ${profile} `));
		try {
			const spec = path.join(root, 'api input.json');
			const out = path.join(root, 'generated output');
			const sourceText = JSON.stringify({
				openapi: '3.1.0', info: { title: 'Not a package or import identifier', version: 'unrelated API version' },
				servers: [{ url: 'https://example.com/api/v1' }], security: [{ bearer: [] }],
				components: { securitySchemes: { bearer: { type: 'http', scheme: 'bearer' } } },
				paths: {
					'/credits': { get: { operationId: 'getCredits', responses: {
						'200': { description: 'Balance', content: { 'application/json': { schema: { type: 'number' } } } },
					} } },
					'/health': { get: { operationId: 'getHealth', security: [], responses: { '204': { description: 'empty' } } } },
					'/invalid': { get: { operationId: 'invalid', security: [{ missing: [] }], responses: {
						'200': { description: 'Invalid security declaration', content: { 'application/json': { schema: { type: 'string' } } } },
					} } },
				},
			}, null, 2);
			await fs.writeFile(spec, sourceText);
			const python = profile === 'python-http';
			const selection = { kind: profile, packageName: python ? 'fixture-editor-python' : 'example.com/fixture/editor-go', packageVersion: '1.2.3', operationIds: ['getCredits', 'getHealth'] };
			await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, check: true })), /"status": "drift"/);
			await assert.rejects(fs.stat(out), { code: 'ENOENT' });
			await runGeneration(binary, generationArgs(spec, out, selection));
			const packageRoot = path.join(out, python ? 'python' : 'go');
			const manifestPath = path.join(packageRoot, python ? 'src/fixture_editor_python/http-manifest.json' : 'http-manifest.json');
			const metadata = JSON.parse(await fs.readFile(manifestPath, 'utf8'));
			assert.equal(metadata.releaseReady, false);
			assert.equal(metadata.operations[0].operationId, 'getCredits');
			assert.deepEqual(metadata.operations.map((operation) => operation.operationId).sort(), ['getCredits', 'getHealth']);
			if (python) {
				const project = await fs.readFile(path.join(packageRoot, 'pyproject.toml'), 'utf8');
				assert.ok(project.includes('name="fixture-editor-python"'));
				assert.ok(project.includes('version="1.2.3"'));
				assert.ok(project.includes('packages=["src/fixture_editor_python"]'));
				assert.equal(metadata.package, selection.packageName);
				assert.ok((await fs.stat(path.join(packageRoot, 'src/fixture_editor_python/__init__.py'))).isFile());
			} else {
				assert.equal(metadata.module, selection.packageName);
				assert.equal(metadata.package, 'sdk');
				assert.equal(metadata.version, selection.packageVersion);
				assert.match(await fs.readFile(path.join(packageRoot, 'go.mod'), 'utf8'), /^module example\.com\/fixture\/editor-go\n/);
				assert.match(await fs.readFile(path.join(packageRoot, 'operations.go'), 'utf8'), /\bpackage sdk\b/);
			}
			const readme = path.join(packageRoot, 'README.md');
			const original = await fs.readFile(readme, 'utf8');
			const before = await fs.stat(readme);
			await runGeneration(binary, generationArgs(spec, out, { ...selection, check: true }));
			assert.equal((await fs.stat(readme)).mtimeMs, before.mtimeMs);
			await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, operationIds: [' getCredits'] })), /sdk-operation-not-found/);
			await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, operationIds: [] })), (error) => {
				const report = reportFrom(error);
				assert.equal(report.profile, profile);
				assert.equal(report.status, 'failed');
				assert.deepEqual(report.compatibilityProfiles, []);
				const diagnostic = report.diagnostics.find((value) => value.code === 'http-security-scheme-unresolved');
				assert.ok(diagnostic, JSON.stringify(report.diagnostics));
				assert.equal(diagnostic.file, pathToFileURL(spec).href);
				assert.equal(diagnostic.pointer, '/paths/~1invalid/get/security/0/missing');
				assert.ok(diagnostic.line > 1);
				assert.ok(diagnostic.col > 0);
				assert.equal(Buffer.from(sourceText).subarray(diagnostic.range.start, diagnostic.range.end).toString(), '[]');
				return true;
			});
			// The picker sends distribution/module identity exactly; the canonical CLI validates it.
			await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, packageName: python ? 'httpx' : 'only-a-package-name' })), python ? /invalid import package name/ : /invalid Go module path/);
			assert.equal(await fs.readFile(readme, 'utf8'), original);
			assert.equal((await fs.stat(readme)).mtimeMs, before.mtimeMs);
			await fs.writeFile(readme, 'caller-owned documentation edit');
			await assert.rejects(runGeneration(binary, generationArgs(spec, out, selection)), /sdk-artifact-conflict/);
			assert.equal(await fs.readFile(readme, 'utf8'), 'caller-owned documentation edit');

			if (python) {
				const config = path.join(root, 'custom import session.json');
				await fs.writeFile(config, JSON.stringify({ spec: 'api input.json', operation_ids: ['getCredits'], targets: [
					{ backend: profile, package_name: selection.packageName, package_version: selection.packageVersion, import_name: 'custom_editor_sdk' },
				] }));
				const identity = await readSdkSessionIdentity(config, 'custom import preview');
				const preview = await startSdkSession(binary, identity, { preview: true }, () => {}).done;
				assert.equal(preview.status, 'drift');
				assert.ok(preview.artifacts.some((file) => file.path === 'python/src/custom_editor_sdk/__init__.py'));
				assert.ok(!preview.artifacts.some((file) => file.path.startsWith('python/src/fixture_editor_python/')));
				assert.ok(preview.artifacts.find((file) => file.path === 'python/pyproject.toml').content.includes('packages=["src/custom_editor_sdk"]'));
				await assert.rejects(fs.stat(identity.outDirectory), { code: 'ENOENT' });
			}
		} finally { await fs.rm(root, { recursive: true, force: true }); }
	});
}
