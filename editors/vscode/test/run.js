// Runs the extension inside a real copy of VS Code against a real liar-lsp.
//
// The extension under test is a copy of exactly the files `vsce` would put in
// the .vsix - no node_modules, no sources. Testing the development folder
// instead would pass with a package that cannot even load its own imports,
// which is how the first packaged build shipped.
//
// The server binary comes from LIAR_LSP, or the workspace's release build.

const { execFileSync } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { runTests } = require("@vscode/test-electron");

const root = path.resolve(__dirname, "..");

function packagedCopy() {
  const vsce = path.join(root, "node_modules", ".bin", process.platform === "win32" ? "vsce.cmd" : "vsce");
  const listing = execFileSync(vsce, ["ls", "--no-dependencies"], {
    cwd: root,
    encoding: "utf8",
    shell: process.platform === "win32",
  });

  const target = fs.mkdtempSync(path.join(os.tmpdir(), "liar-vsix-"));
  for (const file of listing.split(/\r?\n/).filter(Boolean)) {
    fs.mkdirSync(path.dirname(path.join(target, file)), { recursive: true });
    fs.copyFileSync(path.join(root, file), path.join(target, file));
  }
  return target;
}

function serverBinary() {
  if (process.env.LIAR_LSP) {
    return path.resolve(process.env.LIAR_LSP);
  }
  const name = process.platform === "win32" ? "liar-lsp.exe" : "liar-lsp";
  return path.resolve(root, "..", "..", "target", "release", name);
}

async function main() {
  const binary = serverBinary();
  if (!fs.existsSync(binary)) {
    console.error(`no liar-lsp at ${binary}: run 'cargo build --release -p liar-lsp' first`);
    process.exit(1);
  }

  // A fresh copy of the fixture each run, so nothing left by a previous run
  // can make this one pass.
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "liar-workspace-"));
  fs.cpSync(path.join(__dirname, "fixture"), workspace, { recursive: true });

  try {
    await runTests({
      extensionDevelopmentPath: packagedCopy(),
      extensionTestsPath: path.join(__dirname, "suite.js"),
      // A fresh profile too: the suite changes settings, and a tone left over
      // from the last run would fail this one.
      launchArgs: [
        workspace,
        "--user-data-dir",
        fs.mkdtempSync(path.join(os.tmpdir(), "liar-profile-")),
        "--disable-extensions",
        "--skip-welcome",
        "--skip-release-notes",
      ],
      extensionTestsEnv: { LIAR_LSP: binary },
    });
  } catch (error) {
    console.error(`the extension tests failed: ${error}`);
    process.exit(1);
  }
}

main();
