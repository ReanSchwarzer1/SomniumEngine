"""Protocol/error tests. Live editor proof is separately provided by probe.py."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from pipe_client import BridgeError, EditorClient, read_descriptor
from server import Server, TOOLS, capture_image


class AdapterTests(unittest.TestCase):
    def test_real_stdio_process_handshake_and_missing_editor_error(self):
        with tempfile.TemporaryDirectory() as directory:
            requests = [
                {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}},
                {"jsonrpc":"2.0","method":"notifications/initialized"},
                {"jsonrpc":"2.0","id":2,"method":"tools/list"},
                {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"authoring.query","arguments":{}}},
            ]
            completed = subprocess.run([sys.executable, "-B", str(Path(__file__).with_name("server.py")), "--connection", str(Path(directory)/"absent.json")],
                input="\n".join(json.dumps(request) for request in requests)+"\n", text=True, capture_output=True, timeout=10)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            replies = [json.loads(line) for line in completed.stdout.splitlines()]
            self.assertEqual(len(replies), 3)
            self.assertEqual(replies[0]["result"]["protocolVersion"], "2025-11-25")
            self.assertEqual({tool["name"] for tool in replies[1]["result"]["tools"]}, set(TOOLS))
            self.assertTrue(replies[2]["result"]["isError"])
            self.assertEqual(replies[2]["result"]["structuredContent"]["error"]["code"], "editor_unavailable")

    def test_unknown_tool_and_bad_arguments_do_not_reach_editor(self):
        server = Server(EditorClient(Path("unused")))
        server.initialized = True
        for name, arguments in (("shell", {}), ([], {}), ("authoring.commit", [])):
            reply = server.handle({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":arguments}})
            self.assertEqual(reply["error"]["code"], -32602)

    def test_initialized_notification_alone_does_not_skip_handshake(self):
        server = Server(EditorClient(Path("unused")))
        server.handle({"jsonrpc":"2.0","method":"notifications/initialized"})
        reply = server.handle({"jsonrpc":"2.0","id":1,"method":"tools/list"})
        self.assertEqual(reply["error"]["code"], -32002)

    def test_remote_pipe_descriptor_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/"connection.json"
            path.write_text(json.dumps({"version":1,"pipe_name":"\\\\remote\\pipe\\bad"}))
            with self.assertRaises(BridgeError):
                read_descriptor(path)

    def test_capture_cannot_escape_project_or_embed_non_png(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            project = root/"project"
            project.mkdir()
            png = root/"outside.png"
            png.write_bytes(b"\x89PNG\r\n\x1a\nprivate")
            self.assertIsNone(capture_image({"capture":{"path":str(png)}}, project))
            text = project/"fake.png"
            text.write_text("private content")
            self.assertIsNone(capture_image({"capture":{"path":str(text)}}, project))

    def test_native_method_allowlist_rejects_shell_before_connect(self):
        with self.assertRaises(BridgeError) as caught:
            EditorClient(Path("absent")).call("shell", {})
        self.assertEqual(caught.exception.code, "method_not_found")


if __name__ == "__main__":
    unittest.main()
