#!/usr/bin/env python3
"""End-to-end test: real TTY, curl, bash, image bridge; fake Responses endpoint.
Run: cargo build --release && python3 tests/smoke.py [path/to/mu]
No API key, Python packages, or provider required.
"""
import base64
import fcntl
import http.server
import json
import os
from pathlib import Path
import pty
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time

binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/mu").resolve()
requests, failures = [], []


def message(text):
    return {"type": "message", "id": "msg", "status": "completed", "role": "assistant",
            "content": [{"type": "output_text", "text": text, "annotations": []}]}


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        try:
            assert self.path == "/v1/responses"
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            requests.append(body)
            n = len(requests)
            assert len(body["tools"]) == 1
            assert body["tools"][0]["description"] == "Special bash commands: view_image"
            assert body["store"] is False
            assert body["instructions"] == expected_instructions
            assert body["tools"][0]["parameters"]["properties"]["timeoutMs"] == {"type": "integer"}
            if n == 1:
                output = [
                    {"type": "reasoning", "id": "reason1", "encrypted_content": "opaque",
                     "summary": [{"type": "summary_text", "text": "Inspecting an image."}]},
                    {"type": "function_call", "id": "callitem", "call_id": "call1", "name": "bash",
                     "arguments": json.dumps({"command": "touch ran; printf 'hello µ'; view_image pixel.png; sleep 1; printf ' done'"})},
                ]
            elif n == 2:
                items = body["input"]
                result = next(i for i in items if i.get("type") == "function_call_output")
                assert "hello µ" in result["output"] and "[exit 0]" in result["output"]
                assert any(i.get("encrypted_content") == "opaque" for i in items)
                image = next(i for i in items if isinstance(i.get("content"), list) and any(p["type"] == "input_image" for p in i["content"]))
                assert image["content"][1]["image_url"].startswith("data:image/png;base64,")
                context = items[-1]["content"]
                assert context.startswith("/review steer now @notes.txt"), context
                assert "Only loaded on demand." in context and "ORIGINAL NOTES" in context
                assert "UPDATED NOTES" not in context and "UPDATED SKILL" not in context
                output = [message("# Done\nImage received; steering followed.")]
            elif n == 3:
                assert body["model"] == "test-high"
                assert body["reasoning"] == {"effort": "high", "summary": "auto"}
                assert body["input"] == [{"role": "user", "content": "first"}, {"role": "user", "content": "branch"}]
                output = [message("Branched.")]
            elif n == 4:
                assert body["input"][-1]["content"] == "after resume"
                assert any(i.get("content") == "branch" for i in body["input"])
                assert not any(i.get("type") == "function_call" for i in body["input"])
                output = [message("Resumed.")]
            elif n == 5:
                output = [{"type": "function_call", "id": "slowitem", "call_id": "slow", "name": "bash",
                           "arguments": json.dumps({"command": "sleep 60 & echo $! > child.pid; touch slow; wait"})}]
            elif n == 6:
                # Incomplete responses must never execute their partial calls.
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                self.event({"type": "response.output_item.done", "output_index": 0,
                            "item": {"type": "function_call", "call_id": "bad", "name": "bash",
                                     "arguments": json.dumps({"command": "touch must-not-run"})}})
                return
            elif n == 7:
                output = [message("Retried.")]
            elif n == 8:
                context = body["input"][-1]["content"]
                assert context.startswith("inspect @src/main.rs "), context
                assert "fn main() { /* ORIGINAL SOURCE */ }" in context
                output = [message("File attached.")]
            elif n == 9:
                context = body["input"][-1]["content"]
                assert context.startswith('/review check @"notes space.txt" '), context
                assert "UPDATED SKILL" in context and "SPACED FILE" in context
                history = [i.get("content", "") for i in body["input"] if i.get("role") == "user"]
                assert any(isinstance(c, str) and "ORIGINAL SOURCE" in c for c in history)
                assert not any(isinstance(c, str) and "CHANGED SOURCE" in c for c in history)
                output = [message("Skill and spaced file attached.")]
            else:
                raise AssertionError(f"Unexpected request {n}")
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            for i, item in enumerate(output):
                self.event({"type": "response.output_item.added", "output_index": i,
                            "item": {**item, "arguments": "", "content": [], "summary": []}})
                typ = item["type"]
                if typ == "reasoning":
                    kind, delta = "response.reasoning_summary_text.delta", "Inspecting an image."
                elif typ == "function_call":
                    kind, delta = "response.function_call_arguments.delta", item["arguments"]
                else:
                    kind, delta = "response.output_text.delta", item["content"][0]["text"]
                for part in (delta[:len(delta)//2], delta[len(delta)//2:]):
                    self.event({"type": kind, "output_index": i, "delta": part})
                    time.sleep(0.02)
                self.event({"type": "response.output_item.done", "output_index": i, "item": item})
            self.event({"type": "response.completed", "response": {"output": output, "usage": {
                "input_tokens": 4096 + n * 100, "output_tokens": 15,
                "input_tokens_details": {"cached_tokens": 2048 if n == 1 else 0}}}})
        except Exception as e:
            failures.append(repr(e))
            raise

    def event(self, event):
        data = ("data: " + json.dumps(event, ensure_ascii=False) + "\r\n\r\n").encode()
        # Split in awkward places, including through UTF-8 sequences.
        for pos in range(0, len(data), 7):
            self.wfile.write(data[pos:pos+7])
        self.wfile.flush()


def wait(predicate, label, timeout=10):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if failures:
            raise AssertionError(failures)
        if predicate():
            return
        if proc.poll() is not None:
            raise AssertionError(f"mu exited {proc.returncode}: {screen[-3000:]!r}")
        time.sleep(0.025)
    raise AssertionError(f"Timed out: {label}\n{screen[-3000:]!r}")


with tempfile.TemporaryDirectory(prefix="mu-test-") as temp:
    root = Path(temp)
    project = root / "project"
    project.mkdir()
    (project / "AGENTS.md").write_text("Be minimal.")
    skill = root / ".agents/skills/test/SKILL.md"
    skill.parent.mkdir(parents=True)
    skill.write_text("---\nname: review\ndescription: test skill\n---\nOnly loaded on demand.")
    (project / "notes.txt").write_text("ORIGINAL NOTES")
    (project / "notes space.txt").write_text("SPACED FILE")
    (project / "src").mkdir()
    (project / "src/main.rs").write_text("fn main() { /* ORIGINAL SOURCE */ }")
    (project / ".gitignore").write_text("ignored/\n")
    (project / "ignored").mkdir()
    (project / "ignored/main.rs").write_text("must not appear in file search")
    expected_instructions = f"You are coding agent\n\nProject Instructions:\nBe minimal.\n\nSkills:\n{skill}: test skill"
    (project / "pixel.png").write_bytes(base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aB1sAAAAASUVORK5CYII="))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 110, 0, 0))
    environment = {**os.environ, "HOME": str(root), "XDG_STATE_HOME": str(root / "state"),
                   "MU_BASE_URL": f"http://127.0.0.1:{server.server_port}/v1", "MU_MODEL": "test",
                   "TERM": "xterm-256color"}
    environment.pop("MU_EFFORT", None)
    environment.pop("WAYLAND_DISPLAY", None)
    environment.pop("DISPLAY", None)
    proc = subprocess.Popen([str(binary)], stdin=slave, stdout=slave, stderr=slave, cwd=project, env=environment)
    os.close(slave)
    screen = bytearray()

    def drain():
        while True:
            try:
                data = os.read(master, 65536)
                if not data:
                    break
                screen.extend(data)
            except OSError:
                break
    threading.Thread(target=drain, daemon=True).start()

    def send(text):
        os.write(master, text.encode())

    def saved():
        return [json.loads(p.read_text()) for p in (root / "state/mu").glob("*.json")]

    def original():
        return next((s for s in saved() if s["nodes"]), {"nodes": []})

    def has_text(text):
        return any(text in b["text"] for n in original()["nodes"] for b in n["blocks"])

    try:
        wait(lambda: b"/model" in screen, "initial screen")
        rss = next(l.strip() for l in Path(f"/proc/{proc.pid}/status").read_text().splitlines() if l.startswith("VmRSS:"))
        # Configuration, /new, and selecting an empty root don't create sessions.
        send("/model test\r/new\r/tree\r")
        wait(lambda: b"conversation tree" in screen, "empty tree")
        send("\r/resume\r")
        wait(lambda: b"No saved sessions" in screen, "empty resume")
        assert not (root / "state/mu").exists()
        send("first\r")
        wait(lambda: (project / "ran").exists(), "bash started")
        offset = len(screen)
        send("/review steer now @notes.txt\r")
        wait(lambda: b"queued:" in screen[offset:], "queued skill/file snapshot")
        (project / "notes.txt").write_text("UPDATED NOTES")
        skill.write_text("---\nname: review\ndescription: test skill\n---\nUPDATED SKILL")
        wait(lambda: has_text("Image received"), "steering response")
        assert has_text("cache miss")
        send("\x0f")  # expand thoughts/tools
        send("/mo test-high high\r")
        time.sleep(0.15)
        send("/tree\r")
        time.sleep(0.15)
        send("\x1b[H\x1b[B\r")  # root -> first user
        time.sleep(0.15)
        send("branch\r")
        wait(lambda: has_text("Branched."), "branch response")
        s = original()
        assert len([n for n in s["nodes"] if n["parent"] == 0]) == 2
        assert (project / "ran").exists(), "branching changed the project"
        offset = len(screen)
        send("/n\r/r\r")
        wait(lambda: b"Choose a command:" in screen[offset:], "ambiguous slash prefix")
        assert len(requests) == 3, "ambiguous command invoked the model"
        send("\t\r")  # Tab selects /resume, Enter executes it.
        wait(lambda: b"resume session" in screen[offset:], "resume picker")
        assert len(saved()) == 1, "/new persisted an empty session"
        session_path = next((root / "state/mu").glob("*.json"))
        modified = session_path.stat().st_mtime_ns
        send("\r")  # the only saved session
        time.sleep(0.15)
        assert session_path.stat().st_mtime_ns == modified, "resume rewrote the transcript"
        send("after resume\r")
        wait(lambda: has_text("Resumed."), "resume response")
        send("/copy agent\r")
        wait(lambda: b"\x1b]52;c;" in screen, "clipboard OSC52")
        send("cancel test\r")
        wait(lambda: (project / "slow").exists(), "slow tool started")
        send("\x1b")
        wait(lambda: has_text("cancelled; process group killed"), "cancellation result")
        child = int((project / "child.pid").read_text())
        time.sleep(0.2)
        if Path(f"/proc/{child}/stat").exists():
            assert Path(f"/proc/{child}/stat").read_text().split()[2] == "Z"
        assert len(requests) == 5, "cancelled loop continued"
        send("incomplete\r")
        wait(lambda: b"without response.completed" in screen, "incomplete stream error")
        assert not (project / "must-not-run").exists()
        send("\r")
        wait(lambda: has_text("Retried."), "explicit retry")
        offset = len(screen)
        send("inspect @mnrs")
        wait(lambda: b"src/main.rs" in screen[offset:], "fuzzy file search")
        assert b"ignored/main.rs" not in screen[offset:], "file search ignored .gitignore"
        send("\t\r")
        wait(lambda: has_text("File attached."), "file injection")
        (project / "src/main.rs").write_text("CHANGED SOURCE")
        offset = len(screen)
        send("/rev check @space")
        wait(lambda: b"notes space.txt" in screen[offset:], "file with spaces")
        send("\t\r")
        wait(lambda: has_text("Skill and spaced file attached."), "skill invocation with file")
        # Failed expansion must neither submit nor throw away the editable draft.
        offset = len(screen)
        send("inspect @no-such-file-xyz \r")
        wait(lambda: b"No such file" in screen[offset:], "missing file error")
        assert len(requests) == 9
        assert b"inspect @no-such-file-xyz" in screen[offset:]
        send("\x15")  # Ctrl+U clears the retained draft.
        offset = len(screen)
        send("/new\r/model unused\r")
        wait(lambda: b"\x1b[30;104H\x1b[38;5;8munused " in screen[offset:], "unused session configuration")
        # Narrow resize and Unicode input shouldn't corrupt the editor or panic.
        offset = len(screen)
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 8, 24, 0, 0))
        os.kill(proc.pid, 28)  # SIGWINCH
        wait(lambda: b"\x1b[8;18H" in screen[offset:], "resized status line")
        send("無µ\x7f\x7f")
        send("/q\r")
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            raise AssertionError(f"mu did not quit: {screen[-6000:]!r}") from None
        assert proc.returncode == 0
        assert len(saved()) == 1, "leaving an unused session persisted it"
        print(f"ok: streaming, bash, images, steering, cache warning, branches, lazy sessions, resume without rewriting, clipboard, cancellation, incomplete SSE, file/skill injection, completion, resize; {rss}")
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        os.close(master)
        server.shutdown()
