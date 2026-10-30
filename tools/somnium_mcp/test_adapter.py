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

    # Phase ALYX-M.
    def ready(self):
        server = Server(EditorClient(Path("absent.json")))
        server.initialized = True
        return server

    def test_listed_names_suit_every_client_and_old_names_still_work(self):
        import re
        reply = self.ready().handle({"jsonrpc":"2.0","id":1,"method":"tools/list"})
        for tool in reply["result"]["tools"]:
            self.assertRegex(tool["name"], r"^[A-Za-z0-9_-]{1,64}$")
            self.assertIn("readOnlyHint", tool["annotations"])
        # A dotted name from before 0.2 reaches the editor (which is absent here), not "unknown tool".
        old = self.ready().handle({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"authoring.discover","arguments":{}}})
        self.assertTrue(old["result"]["isError"])
        self.assertEqual(old["result"]["structuredContent"]["error"]["code"], "editor_unavailable")
        composed = self.ready().handle({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"somnium_status","arguments":{}}})
        self.assertTrue(composed["result"]["isError"])

    def test_resources_and_prompts_are_listed_and_checked(self):
        server = self.ready()
        resources = server.handle({"jsonrpc":"2.0","id":1,"method":"resources/list"})["result"]["resources"]
        self.assertIn("somnium://log/problems", {r["uri"] for r in resources})
        missing = server.handle({"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"somnium://nope"}})
        self.assertEqual(missing["error"]["code"], -32002)
        unreachable = server.handle({"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"somnium://status"}})
        self.assertIn("editor_unavailable", unreachable["result"]["contents"][0]["text"])
        prompts = server.handle({"jsonrpc":"2.0","id":4,"method":"prompts/list"})["result"]["prompts"]
        self.assertIn("block_out_space", {p["name"] for p in prompts})
        needs = server.handle({"jsonrpc":"2.0","id":5,"method":"prompts/get","params":{"name":"block_out_space","arguments":{}}})
        self.assertEqual(needs["error"]["code"], -32602)
        prompt = server.handle({"jsonrpc":"2.0","id":6,"method":"prompts/get","params":{"name":"block_out_space","arguments":{"brief":"a cellar"}}})
        self.assertIn("a cellar", prompt["result"]["messages"][0]["content"]["text"])

    def test_run_command_needs_an_id_before_the_editor(self):
        reply = self.ready().handle({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"somnium_run_command","arguments":{}}})
        self.assertEqual(reply["result"]["structuredContent"]["error"]["code"], "invalid_arguments")


if __name__ == "__main__":
    unittest.main()
