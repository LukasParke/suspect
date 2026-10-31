#!/usr/bin/env node
// Bundles the extension into dist/extension.js (run by `vscode:prepublish`,
// after `tsc` has type-checked), so the VSIX carries no node_modules.
// `vscode` is provided by the host. The other dist/*.js files are tsc's
// per-module output, which the headless tests load; .vscodeignore leaves
// them out of the VSIX.

const path = require('node:path');
const esbuild = require('esbuild');

const root = path.join(__dirname, '..');

esbuild
	.build({
		entryPoints: [path.join(root, 'src', 'extension.ts')],
		outfile: path.join(root, 'dist', 'extension.js'),
		bundle: true,
		platform: 'node',
		format: 'cjs',
		target: 'node18',
		external: ['vscode'],
		sourcemap: false,
		logLevel: 'warning',
	})
	.catch(() => process.exit(1));
