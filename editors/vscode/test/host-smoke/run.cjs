// Starts a real VS Code with this extension against docs/demo/petstore.yaml
// and checks the language client reaches running state, diagnostics appear
// once each, and a settings change re-filters them.
//
//   SUSPECT_TEST_BINARY=../../target/debug/suspect npm run test:host:smoke
//
// With SUSPECT_SMOKE_VSIX=<platform .vsix> (scripts/package-target.cjs) it
// runs the packaged extension instead of this folder, with no
// suspect.basePath and no `suspect` on PATH, so the language client can
// only start from the binary inside the VSIX. SUSPECT_TEST_BINARY is then
// only the CLI the expected finding count is taken from.
//
// SUSPECT_VSCODE_BIN overrides the VS Code executable (default: the macOS
// application bundle). Needs a desktop session; CI runs the headless suites.
const assert = require('node:assert/strict');
const cp = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const source = path.resolve(__dirname, '../..');
const repo = path.resolve(source, '../..');
const vsix = process.env.SUSPECT_SMOKE_VSIX && path.resolve(process.env.SUSPECT_SMOKE_VSIX);
const code = process.env.SUSPECT_VSCODE_BIN
	?? '/Applications/Visual Studio Code.app/Contents/MacOS/Code';
const binary = path.resolve(process.env.SUSPECT_TEST_BINARY ?? path.join(repo, 'target/debug/suspect'));
assert.ok(fs.existsSync(code), `VS Code not found at ${code}; set SUSPECT_VSCODE_BIN`);
assert.ok(fs.existsSync(binary), `suspect not found at ${binary}; build it or set SUSPECT_TEST_BINARY`);

// A short root: VS Code's IPC socket lives under --user-data-dir and must
// stay under the 103-character socket path limit.
const base = process.platform === 'win32' ? os.tmpdir() : '/tmp';
const root = fs.realpathSync(fs.mkdtempSync(path.join(base, 'sx-')));
const workspace = path.join(root, 'workspace');
// A VSIX is a zip with the extension under extension/.
let extension = source;
if (vsix) {
	assert.ok(fs.existsSync(vsix), `no VSIX at ${vsix}`);
	const unpacked = path.join(root, 'vsix');
	cp.execFileSync('unzip', ['-q', vsix, '-d', unpacked]);
	extension = path.join(unpacked, 'extension');
	const exe = process.platform === 'win32' ? 'suspect.exe' : 'suspect';
	assert.ok(fs.existsSync(path.join(extension, 'bin', exe)), `the VSIX has no bin/${exe}`);
}
fs.mkdirSync(path.join(workspace, '.vscode'), { recursive: true });
const spec = path.join(workspace, 'petstore.yaml');
fs.copyFileSync(path.join(repo, 'docs/demo/petstore.yaml'), spec);
fs.writeFileSync(path.join(workspace, '.vscode/settings.json'), JSON.stringify(vsix ? {} : { 'suspect.basePath': binary }));
// A floor the editor setting later overrides; `hint` filters nothing, so the
// CLI's finding count below is unchanged.
fs.writeFileSync(path.join(workspace, '.suspect.yaml'), 'lint:\n  min_severity: hint\n');
const report = path.join(root, 'report.json');

const env = { ...process.env, SUSPECT_SMOKE_SPEC: spec, SUSPECT_SMOKE_REPORT: report };
// Launched from another Electron app (or VS Code's own terminal), these make
// the binary behave as Node or attach to the running instance.
for (const key of ['ELECTRON_RUN_AS_NODE', 'VSCODE_IPC_HOOK_CLI', 'VSCODE_PID']) delete env[key];
if (vsix) {
	// Nothing on PATH may answer to `suspect`.
	env.PATH = (env.PATH ?? '').split(path.delimiter)
		.filter((dir) => !['suspect', 'suspect.exe'].some((name) => fs.existsSync(path.join(dir, name))))
		.join(path.delimiter);
}
const result = cp.spawnSync(code, [
	`--extensionDevelopmentPath=${extension}`,
	`--extensionTestsPath=${path.join(__dirname, 'suite.cjs')}`,
	`--user-data-dir=${path.join(root, 'user-data')}`,
	`--extensions-dir=${path.join(root, 'extensions')}`,
	'--disable-workspace-trust', '--skip-welcome', '--skip-release-notes',
	workspace,
], { env, stdio: 'inherit', timeout: 180000 });
assert.equal(result.status, 0, `VS Code exited with ${result.status ?? result.signal}`);

const observed = JSON.parse(fs.readFileSync(report, 'utf8'));
console.log(JSON.stringify(observed, null, 2));
assert.equal(observed.error, undefined, observed.error);
assert.ok(observed.activated, 'the extension did not activate');
assert.ok(observed.serverCommandRegistered, 'the language client never reached running state');
// `check` runs the editor's pass: one finding per problem, where
// `validate` plus `lint` would count a missing operationId twice.
const expected = cp.spawnSync(binary, ['check', spec], { encoding: 'utf8' }).stdout.split('\n').filter((l) => /^\S+:\d+:\d+ /.test(l)).length;
assert.equal(observed.before.length, expected, `expected each of the CLI's ${expected} findings once`);
assert.ok(observed.after.length > 0 && observed.after.every((severity) => severity === 0),
	`suspect.lint.minSeverity=error left non-error findings: ${observed.after}`);
assert.match(observed.settingsFile ?? '', /\.suspect\.yaml$/, 'the explain report names the settings file');
assert.match(observed.floorRow ?? '', /\| error \| editor setting \| .*\.suspect\.yaml:2 = hint; editor suspect\.lint\.minSeverity = error \|/,
	'the explain report shows the editor layer over .suspect.yaml');
// Every command path the extension spawns exists in this binary and
// prints no deprecation note.
for (const words of [['lsp'], ['gen', 'sdk'], ['gen', 'session'], ['gen', 'profiles'], ['gen', 'template'], ['traffic', 'gateway'], ['test', 'run'], ['gen', 'contract']]) {
	const help = cp.spawnSync(binary, [...words, '--help'], { encoding: 'utf8' });
	assert.equal(help.status, 0, `suspect ${words.join(' ')} --help exited ${help.status}: ${help.stderr}`);
	assert.equal(help.stderr, '', `suspect ${words.join(' ')} --help wrote to stderr`);
}
const profiles = cp.spawnSync(binary, ['gen', 'profiles', '--format', 'json'], { encoding: 'utf8' });
assert.equal(JSON.parse(profiles.stdout).format, 'suspect.sdk.profiles.v1');
assert.equal(profiles.stderr, '');
fs.rmSync(root, { recursive: true, force: true });
console.log('host smoke: ok');
