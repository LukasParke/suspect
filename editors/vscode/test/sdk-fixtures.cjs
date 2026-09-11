const { EventEmitter } = require('node:events');
const { PassThrough } = require('node:stream');
const { setTimeout: delay } = require('node:timers/promises');

/** The process seam: real byte streams, including late output and delayed termination. */
class MockCli extends EventEmitter {
	constructor({ closeOnKill = true } = {}) {
		super();
		this.pid = 12345;
		this.stdout = new PassThrough();
		this.stderr = new PassThrough();
		this.exitCode = null;
		this.signalCode = null;
		this.signals = [];
		this.closeOnKill = closeOnKill;
		this.closed = false;
	}

	send(record) { this.stdout.write(`${JSON.stringify(record)}\n`); }

	kill(signal) {
		this.signals.push(signal);
		if (this.closeOnKill) queueMicrotask(() => this.close(null, signal));
		return true;
	}

	close(code = 0, signal = null) {
		if (this.closed) return;
		this.closed = true;
		this.exitCode = code;
		this.signalCode = signal;
		this.stdout.end();
		this.stderr.end();
		this.emit('close', code, signal);
	}
}

function record(overrides = {}) {
	const status = overrides.status ?? 'drift';
	return {
		format: 'suspect.sdk.session.v1', success: status === 'current' || status === 'written', status,
		compatibilityProfiles: [],
		generation: 1, changedArtifacts: ['typescript/client.ts'], newDocuments: [],
		delta: { compiles: 1, renders: 1, cache_hits: 0 }, stats: { compiles: 1, renders: 1, cache_hits: 0 },
		diagnostics: [], artifacts: [{ path: 'typescript/client.ts', content: 'export const generated = "café 🦀";\n' }],
		...overrides,
	};
}

/** The CLI inventory envelope, independent of the editor's profile/directory map. */
function profileInventory() {
	return {
		format: 'suspect.sdk.profiles.v1',
		compatibilityProfiles: ['legacy-binary-string-v1'],
		profiles: [
			['typescript-http', 'typescript', 'Source-selected HTTP operations, exact codecs and ESM packaging'],
			['rust-http', 'rust', 'Source-selected native HTTP clients, exact codecs and Cargo packaging'],
			['python-http', 'python', 'Source-selected sync/async clients, exact codecs and wheel packaging'],
			['go-http', 'go', 'Source-selected context-aware clients, exact codecs and Go modules'],
			['swift-http', 'swift', 'Source-selected async clients, exact codecs, SwiftPM and DocC'],
			['ruby-http', 'ruby', 'Source-selected keyword clients, exact codecs, gems, RBS and YARD'],
			['csharp-http', 'csharp', 'Source-selected Task clients, exact codecs, NuGet and native .NET docs'],
			['dart-http', 'dart', 'Source-selected Future clients, exact codecs, pub packages and dartdoc'],
			['cpp-http', 'cpp', 'Source-selected C++20 clients, exact codecs, CMake, libcurl and Doxygen'],
			['kotlin-http', 'kotlin', 'Source-selected coroutine clients, exact codecs, Maven and Dokka'],
			['php-http', 'php', 'Source-selected typed PHP clients, exact codecs, Composer and PHPDoc'],
			['java-http', 'java', 'Source-selected immutable Java clients, exact codecs, CompletableFuture, Maven and Javadoc'],
		].map(([profile, directory, description]) => ({ profile, directory, description, experimental: true })),
	};
}

async function until(predicate, message = 'condition was not reached') {
	const deadline = Date.now() + 3000;
	while (!await predicate()) {
		if (Date.now() > deadline) throw new Error(message);
		await delay(5);
	}
}

module.exports = { MockCli, record, profileInventory, until };
