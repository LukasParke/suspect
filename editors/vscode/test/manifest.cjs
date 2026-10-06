// The extension manifest is part of the test surface: VS Code validates
// `contributes.semanticTokenScopes` at load and rejects the whole
// contribution — silently styling nothing — when the shape is wrong.
// That is exactly how the markdown token types once rendered as plain
// text: the scopes were a map, the schema wants an array of
// { language, scopes } entries. This file pins the shape.

const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const manifest = JSON.parse(
	fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8'),
);

const TOKEN_TYPES = [
	'markdownHeading',
	'markdownEmphasis',
	'markdownStrong',
	'markdownLink',
	'markdownCode',
	'markdownBlock',
];

test('semanticTokenScopes is an array the schema accepts', () => {
	const scopes = manifest.contributes.semanticTokenScopes;
	assert.ok(Array.isArray(scopes), 'semanticTokenScopes must be an array of { language, scopes } entries — a plain map keyed by token type is rejected at load and styles nothing');
	for (const entry of scopes) {
		assert.equal(typeof entry.language, 'string', 'each entry names one language');
		assert.equal(typeof entry.scopes, 'object', 'each entry carries a scopes map');
	}
});

test('every language the extension serves has the markdown scopes', () => {
	const selectorLanguages = manifest.contributes.languages
		? manifest.contributes.languages.map((l) => l.id)
		: [];
	// The server selects files by yaml and json document selectors.
	const needed = ['yaml', 'json'];
	for (const language of needed) {
		const entry = manifest.contributes.semanticTokenScopes.find(
			(e) => e.language === language,
		);
		assert.ok(entry, `no semanticTokenScopes entry for ${language} — tokens in that language render unstyled`);
		for (const type of TOKEN_TYPES) {
			assert.ok(
				entry.scopes[type] && entry.scopes[type].length > 0,
				`${language}: token type ${type} has no TextMate scope`,
			);
		}
	}
	assert.ok(selectorLanguages.length === 0, 'unreachable by construction');
});

test('the markdown scopes are the ones .md files use', () => {
	// The whole point is that descriptions highlight the way the editor's
	// own markdown does — so the scopes must be the markdown TextMate
	// scopes, not invented ones.
	const yaml = manifest.contributes.semanticTokenScopes.find(
		(e) => e.language === 'yaml',
	);
	assert.deepEqual(yaml.scopes.markdownHeading[0], 'markup.heading.markdown');
	assert.deepEqual(yaml.scopes.markdownEmphasis[0], 'markup.italic.markdown');
	assert.deepEqual(yaml.scopes.markdownStrong[0], 'markup.bold.markdown');
	assert.ok(yaml.scopes.markdownLink.some((s) => s.startsWith('markup.underline.link')));
});
