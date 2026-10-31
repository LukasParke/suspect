// Runs inside a real VS Code extension host (see run.cjs). Writes what it
// observed to SUSPECT_SMOKE_REPORT; run.cjs owns the assertions.
const fs = require('node:fs');
const vscode = require('vscode');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function waitFor(probe, budgetMs) {
	const end = Date.now() + budgetMs;
	while (Date.now() < end) {
		const value = await probe();
		if (value) return value;
		await sleep(250);
	}
	return null;
}

exports.run = async function run() {
	const report = {};
	try {
		const extension = vscode.extensions.getExtension('suspect.suspect-vscode');
		const uri = vscode.Uri.file(process.env.SUSPECT_SMOKE_SPEC);
		await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
		report.activated = Boolean(await waitFor(() => extension.isActive, 20000));
		// Only the server advertises this id, so it exists only once the
		// client reached running state.
		report.serverCommandRegistered = Boolean(await waitFor(
			async () => (await vscode.commands.getCommands(true)).includes('suspect.showRefs'), 20000));
		const before = await waitFor(() => {
			const found = vscode.languages.getDiagnostics(uri);
			return found.length ? found : null;
		}, 20000);
		report.before = before ? before.map((d) => d.severity) : [];
		await vscode.workspace.getConfiguration('suspect')
			.update('lint.minSeverity', 'error', vscode.ConfigurationTarget.Workspace);
		await waitFor(() => vscode.languages.getDiagnostics(uri).every((d) => d.severity === vscode.DiagnosticSeverity.Error), 20000);
		await sleep(2000);
		report.after = vscode.languages.getDiagnostics(uri).map((d) => d.severity);
		// The explain report (what the status-bar item opens, and where the
		// floor picker reads its floor) names the editor layer over the file.
		await vscode.commands.executeCommand('suspect.explainConfig');
		const explained = await waitFor(() => {
			const text = vscode.window.activeTextEditor?.document.getText() ?? '';
			return text.includes('# Suspect configuration') ? text : null;
		}, 20000);
		report.floorRow = explained ? explained.split('\n').find((line) => line.includes('`lint.min_severity`')) : null;
		report.settingsFile = explained ? explained.split('\n').find((line) => line.startsWith('Settings file:')) : null;
	} catch (error) {
		report.error = String(error && error.stack || error);
	}
	fs.writeFileSync(process.env.SUSPECT_SMOKE_REPORT, JSON.stringify(report, null, 2));
};
