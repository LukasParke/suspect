const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { pathToFileURL } = require('node:url');
const { generationArgs, runGeneration } = require('../dist/generation.js');

const binary = process.env.SUSPECT_TEST_BINARY;
if (!binary || !path.isAbsolute(binary)) throw new Error('Set SUSPECT_TEST_BINARY to an absolute path to the freshly built suspect CLI');

/** Extracts the canonical JSON report from a runGeneration failure; the CLI prints it on stdout. */
function reportFrom(error) {
	const start = error.message.indexOf('{\n');
	const end = error.message.lastIndexOf('}');
	return JSON.parse(error.message.slice(start, end + 1));
}

// These are the same argv construction and process execution functions used by
// the extension action, consuming a real CLI rather than echoing a spawn mock.
test('editor generates expanded TypeScript operations, checks drift without writes, and retains invalid-source diagnostics', async () => {
	const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect editor '));
	try {
		const spec = path.join(root, 'api input.json');
		const out = path.join(root, 'generated output');
		const source = {
			openapi: '3.1.0', info: { title: 'Editor fixture', version: '1' },
			servers: [{ url: 'https://example.com/api/v1' }],
			security: [{ apiKey: [] }],
			components: { securitySchemes: { apiKey: { type: 'http', scheme: 'bearer' } } },
			paths: {
				'/credits': { get: { operationId: 'getCredits', responses: {
					'200': { description: 'Balance', content: { 'application/json': { schema: { type: 'object', properties: { balance: { type: 'number' } }, required: ['balance'] } } } },
				} } },
				'/health': { get: { operationId: 'getHealth', security: [], responses: { '204': { description: 'empty' } } } },
				'/invalid': { get: { operationId: 'invalid', security: [{ missing: [] }], responses: {
					'200': { description: 'Invalid security declaration', content: { 'application/json': { schema: { type: 'string' } } } },
				} } },
			},
		};
		const sourceText = JSON.stringify(source, null, 2);
		await fs.writeFile(spec, sourceText);
		const selection = { kind: 'typescript-http', packageName: '@fixture/editor-sdk', packageVersion: '1.2.3', operationIds: ['getCredits', 'getHealth'] };
		await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, check: true })), /"status": "drift"/);
		await assert.rejects(fs.stat(out), { code: 'ENOENT' });
		await runGeneration(binary, generationArgs(spec, out, selection));
		const packageRoot = path.join(out, 'typescript');
		const metadata = JSON.parse(await fs.readFile(path.join(packageRoot, 'package.json'), 'utf8'));
		assert.equal(metadata.name, '@fixture/editor-sdk');
		assert.equal(metadata.version, '1.2.3');
		const manifest = JSON.parse(await fs.readFile(path.join(packageRoot, 'http-manifest.json'), 'utf8'));
		assert.deepEqual(manifest.operations.map((operation) => operation.operationId).sort(), ['getCredits', 'getHealth']);
		await runGeneration(binary, generationArgs(spec, out, { ...selection, check: true }));
		const artifact = path.join(packageRoot, 'operations.ts');
		const before = await fs.readFile(artifact);
		const beforeStat = await fs.stat(artifact);
		assert.match(before.toString(), /getHealth/);
		// Leading whitespace is part of the exact selector, not shell syntax to trim.
		await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, operationIds: [' getCredits'] })), /sdk-operation-not-found/);
		await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, operationIds: ['--getCredits'] })), /sdk-operation-not-found/);
		await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, operationIds: [] })), (error) => {
			const report = reportFrom(error);
			assert.equal(report.profile, 'typescript-http');
			assert.equal(report.status, 'failed');
			assert.deepEqual(report.compatibilityProfiles, []);
			const diagnostic = report.diagnostics.find((entry) => entry.code === 'http-security-scheme-unresolved');
			assert.ok(diagnostic, JSON.stringify(report.diagnostics));
			assert.equal(diagnostic.file, pathToFileURL(spec).href);
			assert.equal(diagnostic.pointer, '/paths/~1invalid/get/security/0/missing');
			assert.ok(diagnostic.line > 1 && diagnostic.col > 0);
			assert.equal(Buffer.from(sourceText).subarray(diagnostic.range.start, diagnostic.range.end).toString(), '[]');
			return true;
		});
		assert.deepEqual(await fs.readFile(artifact), before);
		assert.equal((await fs.stat(artifact)).mtimeMs, beforeStat.mtimeMs);
		await fs.writeFile(artifact, 'user-owned edit');
		await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, check: true })), /sdk-artifact-conflict/);
		assert.equal(await fs.readFile(artifact, 'utf8'), 'user-owned edit');
	} finally {
		await fs.rm(root, { recursive: true, force: true });
	}
});

