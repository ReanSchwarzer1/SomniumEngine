"""Current-user local named-pipe client. No network listener or shell access."""
from __future__ import annotations

import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import re
import time
import uuid

MAX_REQUEST = 1024 * 1024
MAX_RESPONSE = 4 * 1024 * 1024
METHODS = frozenset(f"authoring.{name}" for name in
                    ("discover", "query", "plan", "commit", "execute", "history", "jobs"))


class BridgeError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message

    def as_dict(self):
        return {"code": self.code, "message": self.message}


def read_descriptor(path: Path, expected_project: str | None = None) -> dict:
    try:
        with path.open("rb") as stream:
            raw = stream.read(16385)
        if len(raw) > 16384:
            raise ValueError("oversized connection descriptor")
        data = json.loads(raw)
        prefix = "\\\\.\\pipe\\somnium-authoring-"
        if not isinstance(data, dict) or data.get("version") != 1:
            raise ValueError("unsupported connection descriptor")
        if not isinstance(data.get("pipe_name"), str) or not data["pipe_name"].startswith(prefix):
            raise ValueError("connection is not a local Somnium pipe")
        if not re.fullmatch(r"[a-f0-9]{32}", data["pipe_name"][len(prefix):]):
            raise ValueError("invalid named pipe session")
        if not re.fullmatch(r"[a-f0-9]{32}", data.get("session_id", "")):
            raise ValueError("invalid editor session")
        if not re.fullmatch(r"[a-f0-9]{64}", data.get("token", "")):
            raise ValueError("invalid connection credential")
        if not isinstance(data.get("project_id"), str) or not 1 <= len(data["project_id"]) <= 256:
            raise ValueError("invalid project identity")
        for key, maximum in (("max_request_bytes", MAX_REQUEST), ("max_response_bytes", MAX_RESPONSE)):
            if not isinstance(data.get(key, maximum), int) or not 1024 <= data.get(key, maximum) <= maximum:
                raise ValueError("invalid connection limit")
        if expected_project is not None and data["project_id"] != expected_project:
            raise BridgeError("wrong_project", "Connection descriptor belongs to another project")
        return data
    except BridgeError:
        raise
    except (OSError, ValueError, TypeError, KeyError) as error:
        raise BridgeError("editor_unavailable", f"Cannot read a valid editor connection ({type(error).__name__}); start the intended project") from None


