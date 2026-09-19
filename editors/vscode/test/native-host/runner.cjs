const assert = require('node:assert/strict');
const cp = require('node:child_process');
const crypto = require('node:crypto');
const fsSync = require('node:fs');
const fs = require('node:fs/promises');
const net = require('node:net');
const path = require('node:path');
const { finished } = require('node:stream/promises');
const { setTimeout: delay } = require('node:timers/promises');
const { promisify } = require('node:util');
const { FORMATS, CHECKS, InputError, parseInvocation, verifyInputs, verifyUnchanged, validateInventory,
	requiredChecks, requiredScreenshots, assertCheckSet, verifyInstalledSource, verifyScreenshots, fileIdentity } = require('./contract.cjs');

const execFile = promisify(cp.execFile);
const digest = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');

async function writeJson(filename, value) {
	await fs.writeFile(filename, `${JSON.stringify(value, null, 2)}\n`);
}

async function tree(directory) {
	const files = {};
	async function visit(current, prefix = '') {
		for (const entry of await fs.readdir(current, { withFileTypes: true })) {
			const relative = prefix + entry.name;
			const filename = path.join(current, entry.name);
			if (entry.isDirectory()) await visit(filename, `${relative}/`);
			else if (entry.isFile()) {
				const stat = await fs.stat(filename);
				files[relative] = { sha256: digest(await fs.readFile(filename)), size: stat.size, mtimeMs: stat.mtimeMs, ino: stat.ino };
			} else throw new Error(`Unexpected non-file package artifact: ${filename}`);
		}
	}
	await visit(directory);
	return files;
}

async function freePort() {
	const server = net.createServer();
	await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
	const { port } = server.address();
	await new Promise((resolve) => server.close(resolve));
	return port;
}

async function executablePath(command, environment) {
	if (path.isAbsolute(command)) return command;
	for (const directory of (environment.PATH ?? '').split(path.delimiter).filter(Boolean)) {
		const candidate = path.resolve(directory, command);
		try { await fs.access(candidate, fsSync.constants.X_OK); if ((await fs.stat(candidate)).isFile()) return candidate; } catch { /* Try the next PATH entry. */ }
	}
	throw new Error(`Executable is unavailable on PATH: ${command}`);
}

