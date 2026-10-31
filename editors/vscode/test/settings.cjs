// The `suspect.*` settings are generated from the configuration schema
// (scripts/settings.cjs). These tests fail when package.json or
// src/settings.generated.ts drift from it, and pin the mapping rules.

const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const { generate, manifestWith, MANIFEST, GETTERS, WRITE_COMMAND } = require('../scripts/settings.cjs');

const generated = generate();
const props = generated.configuration.properties;
const schema = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'schemas', 'suspect.schema.json'), 'utf8'));

test('package.json contributes exactly the generated settings', () => {
	assert.equal(
		fs.readFileSync(MANIFEST, 'utf8'),
		manifestWith(generated.configuration),
		`package.json contributes.configuration is stale; run \`${WRITE_COMMAND}\``,
	);
});

test('src/settings.generated.ts is current', () => {
	assert.equal(
		fs.readFileSync(GETTERS, 'utf8'),
		generated.typescript,
		`src/settings.generated.ts is stale; run \`${WRITE_COMMAND}\``,
	);
});

test('each exposed config key is one setting: suspect. plus the camelCase path', () => {
	const exposed = [];
	const walk = (node, at) => {
		for (const [key, prop] of Object.entries(node.properties ?? {})) {
			const next = [...at, key];
			if (prop['x-suspect-editor'] && !prop.deprecated) exposed.push([next.join('.'), prop]);
			if (prop.properties) walk(prop, next);
		}
	};
	walk(schema, []);
	assert.ok(exposed.length >= 13, `only ${exposed.length} exposed keys`);
	const camel = (s) => s.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
	for (const [configKey, prop] of exposed) {
		const key = `suspect.${configKey.split('.').map(camel).join('.')}`;
		assert.equal(prop['x-suspect-editor'].key, key);
		const setting = props[key];
		assert.ok(setting, `${key} is not contributed`);
		assert.equal(setting.default, null, `${key}: unset defers to .suspect.yaml`);
		assert.ok(setting.type.includes('null'), `${key}: null must be a valid value`);
		assert.ok(setting.markdownDescription.includes(`\`${configKey}\``), `${key} names its config key`);
		if (prop.enum) assert.deepEqual(setting.enum, [...prop.enum, null]);
	}
});

test('renamed settings stay as deprecated aliases with a null default', () => {
	for (const [from, to] of [
		['suspect.testBaseUrl', 'suspect.test.baseUrl'],
		['suspect.gatewayPort', 'suspect.gateway.port'],
		['suspect.ci.baseline', 'suspect.diff.baseline'],
		['suspect.formatting.sortKeys', 'suspect.format.sortKeys'],
		['suspect.inlayHints.refs', 'suspect.editor.inlayHints.refs'],
		['suspect.inlayHints.properties', 'suspect.editor.inlayHints.properties'],
		['suspect.sdk.packageName', 'suspect.codegen.packageName'],
		['suspect.sdk.packageVersion', 'suspect.codegen.packageVersion'],
		['suspect.sdk.importNames', 'suspect.codegen.importNames'],
		['suspect.sdk.operationIds', 'suspect.codegen.operationIds'],
		['suspect.sdk.compatibilityProfiles', 'suspect.codegen.compatibilityProfiles'],
		['suspect.sdk.sessionConfig', 'suspect.codegen.session.config'],
		['suspect.sdk.sessionOutput', 'suspect.codegen.session.out'],
	]) {
		assert.ok(props[to], `${to} is contributed`);
		assert.ok(props[from], `${from} is still contributed`);
		assert.equal(props[from].default, null, `${from}: an untouched old key must not override the files`);
		assert.ok(props[from].deprecationMessage.includes(to), `${from} names ${to}`);
		assert.match(generated.typescript, new RegExp(`\\{ from: '${from}', to: '${to}' \\}`));
	}
});

test('editor-only settings come from settings.editor.json unchanged', () => {
	const editorOnly = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'settings.editor.json'), 'utf8'));
	for (const key of ['suspect.basePath', 'suspect.server.path']) {
		assert.deepEqual(props[key], editorOnly[key]);
	}
	assert.ok(props['suspect.server.path'].deprecationMessage.includes('suspect.basePath'));
});

test('every getter has a type: objects and lists are not read as strings', () => {
	const ts = generated.typescript;
	assert.match(ts, /export function lintMinSeverity\(scope\?: SettingScope\): "error" \| "warning" \| "info" \| "hint" \| null/);
	assert.match(ts, /export function lintRules\(scope\?: SettingScope\): Readonly<Record<string, "error" \| "warning" \| "info" \| "hint" \| "off">> \| null/);
	assert.match(ts, /export function testBaseUrl\(scope\?: SettingScope\): string \| null \{\n\treturn first<string>\(scope, \['test.baseUrl', 'testBaseUrl'\]\);/);
	assert.match(ts, /export function codegenImportNames\(scope\?: SettingScope\): Readonly<Record<string, string>> \| null \{/);
	assert.match(ts, /export function codegenOperationIds\(scope\?: SettingScope\): readonly string\[\] \| null \{/);
});

test('compatibility profiles offer every name suspect gen sdk accepts', () => {
	const { SDK_COMPATIBILITY_PROFILES } = require('../dist/generation.js');
	const row = schema.properties.codegen.properties.compatibility_profiles;
	assert.deepEqual(row.items.enum, [...SDK_COMPATIBILITY_PROFILES]);
	assert.deepEqual(props['suspect.codegen.compatibilityProfiles'].items.enum, row.items.enum);
	assert.equal(props['suspect.codegen.compatibilityProfiles'].uniqueItems, true);
});

test('no source file reads a suspect setting by string', () => {
	const src = path.join(__dirname, '..', 'src');
	for (const file of fs.readdirSync(src)) {
		if (file === 'settings.generated.ts') continue;
		const text = fs.readFileSync(path.join(src, file), 'utf8');
		assert.doesNotMatch(text, /getConfiguration\('suspect'[^)]*\)\s*\.get\b/, `${file} reads a setting by string; use the generated getter`);
		assert.doesNotMatch(text, /config(?:\(\))?\.get<[^>]*>\('(?:sdk|lint|test|gateway|ci|basePath|server)/, `${file} reads a setting by string`);
	}
});
