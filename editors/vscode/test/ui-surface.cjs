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

// A `suspect/explainConfig` report, as the server sends it.
function report(floor, extra = {}) {
	return {
		root: '/w', config: '/w/.suspect.yaml', source: '.suspect.yaml', error: null, files: ['/w/.suspect.yaml'],
		values: [
			{ key: 'lint.min_severity', value: floor.value, origin: floor.origin, layer: floor.layer, trace: floor.trace ?? [] },
			{ key: 'gateway.port', value: 8080, origin: 'default', layer: 'default', trace: [] },
		],
		warnings: [], errors: [], ...extra,
	};
}

test('the severity floor comes from the server report and names its layer', () => {
	const { resolveFloor } = loadModule('config', {});
	// Without a report (no server, or one older than explainConfig): the
	// setting, then the manifest, then the default. No file parsing.
	assert.deepEqual(resolveFloor({}), { value: 'hint', source: 'default' });
	assert.deepEqual(resolveFloor({ manifest: 'warning' }), { value: 'warning', source: 'suspect.project.json' });
	assert.deepEqual(resolveFloor({ setting: 'info', manifest: 'warning' }), { value: 'info', source: 'editor setting' });
	// With one, the report is the answer, whatever the setting says.
	const fromFile = report({ value: 'error', origin: '/w/api/.suspect.yaml:3', layer: 'file' });
	assert.deepEqual(resolveFloor({ setting: null, report: fromFile, manifest: 'warning' }), { value: 'error', source: '.suspect.yaml' });
	const fromEditor = report({ value: 'warning', origin: 'editor suspect.lint.minSeverity', layer: 'editor' });
	assert.deepEqual(resolveFloor({ setting: 'warning', report: fromEditor }), { value: 'warning', source: 'editor setting' });
	const fromEnv = report({ value: 'info', origin: 'env SUSPECT_LINT_MIN_SEVERITY', layer: 'env' });
	assert.deepEqual(resolveFloor({ report: fromEnv }), { value: 'info', source: 'SUSPECT_LINT_MIN_SEVERITY' });
	assert.deepEqual(resolveFloor({ report: report({ value: 'hint', origin: 'default', layer: 'default' }) }), { value: 'hint', source: 'default' });
	// The regex over .suspect.yaml is gone.
	const source = fs.readFileSync(path.join(__dirname, '..', 'src', 'config.ts'), 'utf8');
	assert.ok(!source.includes('min_severity:'), 'config.ts must not parse .suspect.yaml itself');
});

