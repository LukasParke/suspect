const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const cfg = vscode.workspace.getConfiguration("suspect");
  const serverPath = cfg.get("server.path", "suspect");

  // { run, debug } shape is the languageclient ServerOptions at runtime.
  const serverOptions = {
    run: { command: serverPath, args: ["lsp"] },
    debug: { command: serverPath, args: ["lsp"] },
  };

  const clientOptions = {
    documentSelector: [
      { scheme: "file", language: "yaml" },
      { scheme: "file", language: "json" },
      { scheme: "untitled", language: "yaml" },
      { scheme: "untitled", language: "json" },
    ],
    synchronize: {
      configurationSection: "suspect",
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.y*ml"),
    },
    outputChannelName: "suspect LSP",
  };

  client = new LanguageClient("suspectLsp", "suspect", serverOptions, clientOptions);
  context.subscriptions.push(client);
  client.start();

  context.subscriptions.push(
    vscode.commands.registerCommand("suspect.restart", async () => {
      await client.stop();
      client.start();
    })
  );
}

function deactivate() {
  if (client) return client.stop();
}

module.exports = { activate, deactivate };
