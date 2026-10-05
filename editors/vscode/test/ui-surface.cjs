// Tests for the new native-UI surface: the pure logic behind the Overview
// view, the status item, and the config commands — plus the manifest
// contract for everything the new UI contributes.

const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');

// The compiled modules import 'vscode', which does not exist outside the
// extension host. Extract the pure functions by evaluating the source
// with a stub — the functions under test never touch the API.
function loadModule(name, needs, modules = new Map()) {
	const source = fs.readFileSync(path.join(__dirname, '..', 'dist', `${name}.js`), 'utf8');
	const module = { exports: {} };
	// The compiled modules require each other by path; serve those from
	// the same evaluated-module cache.
	const cache = modules;
	const wrapper = new Function('require', 'module', 'exports', source);
	const stubRequire = (what) => {
		if (what === 'vscode') return needs;
		if (what === 'fs') return require('fs');
		if (what === 'path') return require('path');
		if (what === 'child_process') return require('child_process');
		if (what.startsWith('.')) {
			const name = path.basename(what);
			if (!cache.has(name)) {
				cache.set(name, loadModule(name, needs, cache));
			}
			return cache.get(name);
		}
		throw new Error(`unexpected require: ${what}`);
	};
	wrapper(stubRequire, module, module.exports);
	return module.exports;
}

test('severity counting matches the diagnostic scale', () => {
	const { countSeverities, statusText } = loadModule('status', {});
	assert.deepEqual(
		countSeverities([{ severity: 0 }, { severity: 0 }, { severity: 1 }, { severity: 3 }]),
		{ error: 2, warning: 1, information: 0, hint: 1 },
	);
	// The blocking severities lead; a clean document says so.
	assert.match(statusText({ error: 2, warning: 1, information: 0, hint: 0 }), /error/);
	assert.match(statusText({ error: 0, warning: 0, information: 0, hint: 0 }), /check/);
});

test('project detection finds the config surface', () => {
	const { detectProjectFiles } = loadModule('overview', {});
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'suspect-ui-'));
	fs.writeFileSync(path.join(dir, 'openapi.yaml'), 'openapi: 3.1.0\n');
	fs.writeFileSync(path.join(dir, '.suspect.yaml'), 'lint: {}\n');
	fs.writeFileSync(path.join(dir, 'suspect.project.json'), '{}\n');
	fs.mkdirSync(path.join(dir, 'overlays'));
	fs.writeFileSync(path.join(dir, 'overlays', 'public.yaml'), 'overlay: 1.0.0\n');
	fs.mkdirSync(path.join(dir, 'workflows'));
	fs.writeFileSync(path.join(dir, 'workflows', 'health.arazzo.yaml'), 'arazzo: 1.0.0\n');

	const files = detectProjectFiles(dir);
	assert.equal(files.entry, 'openapi.yaml');
	assert.equal(files.configs.length, 2);
	assert.equal(files.overlays.length, 1);
	assert.equal(files.workflows.length, 1);

	const empty = detectProjectFiles(fs.mkdtempSync(path.join(os.tmpdir(), 'suspect-empty-')));
	assert.equal(empty.entry, undefined);
	assert.equal(empty.configs.length, 0);
});

test('the manifest carries the new UI surface consistently', () => {
	const manifest = JSON.parse(
		fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8'),
	);
	const commands = new Set(manifest.contributes.commands.map((c) => c.command));

	// The Overview view exists and is first in the container.
	const views = manifest.contributes.views['suspect-explorer'];
	assert.equal(views[0].id, 'suspect.overview', 'the Overview leads the activity-bar view list');

	// Every command the code registers is contributed, and none the
	// manifest contributes is private-underscored without reason.
	for (const needed of [
		'suspect.showOverview',
		'suspect.setSeverityFloor',
		'suspect.openSuspectConfig',
	]) {
		assert.ok(commands.has(needed), `command ${needed} is not in the manifest`);
	}

	// No menu references a command that does not exist.
	const menus = manifest.contributes.menus || {};
	for (const entries of Object.values(menus)) {
		for (const entry of entries) {
			if (entry.command !== undefined) {
				assert.ok(
					commands.has(entry.command),
					`menu references unknown command ${entry.command}`,
				);
			}
		}
	}

	// The floor setting is the shape the server reads live.
	const floor = manifest.contributes.configuration.properties['suspect.lint.minSeverity'];
	assert.deepEqual(floor.enum, ['error', 'warning', 'information', 'hint']);
});

test('contract suite logic: manifest tests and CI stage parsing', () => {
	const { readManifestTests, parseCiStages } = loadModule('contractSuite', {});

	// The tests section of a real manifest.
	const manifest = {
		name: 'demo', entry: 'openapi.yaml',
		tests: { arazzo: ['workflows/health.arazzo.yaml'], base_url: 'http://localhost:32400' },
	};
	const tests = readManifestTests(JSON.stringify(manifest));
	assert.deepEqual(tests, { arazzo: ['workflows/health.arazzo.yaml'], base_url: 'http://localhost:32400', cassette: undefined });
	assert.equal(readManifestTests('{"name":"x"}'), undefined, 'no tests section → undefined');
	assert.equal(readManifestTests('{not json'), undefined, 'unparseable → undefined');

	// A cassette turns the suite offline.
	const offline = readManifestTests(JSON.stringify({
		tests: { arazzo: ['a.yaml'], base_url: 'http://x', cassette: 'tapes/demo' },
	}));
	assert.equal(offline.cassette, 'tapes/demo');

	// The ci --format json document shape.
	const ci = parseCiStages(JSON.stringify({
		projects: [{ name: 'demo', stages: [
			{ stage: 'validate', passed: false, errors: 2, warnings: 58, summary: 'spec.yaml' },
			{ stage: 'lint', passed: true, errors: 0, warnings: 745, summary: 'ruleset' },
		] }],
	}));
	assert.equal(ci.length, 2);
	assert.equal(ci[0].stage, 'validate');
	assert.equal(ci[0].passed, false);
	assert.equal(ci[1].passed, true);
	// Tolerance for malformed documents: the run still reports honestly.
	assert.deepEqual(parseCiStages('not json'), []);
	assert.deepEqual(parseCiStages('{"projects":[]}'), []);
});
