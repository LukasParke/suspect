// Starts a real VS Code with this extension against docs/demo/petstore.yaml
// and checks the language client reaches running state, diagnostics appear
// once each, and a settings change re-filters them.
//
//   SUSPECT_TEST_BINARY=../../target/debug/suspect npm run test:host:smoke
//
// SUSPECT_VSCODE_BIN overrides the VS Code executable (default: the macOS
// application bundle). Needs a desktop session; CI runs the headless suites.
const assert = require('node:assert/strict');
const cp = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const extension = path.resolve(__dirname, '../..');
const repo = path.resolve(extension, '../..');
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
fs.mkdirSync(path.join(workspace, '.vscode'), { recursive: true });
const spec = path.join(workspace, 'petstore.yaml');
fs.copyFileSync(path.join(repo, 'docs/demo/petstore.yaml'), spec);
fs.writeFileSync(path.join(workspace, '.vscode/settings.json'), JSON.stringify({ 'suspect.basePath': binary }));
const report = path.join(root, 'report.json');

const env = { ...process.env, SUSPECT_SMOKE_SPEC: spec, SUSPECT_SMOKE_REPORT: report };
// Launched from another Electron app (or VS Code's own terminal), these make
// the binary behave as Node or attach to the running instance.
for (const key of ['ELECTRON_RUN_AS_NODE', 'VSCODE_IPC_HOOK_CLI', 'VSCODE_PID']) delete env[key];
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
const expected = cp.spawnSync(binary, ['validate', spec], { encoding: 'utf8' }).stdout.split('\n').filter((l) => /^\S+:\d+:\d+ /.test(l)).length
	+ cp.spawnSync(binary, ['lint', spec], { encoding: 'utf8' }).stdout.split('\n').filter((l) => /^\S+:\d+:\d+ /.test(l)).length;
assert.equal(observed.before.length, expected, `expected each of the CLI's ${expected} findings once`);
assert.ok(observed.after.length > 0 && observed.after.every((severity) => severity === 0),
	`suspect.lint.minSeverity=error left non-error findings: ${observed.after}`);
fs.rmSync(root, { recursive: true, force: true });
console.log('host smoke: ok');
