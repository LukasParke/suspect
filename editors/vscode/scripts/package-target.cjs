#!/usr/bin/env node
// Packages a platform VSIX: the extension plus the suspect CLI for one
// VS Code target, at bin/suspect (bin/suspect.exe on Windows). runner.ts
// prefers that copy over PATH. The universal VSIX is `npm run package`.
//
//   node scripts/package-target.cjs --target darwin-arm64 --binary <suspect> [--out <file.vsix>]
//
// The release workflow (package-editors.yml) runs this once per row of
// TARGETS with the binary from the matching suspect-cli-{triple} archive.

const cp = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.join(__dirname, '..');

/** VS Code target -> the P5 release triple whose binary it bundles. Linux uses the static musl build. */
const TARGETS = {
	'darwin-arm64': 'aarch64-apple-darwin',
	'darwin-x64': 'x86_64-apple-darwin',
	'linux-x64': 'x86_64-unknown-linux-musl',
	'linux-arm64': 'aarch64-unknown-linux-musl',
	'alpine-x64': 'x86_64-unknown-linux-musl',
	'alpine-arm64': 'aarch64-unknown-linux-musl',
	'win32-x64': 'x86_64-pc-windows-msvc',
};

function arg(name) {
	const at = process.argv.indexOf(name);
	return at > 0 ? process.argv[at + 1] : undefined;
}

function main() {
	const target = arg('--target');
	const binary = arg('--binary');
	if (!target || !TARGETS[target] || !binary) {
		console.error(`usage: package-target.cjs --target <${Object.keys(TARGETS).join('|')}> --binary <path> [--out <file>]`);
		process.exit(2);
	}
	if (!fs.statSync(binary, { throwIfNoEntry: false })?.isFile()) {
		console.error(`no binary at ${binary}`);
		process.exit(2);
	}
	const version = JSON.parse(fs.readFileSync(path.join(ROOT, 'package.json'), 'utf8')).version;
	const out = path.resolve(arg('--out') ?? path.join(ROOT, `suspect-vscode-${target}-${version}.vsix`));
	const exe = target.startsWith('win32') ? 'suspect.exe' : 'suspect';
	const bin = path.join(ROOT, 'bin');
	fs.rmSync(bin, { recursive: true, force: true });
	fs.mkdirSync(bin);
	try {
		fs.copyFileSync(binary, path.join(bin, exe));
		fs.chmodSync(path.join(bin, exe), 0o755);
		const vsce = path.join(ROOT, 'node_modules', '.bin', process.platform === 'win32' ? 'vsce.cmd' : 'vsce');
		const result = cp.spawnSync(vsce, ['package', '--no-dependencies', '--target', target, '--out', out], {
			cwd: ROOT, stdio: 'inherit', shell: process.platform === 'win32',
		});
		if (result.status !== 0) process.exit(result.status ?? 1);
	} finally {
		fs.rmSync(bin, { recursive: true, force: true });
	}
	console.log(`${out} (${TARGETS[target]})`);
}

module.exports = { TARGETS };

if (require.main === module) main();
