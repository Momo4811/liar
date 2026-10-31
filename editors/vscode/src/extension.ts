// A launcher for the liar language server, and nothing else.
//
// No analysis logic lives here. If a rule, a threshold, or a message ever
// appears in this file, it is in the wrong crate: the server is the only thing
// that decides what a finding is, so that the editor and the command line can
// never disagree about it.

import { execFileSync } from "child_process";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

const BINARY = "liar-lsp";

let client: LanguageClient | undefined;

// One channel for the life of the extension. Creating it per start would add
// another "liar" entry to the Output panel every time the tone changed.
let output: vscode.OutputChannel | undefined;

// Files the server is told about when they change on disk: liar.toml, because
// it decides which files and checks are in force, and Python files created or
// deleted outside the editor, because they change what resolves in their
// neighbours. Created once and shared by every server started.
let watchers: vscode.FileSystemWatcher[] = [];

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  output = vscode.window.createOutputChannel("liar");
  context.subscriptions.push(output);

  watchers = [
    vscode.workspace.createFileSystemWatcher("**/liar.toml"),
    vscode.workspace.createFileSystemWatcher("**/*.{py,pyi}"),
  ];
  context.subscriptions.push(...watchers);

  context.subscriptions.push(
    vscode.commands.registerCommand("liar.restart", restart),
  );

  // A change of tone is applied by restarting the server, which reads it once
  // at initialize. Restarting the server is not restarting the editor, and it
  // takes long enough to notice only on very large workspaces.
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration(async (event) => {
      if (event.affectsConfiguration("liar")) {
        await restart();
      }
    }),
  );

  await start();
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}

async function restart(): Promise<void> {
  await client?.stop();
  client = undefined;
  await start();
}

async function start(): Promise<void> {
  const settings = vscode.workspace.getConfiguration("liar");

  if (!settings.get<boolean>("enable", true)) {
    return;
  }

  const command = locateServer(settings.get<string>("path", ""));
  if (!command) {
    reportMissingBinary();
    return;
  }

  const serverOptions: ServerOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "python" }],
    initializationOptions: {
      liar: { tone: settings.get<string>("tone", "dry") },
    },
    outputChannel: output,
    synchronize: { fileEvents: watchers },
  };

  client = new LanguageClient("liar", "liar", serverOptions, clientOptions);

  try {
    await client.start();
  } catch (error) {
    vscode.window.showErrorMessage(`liar: the language server failed to start: ${error}`);
    client = undefined;
  }
}

/**
 * Finds the server binary, or returns undefined.
 *
 * A configured path is trusted as given. Otherwise the binary is looked for on
 * PATH by running it — checking that a name resolves is the only reliable way,
 * since PATH lookup rules differ per platform and `where`/`which` are not
 * always present.
 */
function locateServer(configured: string): string | undefined {
  if (configured.trim().length > 0) {
    return configured.trim();
  }

  try {
    execFileSync(BINARY, ["--help"], { stdio: "ignore", timeout: 5000 });
    return BINARY;
  } catch (error) {
    // A binary that exists but exits non-zero on --help is still a binary we
    // can launch; only "not found" means it is genuinely absent.
    const code = (error as NodeJS.ErrnoException).code;
    return code === "ENOENT" ? undefined : BINARY;
  }
}

function reportMissingBinary(): void {
  const build = "Show me how";
  vscode.window
    .showWarningMessage(
      `liar: could not find '${BINARY}'. Build it, or set liar.path to its location.`,
      build,
    )
    .then((choice) => {
      if (choice === build) {
        vscode.env.openExternal(
          vscode.Uri.parse("https://github.com/Momo4811/liar/tree/main/editors/vscode#requirements"),
        );
      }
    });
}
