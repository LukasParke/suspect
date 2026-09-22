const assert = require('node:assert/strict');
const cp = require('node:child_process');
const { once } = require('node:events');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { availableSdkProfiles } = require('../dist/generation.js');
const { profileInventory } = require('./sdk-fixtures.cjs');

/** Real execFile processes/limits; substitute only a portable Node fixture for the CLI executable. */
async function discoveryCli(t, source) {
	const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect discovery ; prose '));
	const script = path.join(root, 'CLI fixture.cjs');
	await fs.writeFile(script, source);
	const execFile = cp.execFile;
	const children = [];
	const calls = [];
	t.mock.method(cp, 'execFile', (binary, args, options, reply) => {
		calls.push([binary, args, options]);
		const child = execFile(process.execPath, [script, ...args], options, reply);
		children.push(child);
		return child;
	});
	t.after(async () => {
		for (const child of children) {
			if (child.exitCode === null && child.signalCode === null) {
				const closed = once(child, 'close');
				child.kill('SIGKILL');
				await closed;
			}
		}
		await fs.rm(root, { recursive: true, force: true });
	});
	return { root, children, calls, binary: path.join(root, 'suspect ; $(prose)') };
}

test('profile discovery reads the real execFile envelope with exact argv, UTF-8 descriptions and bounded IO', async (t) => {
	const inventory = profileInventory();
	inventory.profiles[0].description = 'café 🦀 [run](command:execute) $(touch sentinel)';
	const cli = await discoveryCli(t, `
		const assert = require('node:assert/strict');
		assert.deepEqual(process.argv.slice(2), ['codegen-profiles', '--format', 'json']);
		process.stdout.write(${JSON.stringify(JSON.stringify(inventory, null, 2))});
	`);
	const profiles = await availableSdkProfiles(cli.binary, cli.root);
	assert.deepEqual(profiles, inventory.profiles.map(({ profile, directory, description }) => ({ profile, directory, description })));
	assert.deepEqual(cli.calls, [[cli.binary, ['codegen-profiles', '--format', 'json'], {
		cwd: cli.root, encoding: 'utf8', timeout: 5000, maxBuffer: 256 * 1024, windowsHide: true, shell: false, killSignal: 'SIGKILL',
	}]]);
	assert.equal(cli.children[0].exitCode, 0);
	await assert.rejects(fs.stat(path.join(cli.root, 'sentinel')), { code: 'ENOENT' });
});

test('profile discovery rejects a real oversized response instead of presenting a truncated inventory', async (t) => {
	const inventory = profileInventory();
	const cli = await discoveryCli(t, `process.stdout.write(${JSON.stringify(JSON.stringify(inventory))} + ' '.repeat(256 * 1024));`);
	await assert.rejects(availableSdkProfiles(cli.binary, cli.root), /Unable to read SDK profiles.*stdout maxBuffer length exceeded/s);
	assert.equal(cli.children.length, 1);
});

test('profile discovery deadline kills a real CLI that ignores SIGTERM, even after valid stdout', { timeout: 8000 }, async (t) => {
	const cli = await discoveryCli(t, `
		process.on('SIGTERM', () => {});
		process.stdout.write(${JSON.stringify(JSON.stringify(profileInventory()))});
		setInterval(() => {}, 1000);
	`);
	let deadline;
	try {
		await assert.rejects(Promise.race([
			availableSdkProfiles(cli.binary, cli.root),
			new Promise((_, reject) => { deadline = setTimeout(() => reject(new Error('SDK discovery remained alive past its deadline')), 6500); }),
		]), /Unable to read SDK profiles/);
		assert.equal(cli.children[0].signalCode, 'SIGKILL');
	} finally { clearTimeout(deadline); }
});