async function run(environment = process.env, argv = []) {
	let invocation;
	let inputs;
	try {
		invocation = parseInvocation(environment, argv);
		inputs = await verifyInputs(invocation);
	} catch (error) {
		if (error instanceof InputError) throw error;
		throw new InputError(`Native input verification failed: ${error.message}`, 'E_INPUT_IO');
	}
	const attempt = invocation.out;
	try { await fs.mkdir(attempt); }
	catch (error) { throw new InputError(`Cannot claim fresh native output: ${error.message}`, 'E_OUTPUT_EXISTS'); }
	const reportFile = path.join(attempt, 'report.json');
	const report = {
		format: FORMATS.report, runId: crypto.randomUUID(), mode: invocation.mode, scenario: invocation.scenario,
		status: 'running', phase: 'preparing', startedAt: new Date().toISOString(), inputs,
		requiredNativeChecks: requiredChecks(invocation.scenario, invocation.mode),
		requiredScreenshots: requiredScreenshots(invocation.scenario, invocation.mode),
		requiredClaims: ['inputPinsVerified', 'backendInventoryExact', 'vsixMatchesSource', 'nativeChecksComplete', 'screenshotsVerified', 'hostExitedZero', 'inputsUnchanged',
			...(invocation.mode === 'run' ? ['ownedOutputUnchanged'] : ['selectionProbeOnly'])],
		claims: { inputPinsVerified: true }, checks: [], commands: [],
	};
	const saveReport = () => writeJson(reportFile, report);
	await writeJson(path.join(attempt, 'inputs.json'), inputs);
	await saveReport();
	let failure;
	try {
	const extensionRoot = inputs.extension.directory;
	const tools = inputs.tools.directory;
	const binary = invocation.binary;
	const app = { executable: inputs.vscode.executable.path, executableSHA256: inputs.vscode.executable.sha256,
		productVersion: inputs.vscode.version, commit: inputs.vscode.commit, appRoot: inputs.vscode.appRoot };
	// The caller supplies the short external parent. No historical profile/cache
	// directory is consulted or selected as a fallback.
	const root = await fs.mkdtemp(path.join(invocation.scratch, 'n-'));
	const workspace = path.join(root, 'native workspace');
	const userData = path.join(root, 'u');
	const extensions = path.join(root, 'extensions');
	const home = path.join(root, 'home');
	const stage = path.join(root, 'package-source');
	const driver = path.join(root, 'test-driver');
	const config = path.join(workspace, 'config dir/sdk session.json');
	const source = path.join(workspace, 'selected source/api alias.json');
	const physicalSource = path.join(workspace, 'physical source/api.json');
	const model = path.join(path.dirname(source), 'model.json');
	const out = path.join(workspace, 'generated SDK');
	for (const directory of [workspace, userData, extensions, home, stage, driver, path.dirname(config), path.dirname(source), path.dirname(physicalSource), path.join(workspace, '.vscode'), path.join(root, 'tmp')]) {
		await fs.mkdir(directory, { recursive: true });
	}
	const env = {
		...environment,
		HOME: home, XDG_CONFIG_HOME: path.join(home, '.config'), XDG_CACHE_HOME: path.join(home, '.cache'), XDG_DATA_HOME: path.join(home, '.local/share'),
		TMPDIR: path.join(root, 'tmp'), npm_config_cache: path.join(root, 'npm-cache'), VSCE_STORE: 'file',
	};
	for (const key of ['VSCODE_IPC_HOOK_CLI', 'VSCODE_IPC_HOOK', 'VSCODE_PORTABLE', 'VSCODE_USER_DATA_DIR', 'VSCODE_EXTENSIONS', 'ELECTRON_RUN_AS_NODE']) delete env[key];
	report.isolation = { root, workspace, userData, extensions, home, temporaryDirectory: env.TMPDIR, npmCache: env.npm_config_cache };
	async function command(label, executable, args, cwd = root) {
		const selected = await executablePath(executable, env);
		const record = { label, executable: selected, identity: await fileIdentity(selected), args, cwd, shell: false };
		report.commands.push(record);
		try {
			const result = await execFile(selected, args, { cwd, env, shell: false, timeout: 120000, maxBuffer: 32 * 1024 * 1024 });
			record.exitCode = 0;
			await fs.writeFile(path.join(attempt, `${label}.log`), `${JSON.stringify(record)}\n${result.stdout}\n${result.stderr}`);
			return result.stdout;
		} catch (error) {
			record.exitCode = error.code;
			await fs.writeFile(path.join(attempt, `${label}.log`), `${JSON.stringify(record)}\n${error.stdout ?? ''}\n${error.stderr ?? ''}\n${error.stack}`);
			throw error;
		} finally { await writeJson(path.join(attempt, 'commands.json'), report.commands); await saveReport(); }
	}
	report.phase = 'inventory';
	const inventory = validateInventory(JSON.parse(await command('cli-inventory', binary, ['codegen-profiles', '--format', 'json'])), inputs.cli.expectedProfiles);
	report.inventory = inventory;
	report.claims.backendInventoryExact = true;
	await writeJson(path.join(attempt, 'cli-inventory.json'), inventory);
	report.phase = 'packaging';
	const settings = {
		'suspect.basePath': binary,
		'suspect.sdk.sessionConfig': 'config dir/sdk session.json',
		'suspect.sdk.sessionOutput': '../generated SDK',
		'suspect.sdk.packageName': 'native-host-python',
		'suspect.sdk.packageVersion': '1.2.3',
		'suspect.sdk.operationIds': ['getBalance'],
		'suspect.sdk.importNames': { 'python-http': 'native_host_sdk' },
		'suspect.sdk.compatibilityProfiles': [],
		'files.autoSave': 'off', 'files.hotExit': 'off', 'git.enabled': false,
		'update.mode': 'none', 'extensions.autoUpdate': false, 'extensions.autoCheckUpdates': false,
		'telemetry.telemetryLevel': 'off', 'chat.disableAIFeatures': true,
		'workbench.startupEditor': 'none', 'workbench.tips.enabled': false, 'workbench.editor.enablePreview': true,
		'editor.accessibilitySupport': 'on', 'editor.minimap.enabled': false, 'diffEditor.renderSideBySide': true,
		'security.workspace.trust.enabled': false,
	};
	await writeJson(path.join(workspace, '.vscode/settings.json'), Object.fromEntries(Object.entries(settings).filter(([key]) => key.startsWith('suspect.'))));
	await fs.mkdir(path.join(userData, 'User'), { recursive: true });
	await writeJson(path.join(userData, 'User/settings.json'), settings);
	const sourceText = JSON.stringify({
		openapi: '3.1.0', info: { title: 'Native VS Code SDK fixture', version: 'API-version' },
		servers: [{ url: 'https://example.com/v1' }], security: [{ Bearer: [] }],
		components: { securitySchemes: { Bearer: { type: 'http', scheme: 'bearer' } } },
		paths: { '/balance': { get: { operationId: 'getBalance', responses: {
			'200': { description: 'Balance', content: { 'application/json': { schema: { $ref: './model.json#/$defs/Balance' } } } },
		} } } },
	}, null, 2);
	const modelText = (origin) => JSON.stringify({ $defs: { Balance: {
		type: 'object', properties: { origin: { type: 'string', enum: [origin] } }, required: ['origin'], additionalProperties: false,
	} } }, null, 2);
	await fs.writeFile(physicalSource, sourceText);
	await fs.symlink(physicalSource, source);
	await fs.writeFile(model, modelText('native-A'));
	await fs.writeFile(path.join(path.dirname(physicalSource), 'model.json'), modelText('WRONG_PHYSICAL_BASE'));
	const configuration = {
		spec: '../selected source/api alias.json',
		targets: [
			{ backend: 'typescript-http', package_name: '@fixture/native-host-sdk', package_version: '1.2.3' },
			{ backend: 'rust-http', package_name: 'native-host-sdk', package_version: '1.2.3' },
		],
		operation_ids: ['getBalance'], compatibility_profiles: [], owner: 'native-host-fixture', cache_entries: 4,
	};
	await writeJson(config, configuration);
	const sourceHashes = inputs.extension.hashes;
	const vsix = path.join(attempt, 'extension.vsix');
	if (inputs.extension.vsix) {
		await fs.copyFile(inputs.extension.vsix.path, vsix, fsSync.constants.COPYFILE_EXCL);
		await fileIdentity(vsix, inputs.extension.vsix.sha256);
	} else {
		for (const filename of Object.keys(sourceHashes).filter((name) => name === 'package.json' || name === 'package-lock.json' || name.startsWith('dist/') || name.startsWith('media/'))) {
			const bytes = await fs.readFile(path.join(extensionRoot, filename));
			assert.equal(digest(bytes), sourceHashes[filename], `Editor source changed while staging: ${filename}`);
			await fs.mkdir(path.dirname(path.join(stage, filename)), { recursive: true });
			await fs.writeFile(path.join(stage, filename), bytes, { flag: 'wx' });
		}
		await fs.writeFile(path.join(stage, 'README.md'), '# Suspect\n\nCanonical SDK preview, watch, drift checks and generation.\n');
		await command('package-dependencies', 'npm', ['ci', '--omit=dev', '--ignore-scripts', '--no-audit', '--no-fund'], stage);
		await command('package-vsix', process.execPath, [path.join(tools, 'node_modules/@vscode/vsce/vsce'), 'package', '--out', vsix, '--allow-missing-repository', '--skip-license', '--no-rewrite-relative-links'], stage);
	}
	report.vsix = { mode: inputs.extension.vsix ? 'supplied' : 'packaged', ...await fileIdentity(vsix) };
	const { resolveCliArgsFromVSCodeExecutablePath } = require(path.join(tools, 'node_modules/@vscode/test-electron'));
	const [cli, ...cliArgs] = resolveCliArgsFromVSCodeExecutablePath(app.executable, { reuseMachineInstall: true });
	const version = (await command('vscode-version', cli, [...cliArgs, '--version', '--user-data-dir', userData, '--extensions-dir', extensions])).trim().split(/\r?\n/);
	assert.deepEqual(version.slice(0, 3), [inputs.vscode.version, inputs.vscode.commit, process.arch], 'Selected VS Code CLI differs from its pins');
	await command('install-vsix', cli, [...cliArgs, '--user-data-dir', userData, '--extensions-dir', extensions, '--do-not-include-pack-dependencies', '--install-extension', vsix]);
	await command('installed-extensions', cli, [...cliArgs, '--user-data-dir', userData, '--extensions-dir', extensions, '--list-extensions', '--show-versions']);
	const installed = [];
	for (const entry of await fs.readdir(extensions, { withFileTypes: true })) {
		if (!entry.isDirectory()) continue;
		const directory = path.join(extensions, entry.name);
		let metadata;
		try { metadata = JSON.parse(await fs.readFile(path.join(directory, 'package.json'), 'utf8')); } catch { continue; }
		if (`${metadata.publisher}.${metadata.name}` === inputs.extension.id) installed.push(directory);
	}
	assert.equal(installed.length, 1, 'Exactly one pinned-source extension must be installed');
	report.installedExtension = await verifyInstalledSource(installed[0], inputs.extension);
	report.claims.vsixMatchesSource = true;
	let baseline = {};
	if (invocation.mode === 'run') {
		report.phase = 'baseline';
		const written = JSON.parse(await command('owned-sdk-baseline', binary, ['codegen-session', '--config', config, '--out', out, '--format', 'json'], workspace));
		assert.equal(written.status, 'written');
		baseline = await tree(out);
		assert.ok(baseline['typescript/models.ts']);
		await writeJson(path.join(attempt, 'owned-output-before.json'), baseline);
		report.baseline = { owner: configuration.owner, files: Object.keys(baseline).length, before: path.join(attempt, 'owned-output-before.json') };
	} else report.baseline = null;
	await writeJson(path.join(driver, 'package.json'), {
		name: 'suspect-native-host-driver', publisher: 'suspect-tests', version: '0.0.0', engines: { vscode: '^1.85.0' },
		main: './index.cjs', activationEvents: ['*'],
	});
	await fs.writeFile(path.join(driver, 'index.cjs'), 'exports.activate = () => {};\nexports.deactivate = () => {};\n');
	for (const filename of ['suite.cjs', 'contract.cjs']) await fs.copyFile(path.join(__dirname, filename), path.join(driver, filename));
	const port = await freePort();
	const setup = {
		attempt, root, workspace, userData, extensions, tools, config, source, physicalSource, model, out, binary, inventory,
		runId: report.runId, mode: invocation.mode, scenario: invocation.scenario, pinsSha256: inputs.pinsFile.sha256,
		extension: inputs.extension,
		configuration, sourceText, modelA: modelText('native-A'), modelB: modelText('native-B'), baseline,
		app, cliSHA256: inputs.cli.sha256, vsix, vsixSHA256: report.vsix.sha256, sourceHashes, port,
		testHashes: Object.fromEntries(await Promise.all(['run.cjs', 'runner.cjs', 'suite.cjs', 'contract.cjs', 'pins.schema.json'].map(async (filename) => [filename, digest(await fs.readFile(path.join(__dirname, filename)))]))),
	};
	report.harnessHashes = setup.testHashes;
	const setupFile = path.join(attempt, 'setup.json');
	await writeJson(setupFile, setup);
	const args = [
		workspace, '--new-window', '--user-data-dir', userData, '--extensions-dir', extensions,
		`--extensionDevelopmentPath=${driver}`, `--extensionTestsPath=${path.join(driver, 'suite.cjs')}`,
		`--remote-debugging-port=${port}`, '--remote-debugging-address=127.0.0.1', '--skip-welcome', '--skip-release-notes',
		'--disable-updates', '--disable-workspace-trust', '--no-cached-data', '--use-inmemory-secretstorage',
	];
	await writeJson(path.join(attempt, 'launch.json'), { executable: app.executable, args, cwd: root, isolatedHome: home, setupFile });
	report.phase = 'native-host';
	report.launch = { executable: app.executable, args, cwd: root, isolatedHome: home, setupFile };
	await saveReport();
	console.log(`Native VS Code host evidence: ${attempt}`);
	const child = cp.spawn(app.executable, args, { cwd: root, env: { ...env, SUSPECT_NATIVE_SETUP: setupFile }, stdio: ['ignore', 'pipe', 'pipe'], shell: false });
	report.hostPid = child.pid;
	const stdout = fsSync.createWriteStream(path.join(attempt, 'host-stdout.log'), { flags: 'wx' });
	const stderr = fsSync.createWriteStream(path.join(attempt, 'host-stderr.log'), { flags: 'wx' });
	child.stdout.pipe(stdout);
	child.stderr.pipe(stderr);
	child.stdout.on('data', (data) => process.stdout.write(data));
	child.stderr.on('data', (data) => process.stderr.write(data));
	let timedOut = false;
	const timeout = setTimeout(() => { timedOut = true; child.kill('SIGTERM'); }, 180000);
	const force = setTimeout(() => child.kill('SIGKILL'), 185000);
	const exit = await new Promise((resolve, reject) => {
		child.once('error', reject);
		child.once('close', (code, signal) => resolve({ code, signal, timedOut }));
	}).finally(() => { clearTimeout(timeout); clearTimeout(force); });
	await Promise.all([finished(stdout), finished(stderr)]);
	report.hostExit = exit;
	await writeJson(path.join(attempt, 'host-exit.json'), exit);
	try { await fs.cp(path.join(userData, 'logs'), path.join(attempt, 'vscode-logs'), { recursive: true }); }
	catch (error) { if (error.code !== 'ENOENT') throw error; }
	const nativeFile = path.join(attempt, 'native-report.json');
	let observed;
	try { observed = JSON.parse(await fs.readFile(nativeFile, 'utf8')); }
	catch { throw new Error(`VS Code did not produce a native test report. Exit: ${JSON.stringify(exit)}. See ${attempt}`); }
	report.checks = observed.checks;
	if (invocation.mode === 'run' && invocation.scenario === 'lifecycle' && observed.status === 'passed') {
		const pid = observed.shutdownWatchPid;
		assert.ok(Number.isSafeInteger(pid) && pid > 0, 'A final observed watch PID is required');
		assert.ok(observed.processObservations?.some((entry) => entry.sessions?.some((session) => session.pid === pid && session.command.startsWith(`${binary} codegen-session `) && session.command.includes(`--config ${config}`))), 'Shutdown PID must identify the observed canonical watch');
		const alive = async () => {
			try {
				const { stdout } = await execFile('/bin/ps', ['-p', String(pid), '-o', 'command=']);
				return stdout.trim().startsWith(`${binary} codegen-session `) && stdout.includes(`--config ${config}`);
			} catch (error) { if (error.code === 1) return false; throw error; }
		};
		const deadline = Date.now() + 5000;
		while (await alive() && Date.now() < deadline) await delay(50);
		const stopped = !await alive();
		observed.checks.push({ name: CHECKS.shutdown, status: stopped ? 'passed' : 'failed', pid });
		if (!stopped) {
			observed.status = 'failed';
			process.kill(pid, 'SIGTERM');
		}
	}
	observed.hostExit = exit;
	await writeJson(nativeFile, observed);
	report.native = await fileIdentity(nativeFile);
	if (invocation.mode === 'run') {
		const after = await tree(out);
		await writeJson(path.join(attempt, 'owned-output-after.json'), after);
		assert.deepEqual(after, baseline, 'Owned output inventory/bytes/mtime/inode must remain unchanged');
		report.baseline.after = path.join(attempt, 'owned-output-after.json');
		report.claims.ownedOutputUnchanged = true;
	}
	assert.equal(exit.code, 0, `Native VS Code host failed; see ${attempt}`);
	assert.equal(exit.timedOut, false);
	report.claims.hostExitedZero = true;
	assertCheckSet(observed, setup);
	assert.equal(observed.vscodeVersion, inputs.vscode.version);
	assert.equal(observed.appRoot, inputs.vscode.appRoot);
	assert.equal(observed.cli.executable, binary);
	assert.equal(observed.cli.sha256, inputs.cli.sha256);
	assert.deepEqual([...observed.cli.advertisedProfiles].sort(), [...inputs.cli.expectedProfiles].sort());
	report.claims.nativeChecksComplete = true;
	await verifyScreenshots(observed, setup);
	report.screenshots = observed.screenshots;
	report.claims.screenshotsVerified = true;
	if (invocation.mode === 'selection-probe') report.claims.selectionProbeOnly = true;
	for (const [filename, expected] of Object.entries(setup.testHashes)) await fileIdentity(path.join(__dirname, filename), expected);
	} catch (error) {
		failure = error;
		report.error = { code: error.code ?? 'E_NATIVE_RUN', message: error.message, stack: error.stack };
	}
	try { await verifyUnchanged(inputs); report.claims.inputsUnchanged = true; }
	catch (error) {
		failure ??= error;
		report.integrityError = { code: error.code ?? 'E_INPUT_CHANGED', message: error.message };
	}
	if (!failure) {
		try { assert.ok(report.requiredClaims.every((claim) => report.claims[claim] === true), 'Every required claim must be verified'); }
		catch (error) { failure = error; report.error = { code: 'E_INCOMPLETE_REPORT', message: error.message }; }
	}
	report.status = failure ? 'failed' : 'passed';
	report.phase = failure ? report.phase : 'complete';
	report.completedAt = new Date().toISOString();
	report.exitCode = failure ? failure.exitCode ?? 1 : 0;
	await saveReport();
	return { report: reportFile, format: FORMATS.report, status: report.status, mode: invocation.mode, scenario: invocation.scenario, exitCode: report.exitCode };
}

module.exports = { run };