class WindowsPipe:
    """Nonblocking byte-pipe I/O with deadlines and partial-frame handling."""
    def __init__(self, name: str, deadline: float):
        if os.name != "nt":
            raise BridgeError("unsupported_platform", "Somnium local IPC currently requires Windows")
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                        ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
        self.api.CreateFileW.restype = wintypes.HANDLE
        self.api.WaitNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD]
        self.api.WaitNamedPipeW.restype = wintypes.BOOL
        self.api.SetNamedPipeHandleState.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p, ctypes.c_void_p]
        self.api.SetNamedPipeHandleState.restype = wintypes.BOOL
        for fn in (self.api.ReadFile, self.api.WriteFile):
            fn.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
            fn.restype = wintypes.BOOL
        self.api.CloseHandle.argtypes = [wintypes.HANDLE]
        self.api.CloseHandle.restype = wintypes.BOOL
        self.handle = None
        self.received = bytearray()
        self.deadline = deadline
        while time.monotonic() < deadline:
            handle = self.api.CreateFileW(name, 0xC0000000, 0, None, 3, 0x00110000, None)
            if handle != ctypes.c_void_p(-1).value:
                self.handle = handle
                mode = wintypes.DWORD(1)  # PIPE_NOWAIT, byte-read mode
                if not self.api.SetNamedPipeHandleState(handle, ctypes.byref(mode), None, None):
                    self.close()
                    raise BridgeError("transport_error", "Cannot configure local pipe")
                return
            code = ctypes.get_last_error()
            if code not in (2, 231, 121, 233):
                raise BridgeError("editor_unavailable", f"Cannot open editor pipe (Windows error {code})")
            self.api.WaitNamedPipeW(name, 25)
            time.sleep(0.005)
        raise BridgeError("editor_unavailable", "Editor pipe is unavailable or busy; no request was sent")

    def close(self):
        if self.handle is not None:
            self.api.CloseHandle(self.handle)
            self.handle = None

    def write(self, data: bytes, chunk_size: int = 65536):
        offset = 0
        while offset < len(data):
            if time.monotonic() >= self.deadline:
                raise BridgeError("outcome_unknown", "Timed out sending request; query state before retrying a mutation")
            chunk = data[offset:offset + chunk_size]
            buffer = ctypes.create_string_buffer(chunk)
            count = wintypes.DWORD()
            if not self.api.WriteFile(self.handle, buffer, len(chunk), ctypes.byref(count), None):
                code = ctypes.get_last_error()
                raise BridgeError("outcome_unknown", f"Editor disconnected during request (Windows error {code}); query state before retrying")
            offset += count.value
            if count.value == 0:
                time.sleep(0.005)

    def readline(self, limit=MAX_RESPONSE) -> bytes:
        buffer = ctypes.create_string_buffer(65536)
        while time.monotonic() < self.deadline:
            if b"\n" in self.received:
                line, _, rest = self.received.partition(b"\n")
                if len(line) > limit:
                    raise BridgeError("response_too_large", "Editor response exceeded its declared limit")
                self.received = bytearray(rest)
                return bytes(line)
            count = wintypes.DWORD()
            ok = self.api.ReadFile(self.handle, buffer, len(buffer), ctypes.byref(count), None)
            if ok and count.value:
                self.received.extend(buffer.raw[:count.value])
                if len(self.received) > limit + 1:
                    raise BridgeError("response_too_large", "Editor response exceeded its declared limit")
            elif not ok and ctypes.get_last_error() not in (232, 536):
                raise BridgeError("outcome_unknown", "Editor disconnected before replying; query state before retrying a mutation")
            else:
                time.sleep(0.005)
        raise BridgeError("outcome_unknown", "Editor did not reply within the timeout; query state before retrying a mutation")

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


class EditorClient:
    def __init__(self, descriptor: Path, project_id: str | None = None, timeout: float = 10.0):
        self.path = descriptor.resolve()
        self.project_id = project_id
        self.timeout = max(0.1, min(timeout, 30.0))

    def call(self, method: str, params: dict, *, chunk_size: int = 65536) -> dict:
        if method not in METHODS:
            raise BridgeError("method_not_found", "Only declared authoring methods may be called")
        if not isinstance(params, dict):
            raise BridgeError("invalid_params", "Authoring parameters must be an object")
        descriptor = read_descriptor(self.path, self.project_id)
        request_id = uuid.uuid4().hex
        request = {"id": request_id, "project_id": descriptor["project_id"],
                   "session_id": descriptor["session_id"], "token": descriptor["token"],
                   "method": method, "params": params}
        try:
            wire = json.dumps(request, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8") + b"\n"
        except (ValueError, TypeError):
            raise BridgeError("invalid_params", "Parameters must be finite JSON values") from None
        if len(wire) > min(MAX_REQUEST, descriptor.get("max_request_bytes", MAX_REQUEST)):
            raise BridgeError("payload_too_large", "Request exceeds the connection limit")
        with WindowsPipe(descriptor["pipe_name"], time.monotonic() + self.timeout) as pipe:
            pipe.write(wire, chunk_size)
            raw = pipe.readline(min(MAX_RESPONSE, descriptor.get("max_response_bytes", MAX_RESPONSE)))
        try:
            response = json.loads(raw)
        except (ValueError, UnicodeError):
            raise BridgeError("invalid_response", "Editor returned malformed JSON") from None
        if not isinstance(response, dict) or response.get("id") != request_id:
            raise BridgeError("invalid_response", "Editor response did not match the request")
        if "error" in response:
            error = response["error"]
            raise BridgeError(error.get("code", "transport_error"), error.get("message", "Editor rejected request"))
        if "result" not in response:
            raise BridgeError("invalid_response", "Editor response has no result")
        return response["result"]