test('the configuration status names the settings file and opens the explain report', async () => {
	const { statusText, renderReport, explainConfig, EXPLAIN_CONFIG_REQUEST } = loadModule('explainConfig', {});
	const floor = { value: 'warning', origin: 'editor suspect.lint.minSeverity', layer: 'editor', trace: [['/w/.suspect.yaml:2', 'error'], ['editor suspect.lint.minSeverity', 'warning']] };
	assert.equal(statusText(report(floor)).text, 'suspect: .suspect.yaml');
	assert.equal(statusText(report(floor, { config: null, source: null, files: [] })).text, 'suspect: defaults');
	const broken = statusText(report(floor, { error: '/w/.suspect.yaml:2: `lint.min_severity`: unrecognized severity `fatal`' }));
	assert.match(broken.text, /^\$\(warning\) suspect: \.suspect\.yaml$/);
	assert.match(broken.tooltip, /fatal/);
	const text = renderReport(report(floor, { errors: ['bad'] }));
	assert.match(text, /\| `lint\.min_severity` \| warning \| editor setting \| \/w\/\.suspect\.yaml:2 = error; editor suspect\.lint\.minSeverity = warning \|/);
	assert.match(text, /## Rejected values\n\n- bad/);

	// Asked only of a server that advertises the request.
	const sent = [];
	const client = (requests, answer) => ({
		initializeResult: { capabilities: { experimental: { suspect: { protocol: 1, requests, commands: {} } } } },
		sendRequest: async (method, params) => { sent.push([method, params]); return answer; },
	});
	const file = { scheme: 'file', toString: () => 'file:///w/openapi.yaml' };
	assert.equal((await explainConfig(client({ [EXPLAIN_CONFIG_REQUEST]: 1 }, report(floor)), file)).source, '.suspect.yaml');
	assert.deepEqual(sent, [['suspect/explainConfig', { uri: 'file:///w/openapi.yaml' }]]);
	assert.equal(await explainConfig(client({}, report(floor)), file), undefined);
	await explainConfig(client({ [EXPLAIN_CONFIG_REQUEST]: 1 }, report(floor)), { scheme: 'untitled' });
	assert.deepEqual(sent.at(-1), ['suspect/explainConfig', {}], 'an untitled document asks for the workspace');
	assert.equal(await explainConfig(undefined, file), undefined);
});

test('renamed settings move once, at the level they are set, without overwriting a new value', () => {
	const { pendingMoves, TARGETS } = loadModule('migrateSettings', {});
	const values = {
		'suspect.testBaseUrl': { globalValue: 'http://user:1', workspaceValue: 'http://ws:2' },
		'suspect.test.baseUrl': { workspaceValue: 'http://new:3' },
		'suspect.gatewayPort': { workspaceFolderValue: 9000 },
		'suspect.ci.baseline': { workspaceValue: '' },
	};
	const inspect = (key) => values[key];
	assert.deepEqual(pendingMoves(inspect, false), [
		{ from: 'suspect.testBaseUrl', to: 'suspect.test.baseUrl', target: TARGETS.global, value: 'http://user:1' },
	]);
	assert.deepEqual(pendingMoves(inspect, true).map((m) => [m.from, m.target]), [
		['suspect.gatewayPort', TARGETS.workspaceFolder],
		['suspect.testBaseUrl', TARGETS.global],
	]);
	assert.deepEqual(pendingMoves(() => undefined, true), []);
	const source = fs.readFileSync(path.join(__dirname, '..', 'src', 'extension.ts'), 'utf8');
	assert.match(source, /offerSettingsMigration\(context\)/);
});

test('SDK defaults: the codegen setting, then the old sdk spelling, then .suspect.yaml, then the default', () => {
	const values = new Map();
	const stub = { workspace: { getConfiguration: () => ({ get: (key) => values.get(key) }) } };
	const { codegenDefaultsFrom } = loadModule('codegenSettings', stub);
	const report = { values: [
		{ key: 'codegen.package_name', value: 'from-file' },
		{ key: 'codegen.package_version', value: '2.0.0' },
		{ key: 'codegen.import_names.python-http', value: 'file_sdk' },
		{ key: 'codegen.operation_ids', value: ['getFile'] },
		{ key: 'codegen.compatibility_profiles', value: ['colon-path-parameters-v1'] },
		{ key: 'codegen.session.config', value: '/w/sdk.json' },
	] };
	assert.deepEqual({ ...codegenDefaultsFrom(undefined) }, {
		packageName: '', packageVersion: '', importNames: {}, operationIds: [], compatibilityProfiles: [], sessionConfig: '', sessionOut: 'sdk-out',
	});
	const fromFile = codegenDefaultsFrom(report);
	assert.equal(fromFile.packageName, 'from-file');
	assert.deepEqual({ ...fromFile.importNames }, { 'python-http': 'file_sdk' });
	assert.deepEqual(fromFile.operationIds, ['getFile']);
	assert.deepEqual(fromFile.compatibilityProfiles, ['colon-path-parameters-v1']);
	assert.equal(fromFile.sessionConfig, '/w/sdk.json');
	values.set('sdk.packageName', 'old-key');
	values.set('sdk.sessionOutput', '../old-out');
	values.set('codegen.importNames', { 'ruby-http': 'Editor' });
	const edited = codegenDefaultsFrom(report);
	assert.equal(edited.packageName, 'old-key');
	assert.equal(edited.sessionOut, '../old-out');
	assert.deepEqual(edited.importNames, { 'ruby-http': 'Editor' });
	values.set('codegen.packageName', 'new-key');
	assert.equal(codegenDefaultsFrom(report).packageName, 'new-key');
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
	// Ids starting with `_` are internal and need no contribution.
	for (const name of registered) {
		assert.ok(commands.has(name) || name.startsWith('_'), `registered command ${name} is not in the manifest`);
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

	// The settings the Testing view reads exist in the shape the code expects,
	// which is the configuration schema's (test/settings.cjs pins the rest).
	const props = contributes.configuration.properties;
	const schema = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'schemas', 'suspect.schema.json'), 'utf8'));
	const lint = schema.properties.lint.properties;
	// One severity spelling across settings: minSeverity matches the rule
	// severity enum (`info`), and unset (null) defers to .suspect.yaml.
	assert.deepEqual(props['suspect.lint.minSeverity'].enum, [...lint.min_severity.enum, null]);
	assert.equal(props['suspect.lint.minSeverity'].default, null);
	assert.deepEqual(props['suspect.lint.rules'].additionalProperties.enum, lint.rules.additionalProperties.enum);
	// The legacy extension's binary setting survives as a deprecated alias.
	assert.equal(props['suspect.server.path'].type, 'string');
	assert.ok(props['suspect.server.path'].deprecationMessage.includes('suspect.basePath'));
	assert.deepEqual(props['suspect.diff.baseline'].type, [schema.properties.diff.properties.baseline.type, 'null']);
	assert.ok(props['suspect.ci.baseline'].deprecationMessage.includes('suspect.diff.baseline'));
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
	for (const [key, type] of [
		['suspect.lint.recommended', 'boolean'],
		['suspect.lint.rules', 'object'],
		['suspect.lint.design', 'string'],
		['suspect.lint.ruleset', 'string'],
		['suspect.validate.strictFormat', 'boolean'],
		['suspect.ref.maxDocs', 'integer'],
		['suspect.editor.inlayHints.refs', 'boolean'],
		['suspect.editor.inlayHints.properties', 'boolean'],
		['suspect.format.sortKeys', 'boolean'],
	]) {
		assert.deepEqual(props[key].type, [type, 'null'], key);
		assert.equal(props[key].default, null, key);
	}
	assert.equal(props['suspect.lint.design'].enum.includes('off'), true);
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

test('every server command the extension registers itself is declared in the handshake', () => {
	// vscode-languageclient registers each id in executeCommandProvider as a
	// VS Code command; a duplicate of a locally registered id throws and
	// aborts the client start. The client declares its own ids in
	// experimental.suspect.clientCommands and the server leaves them out.
	const registered = [];
	const stub = { commands: { registerCommand: (id) => { registered.push(id); return { dispose() {} }; } } };
	const modules = new Map();
	const { registerServerCommands, EDITOR_OWNED_COMMANDS, SERVER_COMMANDS, RUN_WORKFLOW_COMMAND } = loadModule('serverCommands', stub, modules);
	registerServerCommands(() => undefined);
	assert.deepEqual(registered, [...SERVER_COMMANDS]);
	assert.equal(RUN_WORKFLOW_COMMAND, 'suspect.runWorkflow');
	assert.ok(EDITOR_OWNED_COMMANDS.has(RUN_WORKFLOW_COMMAND));

	const { suspectCapabilities, clientProtocol } = loadModule('protocol', stub, modules);
	const declared = new Set(clientProtocol().clientCommands);
	for (const id of [...registered, RUN_WORKFLOW_COMMAND]) assert.ok(declared.has(id), `${id} is registered locally but not declared`);
	const capabilities = { experimental: { other: 1 } };
	suspectCapabilities('0.1.0').fillClientCapabilities(capabilities);
	assert.deepEqual(capabilities.experimental.other, 1, 'other experimental capabilities survive');
	assert.deepEqual(capabilities.experimental.suspect.clientCommands, [...EDITOR_OWNED_COMMANDS]);
	// The server compares this against its own version and warns once.
	assert.equal(capabilities.experimental.suspect.extensionVersion, '0.1.0');
	assert.equal(clientProtocol().extensionVersion, undefined);

	const extension = fs.readFileSync(path.join(__dirname, '..', 'src', 'extension.ts'), 'utf8');
	assert.ok(extension.includes('registerFeature(suspectCapabilities(extensionVersion))'), 'the client sends the declaration');
	assert.ok(!extension.includes('warnOnVersionMismatch'), 'the version check is the server\'s');
	assert.ok(!extension.includes('commands.register ='), 'the language client is not patched');
	assert.ok(extension.includes("configurationSection: 'suspect'"), 'settings changes reach the server');
});

test('features are checked against the server advertisement', () => {
	const { requestVersion, serverProtocol } = loadModule('protocol', {});
	const client = (experimental) => ({ initializeResult: { capabilities: { experimental } } });
	const advertised = client({ suspect: { protocol: 1, requests: { 'suspect/renderSdk': 1 }, commands: {} } });
	assert.equal(requestVersion(advertised, 'suspect/renderSdk'), 1);
	assert.equal(requestVersion(advertised, 'suspect/documentKind'), undefined);
	// A server older than the advertisement is protocol 0.
	assert.equal(serverProtocol(client(undefined)), undefined);
	assert.equal(requestVersion(client(undefined), 'suspect/renderSdk'), undefined);
	assert.equal(requestVersion(undefined, 'suspect/renderSdk'), undefined);
});

test('spec-only editor entries follow the server document kind, and spec files activate the extension', () => {
	const context = packageManifest.contributes.menus['editor/context'];
	for (const name of ['suspect.generateExample', 'suspect.extractSchema', 'suspect.inlineSchema']) {
		const entry = context.find((e) => e.command === name);
		assert.ok(entry, `${name} is in the editor context menu`);
		assert.match(entry.when, /suspect\.documentKind =~ \/\^\(openapi\|fragment\)\$\//, `${name} is gated on the document kind`);
	}
	const events = packageManifest.activationEvents;
	for (const glob of ['**/*.openapi.{yaml,yml,json}', '**/*.overlay.{yaml,yml}', '**/*.arazzo.{yaml,yml}']) {
		assert.ok(events.includes(`workspaceContains:${glob}`), `activates on ${glob}`);
	}
	// An OpenAPI document named api.json must still activate.
	assert.ok(events.includes('onLanguage:yaml') && events.includes('onLanguage:json'));
});

test('the document kind comes from the server, and only when it offers the request', async () => {
	const { documentKind, DOCUMENT_KIND_REQUEST } = loadModule('documentKind', {});
	const sent = [];
	const client = (requests, answer) => ({
		initializeResult: { capabilities: { experimental: { suspect: { protocol: 1, requests, commands: {} } } } },
		sendRequest: async (method, params) => { sent.push([method, params]); if (answer instanceof Error) throw answer; return answer; },
	});
	const file = { scheme: 'file', toString: () => 'file:///w/openapi.yaml' };
	assert.equal(await documentKind(client({ [DOCUMENT_KIND_REQUEST]: 1 }, { kind: 'openapi', version: '3.1' }), file), 'openapi');
	assert.deepEqual(sent, [['suspect/documentKind', { uri: 'file:///w/openapi.yaml' }]]);
	// A server that predates the request is not asked.
	assert.equal(await documentKind(client({}, { kind: 'openapi' }), file), undefined);
	assert.equal(sent.length, 1);
	assert.equal(await documentKind(undefined, file), undefined);
	assert.equal(await documentKind(client({ [DOCUMENT_KIND_REQUEST]: 1 }, { kind: 'none' }), { scheme: 'untitled', toString: () => 'untitled:1' }), undefined);
	assert.equal(await documentKind(client({ [DOCUMENT_KIND_REQUEST]: 1 }, new Error('gone')), file), undefined);
});

test('the CLI binary honors the deprecated suspect.server.path only while suspect.basePath is default', () => {
	const settings = new Map();
	const stub = { workspace: { getConfiguration: () => ({ get: (key, fallback) => (settings.has(key) ? settings.get(key) : fallback) }) } };
	const { suspectBinary } = loadModule('runner', stub);
	assert.equal(suspectBinary(), 'suspect');
	settings.set('server.path', '/opt/legacy/suspect');
	assert.equal(suspectBinary(), '/opt/legacy/suspect');
	settings.set('basePath', 'suspect');
	assert.equal(suspectBinary(), '/opt/legacy/suspect');
	settings.set('basePath', '/opt/current/suspect');
	assert.equal(suspectBinary(), '/opt/current/suspect');
});

test('the CLI resolves from the setting, then the binary in this VSIX, then PATH', () => {
	const settings = new Map();
	const stub = { workspace: { getConfiguration: () => ({ get: (key, fallback) => (settings.has(key) ? settings.get(key) : fallback) }) } };
	const { binaryChoice, useExtensionRoot, EXECUTABLE } = loadModule('runner', stub);
	const root = fs.mkdtempSync(path.join(require('node:os').tmpdir(), 'suspect-ext-'));
	try {
		// A universal VSIX (no bin/): PATH.
		useExtensionRoot(root);
		assert.deepEqual(binaryChoice(), { binary: 'suspect', source: 'path' });
		// A platform VSIX: its own copy beats PATH.
		fs.mkdirSync(path.join(root, 'bin'));
		fs.writeFileSync(path.join(root, 'bin', EXECUTABLE), '');
		assert.deepEqual(binaryChoice(), { binary: path.join(root, 'bin', EXECUTABLE), source: 'bundled' });
		// An explicit setting beats both.
		settings.set('basePath', '/opt/pinned/suspect');
		assert.deepEqual(binaryChoice(), { binary: '/opt/pinned/suspect', source: 'setting' });
		settings.set('basePath', 'suspect');
		assert.equal(binaryChoice().source, 'bundled', 'the default value is not an explicit choice');
	} finally {
		useExtensionRoot(undefined);
		fs.rmSync(root, { recursive: true, force: true });
	}
});

test('the VSIX ships the bundle and leaves node_modules and sources out', () => {
	const ignore = fs.readFileSync(path.join(__dirname, '..', '.vscodeignore'), 'utf8').split('\n').map((l) => l.trim());
	for (const left of ['node_modules/**', 'src/**', 'test/**', 'scripts/**', 'dist/**']) assert.ok(ignore.includes(left), `${left} is shipped`);
	assert.ok(ignore.includes('!dist/extension.js'), 'the bundle must ship');
	assert.ok(!ignore.some((l) => l.startsWith('bin')), 'a platform VSIX ships bin/suspect');
	assert.match(packageManifest.scripts['vscode:prepublish'], /bundle/);
	assert.match(packageManifest.scripts.package, /--no-dependencies/);
	assert.ok(packageManifest.contributes.commands.some((c) => c.command === 'suspect.locateBinary'));
	const { TARGETS } = require('../scripts/package-target.cjs');
	assert.deepEqual(Object.keys(TARGETS).sort(), ['alpine-arm64', 'alpine-x64', 'darwin-arm64', 'darwin-x64', 'linux-arm64', 'linux-x64', 'win32-x64']);
	// The release workflows package and publish the same targets.
	const workflows = path.join(__dirname, '..', '..', '..', '.github', 'workflows');
	const packaging = fs.readFileSync(path.join(workflows, 'package-editors.yml'), 'utf8');
	const publishing = fs.readFileSync(path.join(workflows, 'publish-editors.yml'), 'utf8');
	const published = publishing.match(/^\s+TARGETS: (.+)$/m)[1].trim().split(/\s+/);
	assert.deepEqual(published.sort(), Object.keys(TARGETS).sort(), 'publish-editors.yml TARGETS');
	for (const [target, triple] of Object.entries(TARGETS)) {
		assert.ok(packaging.includes(`{ target: ${target}, triple: ${triple},`), `package-editors.yml packages ${target} from ${triple}`);
	}
});
