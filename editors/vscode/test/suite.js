// What a user would see, checked inside VS Code itself.
//
// Plain asserts rather than a test framework: there are four things to check
// and they run in order, each depending on the last.

const assert = require("assert");
const path = require("path");
const vscode = require("vscode");

const TIMEOUT_MS = 30000;

async function waitFor(what, read) {
  const deadline = Date.now() + TIMEOUT_MS;
  while (Date.now() < deadline) {
    const value = read();
    if (value) {
      return value;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
}

function liarDiagnostics(uri) {
  return vscode.languages.getDiagnostics(uri);
}

async function run() {
  await vscode.workspace
    .getConfiguration("liar")
    .update("path", process.env.LIAR_LSP, vscode.ConfigurationTarget.Global);

  const folder = vscode.workspace.workspaceFolders[0].uri.fsPath;
  const uri = vscode.Uri.file(path.join(folder, "handlers.py"));
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  // 1. Opening a Python file activates the extension and starts the server.
  const extension = vscode.extensions.all.find((e) => e.packageJSON.name === "liar");
  assert.ok(extension, "the extension is installed");
  await waitFor("activation", () => extension.isActive);

  // 2. Findings appear, one for each un-awaited call.
  const findings = await waitFor("two C1 findings", () => {
    const found = liarDiagnostics(uri).filter((d) => d.code === "C1");
    return found.length === 2 ? found : undefined;
  });
  const [inAsync, inSync] = findings.sort((a, b) => a.range.start.line - b.range.start.line);
  assert.strictEqual(inAsync.range.start.line, 5);
  assert.strictEqual(inSync.range.start.line, 9);
  assert.match(inAsync.message, /never awaited/, "the default tone is dry");

  // 3. The quick fix is offered inside async def, and only there.
  const fixesFor = (diagnostic) =>
    vscode.commands.executeCommand(
      "vscode.executeCodeActionProvider",
      uri,
      diagnostic.range,
      vscode.CodeActionKind.QuickFix.value,
    );
  const asyncFixes = (await fixesFor(inAsync)).filter((a) => a.title === "Add await");
  const syncFixes = (await fixesFor(inSync)).filter((a) => a.title === "Add await");
  assert.strictEqual(asyncFixes.length, 1, "Add await is offered in async def");
  assert.strictEqual(syncFixes.length, 0, "Add await is not offered in a plain def");

  // 4. Changing the tone restarts the server and rewords the findings.
  await vscode.workspace
    .getConfiguration("liar")
    .update("tone", "brutal", vscode.ConfigurationTarget.Global);
  await waitFor("the brutal tone", () =>
    liarDiagnostics(uri).some((d) => /threw it away/.test(d.message)),
  );

  // Applying the fix clears the finding once saved.
  await vscode.workspace.applyEdit(asyncFixes[0].edit);
  await document.save();
  await waitFor("the fixed finding to clear", () => {
    const remaining = liarDiagnostics(uri).filter((d) => d.code === "C1");
    return remaining.length === 1 && remaining[0].range.start.line === 9;
  });

  console.log("activation, findings, quick fix, tone and save all correct");
}

module.exports = { run };
