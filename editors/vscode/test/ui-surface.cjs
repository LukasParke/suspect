// Tests for the native UI surface: the pure logic behind the Project view
// and the single Testing controller — manifest reading, Arazzo discovery
// by content, test-ID construction, run grouping, CI stage parsing, the
// severity-floor resolution — plus the manifest contract for everything
// the UI contributes.

const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

// The compiled modules import 'vscode', which does not exist outside the
// extension host. Extract the pure functions by evaluating the source
// with a stub — the functions under test never touch the API.
function loadModule(name, needs, modules = new Map()) {
	const source = fs.readFileSync(path.join(__dirname, '..', 'dist', `${name}.js`), 'utf8');
	const module = { exports: {} };
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

const packageManifest = JSON.parse(
	fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8'),
);

// The shape of the user's real manifest: entry is not `openapi.yaml`, the
// suites are not `*.arazzo.yaml`.
const PLEX_LIKE = {
	version: 1,
	name: 'plex-api-spec',
	entry: 'plex-api-spec.yaml',
	publish: { output: '.suspect/spec.yaml', profiles: { cloud: ['profiles/cloud.overlay.yaml'] } },
	lint: { min_severity: 'warning' },
	codegen: [
		{ name: 'typescript', profile: 'typescript-http', package_name: '@plexapi/plex-api', package_version: '1.1.1', out: '.suspect/sdk/typescript' },
		{ name: 'go', profile: 'go-http', package_name: 'github.com/plexapi/plex-api', package_version: '1.1.1', out: '.suspect/sdk/go' },
	],
	tests: {
		arazzo: ['workflows/server-health-check.yaml', 'workflows/browse-library.yaml'],
		base_url: 'http://localhost:32400',
	},
};

test('the project manifest is read whole: entry, profiles, SDK targets, tests, floor', () => {
	const { readProjectManifest, readManifestTests } = loadModule('project', {});
	const manifest = readProjectManifest(JSON.stringify(PLEX_LIKE));
	assert.equal(manifest.name, 'plex-api-spec');
	assert.equal(manifest.entry, 'plex-api-spec.yaml');
	assert.deepEqual(manifest.profiles, [{ name: 'cloud', overlays: ['profiles/cloud.overlay.yaml'] }]);
	assert.deepEqual(manifest.codegen.map((t) => [t.name, t.profile, t.packageName, t.packageVersion]), [
		['typescript', 'typescript-http', '@plexapi/plex-api', '1.1.1'],
		['go', 'go-http', 'github.com/plexapi/plex-api', '1.1.1'],
	]);
	assert.deepEqual(manifest.tests, {
		arazzo: ['workflows/server-health-check.yaml', 'workflows/browse-library.yaml'],
		base_url: 'http://localhost:32400',
		cassette: undefined,
	});
	assert.equal(manifest.lintMinSeverity, 'warning');

	// The tests-only reader is the same reader.
	assert.deepEqual(readManifestTests(JSON.stringify(PLEX_LIKE)), manifest.tests);
	assert.equal(readManifestTests('{"name":"x"}'), undefined, 'no tests section → undefined');
	assert.equal(readProjectManifest('{not json'), undefined, 'unparseable → undefined');
	assert.equal(readProjectManifest('[1,2]'), undefined, 'non-object → undefined');

	// A cassette turns the suite offline; a minimal manifest has empty lists, not crashes.
	const offline = readProjectManifest(JSON.stringify({ tests: { arazzo: ['a.yaml'], base_url: 'http://x', cassette: 'tapes/demo' } }));
	assert.equal(offline.tests.cassette, 'tapes/demo');
	assert.deepEqual(offline.profiles, []);
	assert.deepEqual(offline.codegen, []);
	assert.equal(offline.entry, undefined);
});

test('Arazzo documents are recognised by content, not by file name', () => {
	const { looksLikeArazzo, undeclaredDocuments } = loadModule('project', {});
	assert.ok(looksLikeArazzo("arazzo: '1.0.0'\ninfo:\n  title: x\n"));
	assert.ok(looksLikeArazzo('# comment\narazzo: 1.0.1\n'));
	assert.ok(!looksLikeArazzo('openapi: 3.1.0\ninfo:\n  title: x\n'));
	assert.ok(!looksLikeArazzo('  arazzo: 1.0.0\n'), 'an indented key is not the document version');
	assert.ok(!looksLikeArazzo('description: the arazzo: format\n'));

	// Documents the manifest declares are the contract; the rest are listed apart.
	assert.deepEqual(
		undeclaredDocuments(['/p/workflows/a.yaml', '/p/workflows/b.yaml', '/p/extra.yaml'], ['/p/workflows/a.yaml', '/p/workflows/b.yaml']),
		['/p/extra.yaml'],
	);
});

test('test IDs never carry the NUL delimiter VS Code rejects', () => {
	// The exact failure the Testing view showed nothing for:
	//   Error: Test IDs may not include the "\0" symbol
	// thrown from createTestItem inside the resolve handler, before
	// items.replace ever ran.
	const { testId, TEST_ID_FORBIDDEN } = loadModule('project', {});
	assert.equal(TEST_ID_FORBIDDEN, '\u0000');
	const id = testId('suite', '/p/suspect.project.json', '/p/workflows/health.yaml', 'healthCheck', 'checkIdentity');
	assert.ok(!id.includes('\u0000'));
	assert.notEqual(
		testId('suite', '/p/a.yaml', 'wf'),
		testId('suite', '/p/a.yaml', 'wf', 'step'),
		'parent and child ids differ',
	);
	assert.throws(() => testId('a', 'b\u0000c'), /NUL/);
});

test('a run selection groups into the fewest suspect test invocations', () => {
	const { groupJobs } = loadModule('testing', {});
	const base = { cwd: '/p', baseUrl: 'http://localhost:32400' };
	const jobs = groupJobs([
		{ ...base, file: '/p/a.yaml', workflowId: 'one' },
		{ ...base, file: '/p/a.yaml', workflowId: 'two' },
		{ ...base, file: '/p/a.yaml', workflowId: 'one' },
		{ ...base, file: '/p/b.yaml' },
		{ ...base, file: '/p/b.yaml', workflowId: 'ignored-because-the-document-runs-whole' },
		{ ...base, file: '/p/c.yaml', workflowId: 'x', cassette: '/p/tapes/c' },
	]);
	assert.deepEqual(jobs.map((j) => [j.file, j.workflows, j.cassette]), [
		['/p/a.yaml', ['one', 'two'], undefined],
		['/p/b.yaml', undefined, undefined],
		['/p/c.yaml', ['x'], '/p/tapes/c'],
	]);
	assert.deepEqual(groupJobs([]), []);
});

test('CI stage results are read from suspect ci --format json', () => {
	const { parseCiStages } = loadModule('testing', {});
	const ci = parseCiStages(JSON.stringify({
		format: 'suspect.ci.v1',
		projects: [{ name: 'demo', stages: [
			{ stage: 'validate', passed: false, errors: 16, warnings: 58, summary: '/p/.suspect/spec.yaml' },
			{ stage: 'lint', passed: true, errors: 0, warnings: 745, summary: 'built-in ruleset, at or above Warning' },
			{ stage: 'breaking', passed: true, errors: 0, warnings: 0, summary: 'no --baseline: pass a git ref to gate on' },
		] }],
	}));
	assert.deepEqual(ci.map((s) => [s.stage, s.passed, s.errors, s.warnings]), [
		['validate', false, 16, 58],
		['lint', true, 0, 745],
		['breaking', true, 0, 0],
	]);
	assert.match(ci[2].summary, /baseline/);
	// Tolerance for malformed documents: the run still reports honestly.
	assert.deepEqual(parseCiStages('not json'), []);
	assert.deepEqual(parseCiStages('{"projects":[]}'), []);
	assert.deepEqual(parseCiStages('{"projects":[{"stages":"nope"}]}'), []);
});

test('the severity floor resolves by precedence and names its source', () => {
	const { resolveFloor } = loadModule('config', {});
	assert.deepEqual(resolveFloor({}), { value: 'hint', source: 'default' });
	assert.deepEqual(resolveFloor({ manifest: 'warning' }), { value: 'warning', source: 'suspect.project.json' });
	assert.deepEqual(
		resolveFloor({ manifest: 'warning', workspaceYaml: '# policy\nlint:\n  # floor\n  min_severity: error\n' }),
		{ value: 'error', source: '.suspect.yaml' },
	);
	assert.deepEqual(
		resolveFloor({ setting: 'information', manifest: 'warning', workspaceYaml: 'lint:\n  min_severity: error\n' }),
		{ value: 'information', source: 'editor setting' },
	);
	assert.deepEqual(resolveFloor({ setting: '', workspaceYaml: 'lint: {}\n' }), { value: 'hint', source: 'default' });
});

test('the extension manifest contributes one view, one controller, and commands that exist', () => {
	const { contributes } = packageManifest;
	const commands = new Set(contributes.commands.map((c) => c.command));

	// One side-bar view, with welcome content for workspaces without a manifest.
	const views = contributes.views['suspect-explorer'];
	assert.equal(views.length, 1, 'the container holds exactly one view');
	assert.equal(views[0].id, 'suspect.project');
	assert.ok(contributes.viewsWelcome.some((w) => w.view === 'suspect.project' && /suspect\.project\.json/.test(w.contents)));

	// The surfaces that were removed stay removed.
	for (const gone of ['suspect.overview', 'suspect.workflows']) {
		assert.ok(!views.some((v) => v.id === gone), `${gone} is no longer contributed`);
	}
	for (const gone of ['suspect.showOverview', 'suspect.contract.refresh', 'suspect.tests.refresh', 'suspect.workflows.refresh']) {
		assert.ok(!commands.has(gone), `${gone} is no longer a command`);
	}

	// Every command the code registers is contributed.
	const registered = new Set();
	for (const file of fs.readdirSync(path.join(__dirname, '..', 'src'))) {
		const source = fs.readFileSync(path.join(__dirname, '..', 'src', file), 'utf8');
		for (const match of source.matchAll(/registerCommand\('([^']+)'/g)) registered.add(match[1]);
	}
	for (const name of registered) {
		assert.ok(commands.has(name), `registered command ${name} is not in the manifest`);
	}

	// No menu references a command that does not exist; implementation
	// detail stays out of the palette.
	for (const entries of Object.values(contributes.menus)) {
		for (const entry of entries) {
			assert.ok(commands.has(entry.command), `menu references unknown command ${entry.command}`);
		}
	}
	const hidden = new Set(contributes.menus.commandPalette.filter((e) => e.when === 'false').map((e) => e.command));
	assert.ok(hidden.has('_suspect.toggleGateway'));
	assert.ok(hidden.has('suspect.project.refresh'));

	// The activation events cover projects without a single *.arazzo.yaml.
	assert.ok(packageManifest.activationEvents.includes('workspaceContains:**/suspect.project.json'));

	// The settings the Testing view reads exist in the shape the code expects.
	const props = contributes.configuration.properties;
	assert.deepEqual(props['suspect.lint.minSeverity'].enum, ['error', 'warning', 'information', 'hint']);
	assert.equal(props['suspect.ci.baseline'].type, 'string');
});

test('only one test controller is created', () => {
	// Two controllers with overlapping content read as two products in the
	// Testing view; the tree is one controller with groups.
	const created = [];
	for (const file of fs.readdirSync(path.join(__dirname, '..', 'src'))) {
		const source = fs.readFileSync(path.join(__dirname, '..', 'src', file), 'utf8');
		for (const match of source.matchAll(/createTestController\('([^']+)',\s*'([^']+)'\)/g)) created.push(match.slice(1));
	}
	assert.deepEqual(created, [['suspect', 'Suspect']]);
});

test('the server-side commands have editor entry points and the server settings are declared', () => {
	const { contributes } = packageManifest;
	const commands = new Set(contributes.commands.map((c) => c.command));
	// Every command the language server advertises that the extension owns
	// a surface for is contributed and registered.
	const serverSurfaces = [
		'suspect.showRefGraph', 'suspect.breakingChanges', 'suspect.contractCoverage',
		'suspect.runService', 'suspect.verifyContract', 'suspect.changeImpact',
		'suspect.generateExample', 'suspect.extractSchema', 'suspect.inlineSchema',
		'suspect.renderPreview',
	];
	for (const name of serverSurfaces) {
		assert.ok(commands.has(name), `${name} is contributed`);
	}
	// The cursor-anchored refactors are reachable from the editor context menu.
	const context = (contributes.menus['editor/context'] ?? []).map((e) => e.command);
	for (const name of ['suspect.generateExample', 'suspect.extractSchema', 'suspect.inlineSchema']) {
		assert.ok(context.includes(name), `${name} is in the editor context menu`);
	}
	// The settings the server parses are declared in the Settings UI, in
	// the shapes its configuration schema accepts.
	const props = contributes.configuration.properties;
	assert.equal(props['suspect.lint.recommended'].type, 'boolean');
	assert.equal(props['suspect.lint.rules'].type, 'object');
	assert.equal(props['suspect.lint.ruleset'].type, 'string');
	assert.equal(props['suspect.validate.strictFormat'].type, 'boolean');
	assert.equal(props['suspect.ref.maxDocs'].type, 'number');
	assert.equal(props['suspect.inlayHints.refs'].type, 'boolean');
	assert.equal(props['suspect.inlayHints.properties'].type, 'boolean');
	assert.equal(props['suspect.formatting.sortKeys'].type, 'boolean');
});

test('server commands travel over executeCommand and the admission review over its custom request', () => {
	const surface = fs.readFileSync(path.join(__dirname, '..', 'src', 'serverCommands.ts'), 'utf8');
	assert.ok(
		surface.includes("'workspace/executeCommand'"),
		'server commands route through the running language client',
	);
	assert.ok(
		surface.includes("'suspect/generationContract'"),
		'the admission review uses its custom request',
	);
	const extension = fs.readFileSync(path.join(__dirname, '..', 'src', 'extension.ts'), 'utf8');
	assert.ok(
		extension.includes('generationContract('),
		'generation consults the admission review before running',
	);
});