test('editor generates expanded Rust HTTP operations and reports located invalid operations without changing output', async () => {
	const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect editor rust '));
	try {
		const spec = path.join(root, 'api input.json');
		const out = path.join(root, 'generated output');
		const source = {
			openapi: '3.1.0', info: { title: 'Rust editor fixture', version: '1' },
			servers: [{ url: 'https://example.com/api/v1' }],
			security: [{ apiKey: [] }],
			components: { securitySchemes: { apiKey: { type: 'http', scheme: 'bearer' } } },
			paths: {
				'/credits': { get: { operationId: 'getCredits', responses: {
					'200': { description: 'Balance', content: { 'application/json': { schema: { type: 'number' } } } },
				} } },
				'/health': { get: { operationId: 'getHealth', security: [], responses: { '204': { description: 'empty' } } } },
				'/invalid': { get: { operationId: 'invalid', security: [{ missing: [] }], responses: {
					'200': { description: 'Invalid security declaration', content: { 'application/json': { schema: { type: 'string' } } } },
				} } },
			},
		};
		const sourceText = JSON.stringify(source, null, 2);
		await fs.writeFile(spec, sourceText);
		const selection = { kind: 'rust-http', packageName: 'fixture-editor-rust', packageVersion: '0.2.0', operationIds: ['getCredits', 'getHealth'] };
		await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, check: true })), /"status": "drift"/);
		await assert.rejects(fs.stat(out), { code: 'ENOENT' });
		await runGeneration(binary, generationArgs(spec, out, selection));
		const packageRoot = path.join(out, 'rust');
		// Package identity comes from the explicit prompts, never the OpenAPI title/version.
		const metadata = JSON.parse(await fs.readFile(path.join(packageRoot, 'http-manifest.json'), 'utf8'));
		assert.equal(metadata.package.name, 'fixture-editor-rust');
		assert.equal(metadata.package.version, '0.2.0');
		assert.equal(metadata.releaseReady, false);
		assert.deepEqual(metadata.operations.map((operation) => operation.operationId).sort(), ['getCredits', 'getHealth']);
		assert.ok((await fs.stat(path.join(packageRoot, 'src/operations/get_health.rs'))).isFile());
		await runGeneration(binary, generationArgs(spec, out, { ...selection, check: true }));
		const artifact = path.join(packageRoot, 'src', 'operations', 'get_credits.rs');
		const before = await fs.readFile(artifact);
		const beforeStat = await fs.stat(artifact);
		// Leading whitespace is part of the exact selector, not shell syntax to trim.
		await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, operationIds: [' getCredits'] })), /sdk-operation-not-found/);
		await assert.rejects(runGeneration(binary, generationArgs(spec, out, { ...selection, operationIds: [] })), (error) => {
			const report = reportFrom(error);
			const diagnostic = report.diagnostics.find((entry) => entry.code === 'http-security-scheme-unresolved');
			assert.ok(diagnostic, JSON.stringify(report.diagnostics));
			assert.equal(diagnostic.file, pathToFileURL(spec).href);
			assert.equal(diagnostic.pointer, '/paths/~1invalid/get/security/0/missing');
			assert.ok(diagnostic.line > 1, `expected a located diagnostic, got line ${diagnostic.line}`);
			assert.ok(diagnostic.col > 0);
			assert.equal(Buffer.from(sourceText).subarray(diagnostic.range.start, diagnostic.range.end).toString(), '[]');
			assert.equal(report.status, 'failed');
			assert.deepEqual(report.compatibilityProfiles, []);
			return true;
		});
		assert.deepEqual(await fs.readFile(artifact), before);
		assert.equal((await fs.stat(artifact)).mtimeMs, beforeStat.mtimeMs);
		await fs.writeFile(artifact, 'caller-owned edit');
		await assert.rejects(runGeneration(binary, generationArgs(spec, out, selection)), /"status": "conflict"/);
		assert.equal(await fs.readFile(artifact, 'utf8'), 'caller-owned edit');
	} finally {
		await fs.rm(root, { recursive: true, force: true });
	}
});
