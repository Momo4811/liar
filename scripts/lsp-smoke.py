"""Drive the real liar-lsp binary over stdio and check it behaves.

Not a unit test - a genuine LSP handshake against the shipped binary, so a
failure here means the thing an editor will actually launch is broken.

stdin is held open across the conversation, as a real editor holds it. Closing
it early makes tower-lsp cancel whatever request is in flight, which looks like
a server bug and is not one.
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import threading

BINARY = sys.argv[1]
BUGGY = """\
async def save_user(user):
    pass


async def handle(request):
    save_user(request.user)
"""


def frame(payload: dict) -> bytes:
    body = json.dumps(payload).encode()
    return f"Content-Length: {len(body)}\r\n\r\n".encode() + body


class Reader(threading.Thread):
    """Collects framed messages off the server's stdout until it closes."""

    def __init__(self, stream):
        super().__init__(daemon=True)
        self.stream = stream
        self.messages: list[dict] = []

    def run(self) -> None:
        while True:
            header = b""
            while not header.endswith(b"\r\n\r\n"):
                byte = self.stream.read(1)
                if not byte:
                    return
                header += byte

            length = 0
            for line in header.decode().split("\r\n"):
                if line.lower().startswith("content-length:"):
                    length = int(line.split(":")[1])

            body = self.stream.read(length)
            if not body:
                return
            self.messages.append(json.loads(body))

    def wait_for(self, predicate, timeout: float = 30.0):
        import time

        deadline = time.time() + timeout
        while time.time() < deadline:
            for message in list(self.messages):
                if predicate(message):
                    return message
            time.sleep(0.05)
        return None


def main() -> int:
    workdir = pathlib.Path(tempfile.mkdtemp())
    (workdir / "handlers.py").write_text(BUGGY, encoding="utf-8")

    process = subprocess.Popen(
        [BINARY],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    reader = Reader(process.stdout)
    reader.start()

    failures: list[str] = []

    def send(payload: dict) -> None:
        process.stdin.write(frame(payload))
        process.stdin.flush()

    send({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "processId": None,
            "rootUri": workdir.as_uri(),
            "capabilities": {},
            "initializationOptions": {"liar": {"tone": "brutal"}},
        },
    })

    initialize = reader.wait_for(lambda m: m.get("id") == 1)
    if not initialize or "result" not in initialize:
        failures.append(f"initialize failed: {initialize}")
    else:
        capabilities = initialize["result"]["capabilities"]
        if not capabilities.get("codeActionProvider"):
            failures.append("did not advertise codeActionProvider")
        if "textDocumentSync" not in capabilities:
            failures.append("did not advertise textDocumentSync")
        if initialize["result"].get("serverInfo", {}).get("name") != "liar":
            failures.append("serverInfo.name was not 'liar'")

    send({"jsonrpc": "2.0", "method": "initialized", "params": {}})

    published = reader.wait_for(
        lambda m: m.get("method") == "textDocument/publishDiagnostics"
        and m["params"]["diagnostics"]
    )

    c1 = None
    if not published:
        failures.append("published no diagnostics")
    else:
        c1 = next(
            (d for d in published["params"]["diagnostics"] if d.get("code") == "C1"),
            None,
        )
        if not c1:
            failures.append(f"no C1: {published['params']['diagnostics']}")
        else:
            if "threw it away" not in c1["message"]:
                failures.append(f"brutal tone not applied: {c1['message']!r}")
            if c1["range"]["start"]["line"] != 5:
                failures.append(f"wrong line: {c1['range']}")
            print(f"  diagnostic: {c1['message']}")
            print(f"  at line {c1['range']['start']['line']}, "
                  f"character {c1['range']['start']['character']}")

    # The quick fix, on the diagnostic the server just published.
    if c1:
        send({
            "jsonrpc": "2.0", "id": 3, "method": "textDocument/codeAction",
            "params": {
                "textDocument": {"uri": published["params"]["uri"]},
                "range": c1["range"],
                "context": {"diagnostics": [c1]},
            },
        })
        action = reader.wait_for(lambda m: m.get("id") == 3)
        if not action or not action.get("result"):
            failures.append(f"no code action offered: {action}")
        else:
            first = action["result"][0]
            if first.get("title") != "Add await":
                failures.append(f"unexpected action: {first.get('title')!r}")
            else:
                edits = list(first["edit"]["changes"].values())[0]
                if edits[0]["newText"] != "await ":
                    failures.append(f"wrong edit: {edits[0]}")
                else:
                    print(f"  quick fix:  {first['title']} -> "
                          f"inserts {edits[0]['newText']!r} at "
                          f"line {edits[0]['range']['start']['line']}, "
                          f"character {edits[0]['range']['start']['character']}")

    send({"jsonrpc": "2.0", "id": 4, "method": "shutdown", "params": None})
    if not reader.wait_for(lambda m: m.get("id") == 4):
        failures.append("no response to shutdown")

    send({"jsonrpc": "2.0", "method": "exit", "params": None})
    process.stdin.close()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        failures.append("did not exit after the exit notification")
        process.kill()

    if failures:
        print("\nFAILED:")
        for failure in failures:
            print(f"  - {failure}")
        print(f"\nstderr: {process.stderr.read().decode(errors='replace')[:2000]}")
        return 1

    print("\nHandshake, diagnostics, tone, quick fix and shutdown all correct.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
