/**
 * Renamed settings (`suspect.testBaseUrl` -> `suspect.test.baseUrl`, ...).
 * The old keys still work for two releases: the generated getters read the
 * new key, then the old one, and the server accepts both. On activation the
 * extension offers, once per workspace, to move every old value that is set
 * to its new key at the same level (user, workspace, folder).
 */
import * as vscode from 'vscode';
import { RENAMED_SETTINGS, RenamedSetting } from './settings.generated';

/** VS Code's `ConfigurationTarget` values, so the pure part needs no API. */
export const TARGETS = { global: 1, workspace: 2, workspaceFolder: 3 } as const;

/** The part of `WorkspaceConfiguration.inspect` the move reads. */
export interface Inspected {
	globalValue?: unknown;
	workspaceValue?: unknown;
	workspaceFolderValue?: unknown;
}

/** One value to move. */
export interface PendingMove {
	from: string;
	to: string;
	/** A `ConfigurationTarget`. */
	target: number;
	value: unknown;
}

const isSet = (value: unknown): boolean => value !== undefined && value !== null && value !== '';

/**
 * Every old value that is set at a level where the new key is not. A level
 * that sets both keeps the new one (it already wins) and is left alone.
 * `folder` adds the folder level. Pure.
 */
export function pendingMoves(
	inspect: (key: string) => Inspected | undefined,
	folder: boolean,
	renamed: readonly RenamedSetting[] = RENAMED_SETTINGS,
): PendingMove[] {
	const levels: [keyof Inspected, number][] = [
		['globalValue', TARGETS.global],
		['workspaceValue', TARGETS.workspace],
		...(folder ? [['workspaceFolderValue', TARGETS.workspaceFolder] as [keyof Inspected, number]] : []),
	];
	const moves: PendingMove[] = [];
	for (const { from, to } of renamed) {
		const old = inspect(from);
		const current = inspect(to);
		for (const [level, target] of levels) {
			if (isSet(old?.[level]) && !isSet(current?.[level])) {
				moves.push({ from, to, target, value: old?.[level] });
			}
		}
	}
	return moves;
}

const OFFERED = 'suspect.renamedSettingsOffered';

/** Offers the move once per workspace; moves only on an explicit yes. */
export async function offerSettingsMigration(context: vscode.ExtensionContext): Promise<void> {
	if (context.workspaceState.get<boolean>(OFFERED)) return;
	const scopes: (vscode.WorkspaceFolder | undefined)[] = [undefined, ...(vscode.workspace.workspaceFolders ?? [])];
	const moves: { scope?: vscode.Uri; move: PendingMove }[] = [];
	for (const folder of scopes) {
		const config = vscode.workspace.getConfiguration(undefined, folder?.uri);
		const found = pendingMoves((key) => config.inspect(key), folder !== undefined)
			// User and workspace values are the same for every folder.
			.filter((move) => folder === undefined || move.target === TARGETS.workspaceFolder);
		moves.push(...found.map((move) => ({ scope: folder?.uri, move })));
	}
	if (moves.length === 0) return;
	await context.workspaceState.update(OFFERED, true);
	const names = [...new Set(moves.map(({ move }) => `${move.from} → ${move.to}`))];
	const choice = await vscode.window.showInformationMessage(
		`Suspect renamed ${names.length === 1 ? 'a setting' : `${names.length} settings`} to match .suspect.yaml (${names.join(', ')}). The old ${names.length === 1 ? 'name works' : 'names work'} for two more releases. Move the values now?`,
		'Move',
		'Not now',
	);
	if (choice !== 'Move') return;
	for (const { scope, move } of moves) {
		const config = vscode.workspace.getConfiguration(undefined, scope);
		const target = move.target as vscode.ConfigurationTarget;
		await config.update(move.to, move.value, target);
		await config.update(move.from, undefined, target);
	}
	void vscode.window.showInformationMessage(`Suspect moved ${moves.length} setting value${moves.length === 1 ? '' : 's'}.`);
}
