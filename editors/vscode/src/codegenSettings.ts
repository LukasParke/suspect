/**
 * The `codegen.*` values the SDK commands and Live SDK Preview start from:
 * the editor setting (`suspect.codegen.*`), else the value the server
 * resolved from `.suspect.yaml`
 * (`suspect/explainConfig`), else the setting table's default.
 */
import type * as vscode from 'vscode';
import type { LanguageClient } from 'vscode-languageclient/node';
import { explainConfig, ExplainReport, explainRow } from './explainConfig';
import * as settings from './settings.generated';

export interface CodegenDefaults {
	packageName: string;
	packageVersion: string;
	importNames: Readonly<Record<string, string>>;
	operationIds: readonly string[];
	/** Unchecked: `readSdkCompatibilityProfiles` rejects unknown names. */
	compatibilityProfiles: unknown;
	/** Session config path; `''` asks for one. */
	sessionConfig: string;
	/** Initial session output root, relative to the session config's directory. */
	sessionOut: string;
}

const IMPORT_NAMES = 'codegen.import_names.';

function text(value: unknown): string | undefined {
	return typeof value === 'string' && value !== '' ? value : undefined;
}

/** The defaults for `scope` given an explain report (or none). Pure apart from the settings read. */
export function codegenDefaultsFrom(report: ExplainReport | undefined, scope?: vscode.Uri): CodegenDefaults {
	const resolved = (key: string): unknown => explainRow(report, key)?.value;
	const fileImportNames: Record<string, string> = {};
	for (const row of report?.values ?? []) {
		if (row.key.startsWith(IMPORT_NAMES) && typeof row.value === 'string') {
			fileImportNames[row.key.slice(IMPORT_NAMES.length)] = row.value;
		}
	}
	const fileOperationIds = resolved('codegen.operation_ids');
	return {
		packageName: settings.codegenPackageName(scope) ?? text(resolved('codegen.package_name')) ?? '',
		packageVersion: settings.codegenPackageVersion(scope) ?? text(resolved('codegen.package_version')) ?? '',
		importNames: settings.codegenImportNames(scope) ?? fileImportNames,
		operationIds: settings.codegenOperationIds(scope)
			?? (Array.isArray(fileOperationIds) && fileOperationIds.every((id) => typeof id === 'string') ? fileOperationIds : []),
		compatibilityProfiles: settings.codegenCompatibilityProfiles(scope) ?? resolved('codegen.compatibility_profiles') ?? [],
		sessionConfig: settings.codegenSessionConfig(scope) ?? text(resolved('codegen.session.config')) ?? '',
		sessionOut: settings.codegenSessionOut(scope) ?? text(resolved('codegen.session.out')) ?? 'sdk-out',
	};
}

/** The defaults for `scope`, asking the server for the committed values. */
export async function codegenDefaults(client: LanguageClient | undefined, scope?: vscode.Uri): Promise<CodegenDefaults> {
	return codegenDefaultsFrom(await explainConfig(client, scope), scope);
}

/** Settings whose change ends a running SDK session. */
export const SESSION_SETTINGS = [
	'suspect.codegen.session.config',
	'suspect.codegen.session.out',
] as const;
