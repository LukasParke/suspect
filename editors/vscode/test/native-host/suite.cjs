const assert = require('node:assert/strict');
const cp = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs/promises');
const path = require('node:path');
const { setTimeout: delay } = require('node:timers/promises');
const { promisify } = require('node:util');
// This module is supplied by the official, running VS Code extension host.
const vscode = require('vscode');
const { FORMATS, CHECKS, PROFILE_INFO, requiredChecks, verifyInstalledSource } = require('./contract.cjs');

const execFile = promisify(cp.execFile);
const digest = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');

async function waitFor(predicate, message, timeout = 15000) {
	const deadline = Date.now() + timeout;
	while (!await predicate()) {
		if (Date.now() >= deadline) throw new Error(message);
		await delay(25);
	}
}

async function children() {
	let ids;
	try { ids = (await execFile('/usr/bin/pgrep', ['-P', String(process.pid)])).stdout.trim().split(/\s+/).filter(Boolean); }
	catch (error) { if (error.code === 1) return []; throw error; }
	if (!ids.length) return [];
	const { stdout } = await execFile('/bin/ps', ['-p', ids.join(','), '-o', 'pid=,ppid=,command=']);
	return stdout.trim().split('\n').filter(Boolean).map((line) => {
		const [, pid, parent, command] = /^\s*(\d+)\s+(\d+)\s+(.+)$/.exec(line);
		return { pid: Number(pid), parent: Number(parent), command };
	});
}

