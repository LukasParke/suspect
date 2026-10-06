// The project manifest, read once and shared by every surface.
//
// `suspect.project.json` is the project's own description of itself: the
// entry spec, the publish profiles and their overlays, the SDK targets,
// the contract-test suites and the base URL they run against. Everything
// the editor shows about a project derives from it rather than from
// guessed file names — the Plex spec is `plex-api-spec.yaml`, its suites
// are `workflows/*.yaml`, and a UI that only knew `openapi.yaml` and
// `*.arazzo.yaml` told that project it had neither.
//
// Pure: no VS Code API, unit-tested through the compiled module.

export interface ManifestTests {
	arazzo: string[];
	base_url: string;
	cassette?: string;
}

export interface CodegenTarget {
	name: string;
	profile: string;
	packageName?: string;
	packageVersion?: string;
	out?: string;
}

export interface PublishProfile {
	name: string;
	overlays: string[];
}

export interface ProjectManifest {
	name?: string;
	entry?: string;
	profiles: PublishProfile[];
	codegen: CodegenTarget[];
	tests?: ManifestTests;
	/** `lint.min_severity` committed in the manifest, when present. */
	lintMinSeverity?: string;
}

const asRecord = (value: unknown): Record<string, unknown> | undefined =>
	typeof value === 'object' && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : undefined;
const asString = (value: unknown): string | undefined => (typeof value === 'string' ? value : undefined);
const asStrings = (value: unknown): string[] =>
	Array.isArray(value) ? value.filter((v): v is string => typeof v === 'string') : [];

/** Reads the `tests` section alone; the shape the CI test stage consumes. */
export function readManifestTests(text: string): ManifestTests | undefined {
	return readProjectManifest(text)?.tests;
}

/** Reads a whole project manifest. Unparseable or non-object text is `undefined`. */
export function readProjectManifest(text: string): ProjectManifest | undefined {
	let parsed: unknown;
	try {
		parsed = JSON.parse(text);
	} catch {
		return undefined;
	}
	const root = asRecord(parsed);
	if (root === undefined) {
		return undefined;
	}

	const profiles: PublishProfile[] = [];
	const publishProfiles = asRecord(asRecord(root.publish)?.profiles);
	if (publishProfiles !== undefined) {
		for (const [name, overlays] of Object.entries(publishProfiles)) {
			profiles.push({ name, overlays: asStrings(overlays) });
		}
	}

	const codegen: CodegenTarget[] = [];
	if (Array.isArray(root.codegen)) {
		for (const entry of root.codegen) {
			const target = asRecord(entry);
			if (target === undefined) continue;
			const profile = asString(target.profile);
			if (profile === undefined) continue;
			codegen.push({
				name: asString(target.name) ?? profile,
				profile,
				packageName: asString(target.package_name),
				packageVersion: asString(target.package_version),
				out: asString(target.out),
			});
		}
	}

	let tests: ManifestTests | undefined;
	const rawTests = asRecord(root.tests);
	if (rawTests !== undefined && Array.isArray(rawTests.arazzo)) {
		tests = {
			arazzo: asStrings(rawTests.arazzo),
			base_url: asString(rawTests.base_url) ?? 'http://localhost:8080',
			cassette: asString(rawTests.cassette),
		};
	}

	return {
		name: asString(root.name),
		entry: asString(root.entry),
		profiles,
		codegen,
		tests,
		lintMinSeverity: asString(asRecord(root.lint)?.min_severity),
	};
}

/** The character VS Code reserves as its test-path delimiter; an ID carrying it is rejected at creation. */
export const TEST_ID_FORBIDDEN = '\u0000';

/**
 * Builds a test item ID from its path parts. Throws rather than letting
 * `createTestItem` do so inside a resolve handler, where the error would
 * empty the whole controller — which is exactly how the Testing view came
 * to show nothing.
 */
export function testId(...parts: string[]): string {
	for (const part of parts) {
		if (part.includes(TEST_ID_FORBIDDEN)) {
			throw new Error(`test id part contains the reserved NUL delimiter: ${JSON.stringify(part)}`);
		}
	}
	return parts.join('#');
}

/** A YAML document is Arazzo when it declares the `arazzo:` version key at the top level. */
export function looksLikeArazzo(head: string): boolean {
	return /^arazzo:\s*['"]?\d/m.test(head);
}

/** The documents found by content that no manifest declares — shown under their own heading. */
export function undeclaredDocuments(found: string[], declared: string[]): string[] {
	const known = new Set(declared);
	return found.filter((file) => !known.has(file));
}