exports.run = async function run() {
	const setup = JSON.parse(await fs.readFile(process.env.SUSPECT_NATIVE_SETUP, 'utf8'));
	const reportPath = path.join(setup.attempt, 'native-report.json');
	const report = {
		format: FORMATS.native, runId: setup.runId, pinsSha256: setup.pinsSha256, status: 'running', startedAt: new Date().toISOString(),
		mode: setup.mode, scenario: setup.scenario, requiredNativeChecks: requiredChecks(setup.scenario, setup.mode),
		vscodeVersion: vscode.version, appHost: vscode.env.appHost, uiKind: vscode.env.uiKind, appRoot: vscode.env.appRoot,
		extensionHostPid: process.pid, runtime: process.versions, sessionId: vscode.env.sessionId,
		workspace: setup.workspace, config: setup.config, lexicalSource: setup.source, output: setup.out,
		cli: { executable: setup.binary, sha256: setup.cliSHA256, advertisedProfiles: setup.inventory.profiles.map((entry) => entry.profile) },
		checks: [], screenshots: [], snapshots: [], processObservations: [], documentChanges: [],
	};
	const save = () => fs.writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
	let page;
	let currentCheck;
	let watchPid;
	let pair;
	const changes = vscode.workspace.onDidChangeTextDocument((event) => {
		if (event.document.uri.scheme === 'suspect-sdk-preview') report.documentChanges.push({ uri: event.document.uri.toString(), version: event.document.version, sha256: digest(event.document.getText()) });
	});
	async function check(name, action) {
		currentCheck = { name, status: 'running', startedAt: new Date().toISOString() };
		report.checks.push(currentCheck);
		await save();
		try {
			const details = await action();
			Object.assign(currentCheck, { status: 'passed', completedAt: new Date().toISOString(), ...(details ?? {}) });
		} catch (error) {
			Object.assign(currentCheck, { status: 'failed', error: error.stack ?? String(error) });
			throw error;
		} finally { await save(); }
	}
	async function cliProcesses(label) {
		const all = await children();
		const sessions = all.filter((child) => child.command.startsWith(`${setup.binary} codegen-session `));
		report.processObservations.push({ label, at: new Date().toISOString(), sessions, cliChildren: all.filter((child) => child.command.startsWith(setup.binary)) });
		return sessions;
	}
	async function oneWatch(label, expectedPid = watchPid) {
		const sessions = await cliProcesses(label);
		assert.equal(sessions.length, 1, `Expected one canonical session during ${label}: ${JSON.stringify(sessions)}`);
		const child = sessions[0];
		assert.ok(child.command.includes(`--config ${setup.config}`));
		assert.ok(child.command.includes(`--out ${setup.out}`));
		assert.ok(child.command.includes('--watch') && child.command.includes('--preview') && child.command.includes('--format json'));
		if (expectedPid !== undefined) assert.equal(child.pid, expectedPid, 'saved edits must reuse the same real CLI watch process');
		const lsp = (await children()).filter((process) => process.command === `${setup.binary} lsp`);
		assert.deepEqual(lsp.map((process) => process.pid), [report.lspPid], 'the packaged language client must remain stable during SDK actions');
		return child.pid;
	}
	async function visibleInput(text) {
		await page.waitForFunction((expected) => {
			const widget = document.querySelector('.quick-input-widget');
			const input = widget?.querySelector('.quick-input-box input');
			return widget && input && getComputedStyle(widget).display !== 'none' && widget.getBoundingClientRect().height > 0 &&
				[widget.textContent, input.placeholder, input.getAttribute('aria-label')].some((value) => value?.includes(expected));
		}, text, { timeout: 15000 });
		return page.locator('.quick-input-widget .quick-input-box input').first();
	}
	async function input(text, value) {
		const box = await visibleInput(text);
		if (value !== undefined) await box.fill(value);
		await box.press('Enter');
	}
	async function choose(text, label) {
		const box = await visibleInput(text);
		await box.fill(label);
		await page.locator('.quick-input-widget .monaco-list-row').filter({ hasText: label }).first().waitFor({ state: 'visible' });
		await box.press('Enter');
	}
	async function capture(name) {
		await delay(300); // Let native diff/layout workers finish painting the accepted document update.
		const filename = path.join(setup.attempt, `${name}.png`);
		const dom = await page.evaluate(() => ({
			title: document.title, url: location.href, visibilityState: document.visibilityState,
			width: innerWidth, height: innerHeight,
			status: document.querySelector('.statusbar')?.innerText,
			diffs: document.querySelectorAll('.monaco-diff-editor').length,
			readOnlyInputs: [...document.querySelectorAll('.monaco-diff-editor [aria-readonly], .monaco-diff-editor textarea')].map((node) => ({ role: node.getAttribute('role'), ariaReadOnly: node.getAttribute('aria-readonly'), readOnly: node.readOnly, label: node.getAttribute('aria-label') })),
			quickInput: document.querySelector('.quick-input-widget')?.textContent?.slice(0, 12000),
		}));
		await page.screenshot({ path: filename, animations: 'disabled' });
		report.screenshots.push({ file: filename, sha256: digest(await fs.readFile(filename)), source: 'official VS Code Electron renderer via CDP', dom });
		await save();
	}
	function sdkTabs() {
		return vscode.window.tabGroups.all.flatMap((group) => group.tabs).filter((tab) =>
			tab.input instanceof vscode.TabInputTextDiff && tab.input.modified.scheme === 'suspect-sdk-preview');
	}
	async function selectDiff(artifact = 'typescript/models.ts') {
		await choose('SDK generation', artifact);
		await waitFor(() => sdkTabs().some((tab) => tab.input.modified.path === `/${artifact}`), 'The real SDK diff tab did not open');
		const tab = sdkTabs().find((tab) => tab.input.modified.path === `/${artifact}`);
		const original = await vscode.workspace.openTextDocument(tab.input.original);
		const modified = await vscode.workspace.openTextDocument(tab.input.modified);
		return { original, modified, originalUri: tab.input.original, modifiedUri: tab.input.modified, tab };
	}
	async function snapshot(label) {
		const value = {
			label, originalUri: pair.originalUri.toString(), modifiedUri: pair.modifiedUri.toString(),
			originalVersion: pair.original.version, modifiedVersion: pair.modified.version,
			originalDirty: pair.original.isDirty, modifiedDirty: pair.modified.isDirty,
			original: pair.original.getText(), modified: pair.modified.getText(),
		};
		const filename = path.join(setup.attempt, `snapshot-${label}.json`);
		await fs.writeFile(filename, `${JSON.stringify(value, null, 2)}\n`);
		report.snapshots.push({ label, file: filename, originalSHA256: digest(value.original), modifiedSHA256: digest(value.modified), modifiedVersion: value.modifiedVersion });
		await save();
		return value;
	}
	async function diskUnchanged() {
		for (const [relative, expected] of Object.entries(setup.baseline)) {
			const filename = path.join(setup.out, relative);
			const stat = await fs.stat(filename);
			assert.deepEqual({ sha256: digest(await fs.readFile(filename)), size: stat.size, mtimeMs: stat.mtimeMs, ino: stat.ino }, expected, `Owned SDK artifact changed: ${relative}`);
		}
	}
	async function startWatch() {
		const pending = vscode.commands.executeCommand('suspect.watchSdk', vscode.Uri.file(setup.config));
		await input(`relative paths resolve from ${path.dirname(setup.config)}`, '../generated SDK');
		await pending;
		pair = await selectDiff();
	}
	async function installedExtension() {
		assert.equal(vscode.version, setup.app.productVersion);
		assert.equal(vscode.env.uiKind, vscode.UIKind.Desktop);
		assert.equal(vscode.env.appRoot, setup.app.appRoot);
		assert.deepEqual(vscode.workspace.workspaceFolders.map((folder) => folder.uri.fsPath), [setup.workspace]);
		const extension = vscode.extensions.getExtension(setup.extension.id);
		assert.ok(extension, 'The packaged extension was not installed');
		assert.ok(extension.extensionPath.startsWith(`${setup.extensions}${path.sep}`), 'The tested extension must come from the isolated VSIX installation');
		await verifyInstalledSource(extension.extensionPath, setup.extension);
		await extension.activate();
		assert.equal(extension.isActive, true);
		const registered = await vscode.commands.getCommands(true);
		for (const name of ['suspect.genPreset', 'suspect.previewSdk', 'suspect.checkSdk', 'suspect.watchSdk', 'suspect.stopSdkWatch', 'suspect.showSdkPreview']) assert.ok(registered.includes(name), `Missing production command: ${name}`);
		return { extensionId: extension.id, extensionPath: extension.extensionPath, version: extension.packageJSON.version, codeHashesMatch: true };
	}
	async function attachRenderer() {
		const { chromium } = require(path.join(setup.tools, 'node_modules/playwright-core'));
		const browser = await chromium.connectOverCDP(`http://127.0.0.1:${setup.port}`);
		await waitFor(() => browser.contexts().flatMap((context) => context.pages()).some((candidate) => candidate.url().includes('workbench')), 'No native VS Code workbench CDP target');
		page = browser.contexts().flatMap((context) => context.pages()).find((candidate) => candidate.url().includes('workbench'));
		await page.locator('.monaco-workbench').waitFor({ state: 'visible' });
		await page.bringToFront();
		return { rendererURL: page.url(), title: await page.title() };
	}
	async function profilePicker() {
		const box = await visibleInput('Suspect: choose generation output');
		const expected = [...setup.inventory.profiles.map((entry) => `${PROFILE_INFO[entry.profile][1]} HTTP SDK`), 'Markdown documentation', 'Custom template manifest'];
		// Native quick-pick rows are virtualized. Visit every selection so the
		// complete advertised menu is checked even when twelve backends exceed the viewport.
		const seen = new Map();
		for (let step = 0; step < expected.length; step++) {
			const rows = await page.locator('.quick-input-widget .monaco-list-row').evaluateAll((nodes) => nodes.map((node) => ({
				index: Number(node.getAttribute('data-index')), label: node.querySelector('.label-name')?.textContent,
			})));
			for (const row of rows) { assert.ok(Number.isSafeInteger(row.index) && typeof row.label === 'string'); seen.set(row.index, row.label); }
			await box.press('ArrowDown');
			await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(resolve)));
		}
		const actual = [...seen].sort(([a], [b]) => a - b).map(([, label]) => label);
		assert.deepEqual(actual, expected);
		return { box, actual };
	}
	await save();
	try {
		if (setup.mode === 'selection-probe') {
			await check(CHECKS.selectionProbe, async () => {
				const extension = await installedExtension();
				const renderer = await attachRenderer();
				const pending = vscode.commands.executeCommand('suspect.genPreset');
				const { box, actual } = await profilePicker();
				await capture('probe-01-profile-picker');
				await box.press('Escape');
				await pending;
				assert.deepEqual(await cliProcesses('selection probe cancelled'), []);
				await assert.rejects(fs.stat(path.join(setup.workspace, 'gen-out')), { code: 'ENOENT' });
				await assert.rejects(fs.stat(setup.out), { code: 'ENOENT' });
				return { ...extension, ...renderer, actualLabels: actual, noGeneration: true };
			});
			report.status = 'passed';
			return;
		}
		await check(CHECKS.activation, installedExtension);
		await check(CHECKS.renderer, async () => {
			const renderer = await attachRenderer();
			await capture('01-native-activation');
			return renderer;
		});
		await check(CHECKS.lsp, async () => {
			await waitFor(async () => (await children()).some((child) => child.command === `${setup.binary} lsp`), 'The real language client did not launch canonical suspect lsp');
			await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(vscode.Uri.file(setup.source)));
			await waitFor(async () => (await vscode.commands.getCommands(true)).includes('suspect.generateExample'), 'LSP initialization did not register its server commands');
			const registered = await vscode.commands.getCommands(true);
			for (const command of ['suspect.runWorkflow', 'suspect.generateExample', 'suspect.showRefGraph', 'suspect.breakingChanges', 'suspect.contractCoverage', 'suspect.renderPreview']) assert.ok(registered.includes(command), `Missing local/server command: ${command}`);
			const servers = (await children()).filter((child) => child.command.startsWith(`${setup.binary} lsp`));
			assert.equal(servers.length, 1);
			assert.equal(servers[0].command, `${setup.binary} lsp`);
			report.lspPid = servers[0].pid;
			return { lspPid: report.lspPid, argv: [setup.binary, 'lsp'] };
		});
		if (setup.scenario === 'commands') {
			await check(CHECKS.preview, async () => {
				const pending = vscode.commands.executeCommand('suspect.previewSdk');
				await input(`relative paths resolve from ${path.dirname(setup.config)}`, '../generated SDK');
				pair = await selectDiff();
				await pending;
				const query = new URLSearchParams(pair.modifiedUri.query);
				assert.equal(query.get('config'), setup.config);
				assert.equal(query.get('source'), setup.source);
				assert.equal(query.get('output'), setup.out);
				assert.match(pair.modified.getText(), /native-A/);
				assert.equal(pair.modified.getText(), pair.original.getText());
				assert.equal(pair.modified.isDirty, false);
				assert.deepEqual(await cliProcesses('completed one-shot preview'), []);
				await snapshot('one-shot-preview');
				await capture('commands-01-one-shot-preview');
				await diskUnchanged();
			});
			await check(CHECKS.showLatest, async () => {
				const pending = vscode.commands.executeCommand('suspect.showSdkPreview');
				pair = await selectDiff('typescript/operations.ts');
				await pending;
				assert.equal(pair.modified.getText(), await fs.readFile(path.join(setup.out, 'typescript/operations.ts'), 'utf8'));
				assert.match(pair.modified.getText(), /getBalance/);
				assert.deepEqual(await cliProcesses('show latest completed preview'), []);
				await snapshot('show-latest-operations');
				await diskUnchanged();
			});
			await check(CHECKS.checkCurrent, async () => {
				await vscode.commands.executeCommand('workbench.action.closeAllEditors');
				const pending = vscode.commands.executeCommand('suspect.checkSdk');
				await input(`relative paths resolve from ${path.dirname(setup.config)}`, '../generated SDK');
				await pending;
				assert.equal(sdkTabs().length, 0);
				await waitFor(async () => /SDK #1.*current/.test(await page.locator('.statusbar').innerText()), 'The native current-check status did not finish rendering');
				assert.match(await page.locator('.statusbar').innerText(), /SDK #1.*current/);
				assert.deepEqual(await cliProcesses('completed current check'), []);
				await capture('commands-02-current-check');
				await diskUnchanged();
			});
			await check(CHECKS.checkDrift, async () => {
				await fs.writeFile(setup.model, setup.modelB);
				const pending = vscode.commands.executeCommand('suspect.checkSdk');
				await input(`relative paths resolve from ${path.dirname(setup.config)}`, '../generated SDK');
				await pending;
				assert.equal(sdkTabs().length, 0);
				await waitFor(async () => /SDK #1.*drift/.test(await page.locator('.statusbar').innerText()), 'The native drift-check status did not finish rendering');
				assert.match(await page.locator('.statusbar').innerText(), /SDK #1.*drift/);
				assert.deepEqual(await cliProcesses('completed drift check'), []);
				await capture('commands-03-drift-check');
				await diskUnchanged();
				await fs.writeFile(setup.model, setup.modelA);
			});
			report.status = 'passed';
			return;
		}
		await check(CHECKS.discovery, async () => {
			await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(vscode.Uri.file(setup.source)));
			const pending = vscode.commands.executeCommand('suspect.genPreset');
			const { box, actual } = await profilePicker();
			await capture('02-native-profile-picker');
			await box.press('Escape');
			await pending;
			assert.deepEqual(await cliProcesses('cancelled profile picker'), []);
			await assert.rejects(fs.stat(path.join(setup.workspace, 'gen-out')), { code: 'ENOENT' });
			return { actualLabels: actual, advertisedProfiles: setup.inventory.profiles.map((entry) => entry.profile) };
		});
		let a;
		await check(CHECKS.diff, async () => {
			await startWatch();
			watchPid = await oneWatch('initial native diff', undefined);
			for (const uri of [pair.originalUri, pair.modifiedUri]) {
				assert.equal(uri.scheme, 'suspect-sdk-preview');
				const query = new URLSearchParams(uri.query);
				assert.equal(query.get('config'), setup.config);
				assert.equal(query.get('source'), setup.source);
				assert.equal(query.get('output'), setup.out);
			}
			assert.equal((await fs.lstat(setup.source)).isSymbolicLink(), true);
			assert.equal(pair.original.getText(), await fs.readFile(path.join(setup.out, 'typescript/models.ts'), 'utf8'));
			assert.match(pair.modified.getText(), /native-A/);
			assert.doesNotMatch(pair.modified.getText(), /WRONG_PHYSICAL_BASE/);
			await vscode.commands.executeCommand('workbench.action.focusActiveEditorGroup');
			assert.equal(vscode.window.activeTextEditor.document.uri.toString(), pair.modifiedUri.toString());
			const before = pair.modified.getText();
			await vscode.commands.executeCommand('type', { text: 'NATIVE_READONLY_MUST_REFUSE_EDIT' });
			await delay(100);
			assert.equal(pair.modified.getText(), before, 'Native editor typing must not modify provider-backed generated text');
			assert.equal(pair.original.isDirty, false);
			assert.equal(pair.modified.isDirty, false);
			a = await snapshot('A');
			await capture('03-native-readonly-diff-A');
			return { watchPid, originalUri: pair.originalUri.toString(), modifiedUri: pair.modifiedUri.toString(), nativeTypingRefused: true };
		});
		await check(CHECKS.changed, async () => {
			await fs.writeFile(setup.model, setup.modelB);
			await waitFor(() => pair.modified.getText().includes('native-B'), 'Native diff did not refresh to source revision B');
			assert.equal(pair.original.getText(), a.original);
			assert.ok(pair.modified.version > a.modifiedVersion);
			assert.equal(sdkTabs().length, 1);
			assert.equal(sdkTabs()[0].input.modified.toString(), a.modifiedUri);
			await oneWatch('changed source B');
			await snapshot('B');
			await capture('04-native-live-diff-B');
			await diskUnchanged();
		});
		await check(CHECKS.reverted, async () => {
			await fs.writeFile(setup.model, setup.modelA);
			await waitFor(() => pair.modified.getText() === a.modified, 'Native diff did not restore the A revision');
			await oneWatch('reverted source A');
			assert.equal(pair.original.getText(), a.original);
			await snapshot('reverted-A');
			await diskUnchanged();
		});
		await check(CHECKS.removed, async () => {
			await fs.writeFile(setup.config, `${JSON.stringify({ ...setup.configuration, targets: setup.configuration.targets.filter((target) => target.backend !== 'typescript-http') }, null, 2)}\n`);
			await waitFor(() => pair.modified.getText() === '', 'Removed target did not empty the native generated side');
			assert.equal(pair.original.getText(), a.original);
			await oneWatch('removed TypeScript target');
			await snapshot('removed');
			await capture('05-native-removed-diff');
			await diskUnchanged();
			await fs.writeFile(setup.config, `${JSON.stringify(setup.configuration, null, 2)}\n`);
			await waitFor(() => pair.modified.getText() === a.modified, 'Native diff did not restore the removed target');
			await oneWatch('restored target');
		});
		await check(CHECKS.recovered, async () => {
			await fs.writeFile(setup.model, '{unfinished');
			await waitFor(() => /planning failed/.test(pair.modified.getText()), 'Planning failure left a successful native preview visible');
			assert.match(pair.original.getText(), /planning failed/);
			await oneWatch('source error');
			await snapshot('error');
			await capture('06-native-planning-error');
			await fs.writeFile(setup.model, setup.modelA);
			await waitFor(() => pair.modified.getText() === a.modified, 'Native diff did not recover after fixing the source');
			assert.equal(pair.original.getText(), a.original);
			await oneWatch('source recovery');
			await snapshot('recovered');
			await capture('07-native-recovered-diff');
			await diskUnchanged();
		});
		await check(CHECKS.stop, async () => {
			await vscode.commands.executeCommand('suspect.stopSdkWatch');
			await waitFor(() => /stopped/.test(pair.modified.getText()) && /stopped/.test(pair.original.getText()), 'Stopped native documents retained successful contents');
			assert.deepEqual(await cliProcesses('stopped watch'), []);
			await snapshot('stopped');
			await diskUnchanged();
		});
		await check(CHECKS.dialogCancel, async () => {
			const pending = vscode.commands.executeCommand('suspect.watchSdk', vscode.Uri.file(setup.config));
			const box = await visibleInput(`relative paths resolve from ${path.dirname(setup.config)}`);
			await box.press('Escape');
			await pending;
			assert.deepEqual(await cliProcesses('cancelled watch dialog'), []);
			await diskUnchanged();
		});
		await check(CHECKS.progressCancel, async () => {
			const root = path.join(setup.root, 'cancellation');
			await fs.mkdir(root);
			const fifo = path.join(root, 'blocked-schema.json');
			await execFile('/usr/bin/mkfifo', [fifo]);
			const source = JSON.parse(setup.sourceText);
			source.paths['/balance'].get.responses['200'].content['application/json'].schema.$ref = './blocked-schema.json#/$defs/Balance';
			await fs.writeFile(path.join(root, 'entry.json'), JSON.stringify(source));
			const config = path.join(root, 'blocked-preview.json');
			await fs.writeFile(config, JSON.stringify({ ...setup.configuration, spec: 'entry.json' }));
			const pending = vscode.commands.executeCommand('suspect.previewSdk', vscode.Uri.file(config));
			await input(`relative paths resolve from ${root}`, setup.out);
			await waitFor(async () => (await cliProcesses('waiting for blocked preview')).length === 1, 'Canonical preview did not stay in flight at its FIFO source');
			const cancel = page.getByRole('button', { name: 'Cancel', exact: true });
			await cancel.waitFor({ state: 'visible', timeout: 15000 });
			await capture('08-native-progress-cancellation');
			await cancel.click();
			await pending;
			assert.deepEqual(await cliProcesses('cancelled native progress'), []);
			await diskUnchanged();
			return { blockingFixture: fifo, fixtureKind: 'Regular OpenAPI entry with a FIFO schema reference and no writer; unmodified canonical CLI blocked in reference IO' };
		});
		await check(CHECKS.generate, async () => {
			await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(vscode.Uri.file(setup.source)));
			const pending = vscode.commands.executeCommand('suspect.genPreset');
			await choose('Suspect: choose generation output', 'Python HTTP SDK');
			await input('Explicit Python distribution name', 'native-host-python');
			await input('Explicit package SemVer', '1.2.3');
			await input('Exact operation IDs as a JSON string array', '["getBalance"]');
			await pending;
			const readme = path.join(setup.workspace, 'gen-out/python-http/python/README.md');
			await waitFor(() => vscode.window.activeTextEditor?.document.uri.fsPath === readme, 'Native generation did not open its selected README');
			assert.equal(vscode.window.activeTextEditor.document.getText(), await fs.readFile(readme, 'utf8'));
			assert.ok((await fs.stat(path.join(path.dirname(readme), 'src/native_host_sdk/__init__.py'))).isFile());
			assert.match(await fs.readFile(path.join(path.dirname(readme), 'pyproject.toml'), 'utf8'), /packages=\["src\/native_host_sdk"\]/);
			await capture('09-native-generated-python-readme');
			await diskUnchanged();
			return { profile: 'python-http', readme, importName: 'native_host_sdk' };
		});
		await check(CHECKS.shutdownReady, async () => {
			await startWatch();
			watchPid = undefined;
			report.shutdownWatchPid = await oneWatch('watch before host shutdown', undefined);
			assert.match(pair.modified.getText(), /native-A/);
			await capture('10-native-watch-before-shutdown');
			await diskUnchanged();
			return { shutdownWatchPid: report.shutdownWatchPid };
		});
		report.status = 'passed';
	} catch (error) {
		report.status = 'failed';
		report.error = error.stack ?? String(error);
		if (page) {
			try { await capture('failure-native-ui'); } catch (captureError) { report.captureError = String(captureError); }
		}
		throw error;
	} finally {
		changes.dispose();
		report.completedAt = new Date().toISOString();
		await save();
		console.log(`[suspect-native-host] ${report.status}: ${report.checks.filter((entry) => entry.status === 'passed').length}/${report.checks.length} checks; ${reportPath}`);
	}
};
