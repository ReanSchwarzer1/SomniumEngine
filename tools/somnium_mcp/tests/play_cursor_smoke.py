"""Windows integration check against an existing editor. No scene writes.

Usage: python play_cursor_smoke.py DESCRIPTOR PID RECEIPT [--isolated-desktop]
Exercises real window key/click/focus events and reads the OS cursor/clip state.
"""
import ctypes as c
from ctypes import wintypes as w
import json
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pipe_client import EditorClient

u = c.WinDLL('user32', use_last_error=True)
u.GetForegroundWindow.restype = w.HWND
u.SetForegroundWindow.argtypes = [w.HWND]
u.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
u.GetClientRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
u.ClientToScreen.argtypes = [w.HWND, c.POINTER(w.POINT)]
u.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
u.IsWindowVisible.argtypes = [w.HWND]
u.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
u.GetClassNameW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
u.ShowWindow.argtypes = [w.HWND, c.c_int]
callback = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)

class Cursor(c.Structure):
    _fields_ = [('cbSize', w.DWORD), ('flags', w.DWORD), ('cursor', w.HANDLE), ('point', w.POINT)]

def main():
    descriptor, pid, output = Path(sys.argv[1]), int(sys.argv[2]), Path(sys.argv[3])
    isolated = '--isolated-desktop' in sys.argv[4:]
    client = EditorClient(descriptor, timeout=20)
    def call(method, params=None):
        result = client.call('authoring.' + method, params or {})
        assert result.get('ok') is not False, result
        return result
    def command(name):
        call('execute', {'action': 'command', 'id': name})
    def diag():
        return call('query', {'kind': 'diagnostics'})
    def wait(predicate):
        end = time.monotonic() + 12
        while time.monotonic() < end:
            result = predicate()
            if result:
                return result
            time.sleep(.08)
        raise AssertionError(('timeout', diag()['play_cursor'], 'foreground', u.GetForegroundWindow()))
    windows = []
    @callback
    def visit(hwnd, _):
        owner = w.DWORD()
        u.GetWindowThreadProcessId(hwnd, c.byref(owner))
        if owner.value == pid:
            rect = w.RECT()
            u.GetClientRect(hwnd, c.byref(rect))
            title, kind = c.create_unicode_buffer(256), c.create_unicode_buffer(256)
            u.GetWindowTextW(hwnd, title, 256)
            u.GetClassNameW(hwnd, kind, 256)
            print('Owned window', hwnd, repr(title.value), repr(kind.value), rect.right, rect.bottom, bool(u.IsWindowVisible(hwnd)), flush=True)
            if title.value == 'Somnium Engine':
                windows.append(((rect.right - rect.left) * (rect.bottom - rect.top), hwnd))
        return True
    u.EnumWindows(visit, 0)
    assert windows, 'No editor window on this desktop for the supplied process'
    hwnd = max(windows)[1]
    previous = u.GetForegroundWindow()
    original = w.POINT()
    u.GetCursorPos(c.byref(original))
    def centre():
        rect = w.RECT()
        u.GetClientRect(hwnd, c.byref(rect))
        x, y = rect.right // 2, rect.bottom // 2
        point = w.POINT(x, y)
        u.ClientToScreen(hwnd, c.byref(point))
        u.SetCursorPos(point.x, point.y)
        u.PostMessageW(hwnd, 0x200, 0, x | (y << 16))
        return x, y
    def escape():
        u.PostMessageW(hwnd, 0x100, 0x1B, 1 | (1 << 16))
        u.PostMessageW(hwnd, 0x101, 0x1B, 1 | (1 << 16) | (3 << 30))
    def click():
        x, y = centre()
        u.PostMessageW(hwnd, 0x201, 1, x | (y << 16))
        u.PostMessageW(hwnd, 0x202, 0, x | (y << 16))
    def cursor():
        state = Cursor(cbSize=c.sizeof(Cursor))
        assert u.GetCursorInfo(c.byref(state))
        rect = w.RECT()
        assert u.GetClipCursor(c.byref(rect))
        return {'visible': bool(state.flags & 1), 'clip': [rect.left, rect.top, rect.right, rect.bottom]}
    checks = []
    def check(label, captured, immersive=False):
        state = wait(lambda: (d := diag()) and d['play_cursor']['captured'] == captured
                     and d['play_cursor']['immersive'] == immersive and d)
        os_state = {'visibility_verified': False, 'reason': 'isolated test desktop'} if isolated else wait(
            lambda: (s := cursor()) and s['visible'] != captured and s)
        if captured and not isolated:
            rect, origin = w.RECT(), w.POINT(0, 0)
            u.GetClientRect(hwnd, c.byref(rect))
            u.ClientToScreen(hwnd, c.byref(origin))
            left, top, right, bottom = os_state['clip']
            assert origin.x <= left < right <= origin.x + rect.right, os_state
            assert origin.y <= top < bottom <= origin.y + rect.bottom, os_state
        checks.append({'case': label, 'transport': state['simulation'], 'editor': state['play_cursor'], 'os': os_state})
        print('PASS', label, flush=True)
    try:
        command('editor.simulation.stop')
        u.ShowWindow(hwnd, 9)
        u.SetForegroundWindow(hwnd)
        if not isolated:
            wait(lambda: u.GetForegroundWindow() == hwnd)
        centre()
        command('editor.simulation.play')
        check('Play hides and confines cursor', True)
        escape()
        check('physical Esc releases cursor', False)
        before = call('query', {'kind': 'game'})['game']
        if not isolated:
            u.mouse_event(1, 12, 4, 0, 0)
        time.sleep(.15)
        after = call('query', {'kind': 'game'})['game']
        if before.get('player') and after.get('player'):
            assert before['player']['pose']['view_yaw'] == after['player']['pose']['view_yaw']
        click()
        check('viewport click recaptures', True)
        command('editor.simulation.pause')
        check('Pause releases cursor', False)
        command('editor.simulation.play')
        check('Resume captures cursor', True)
        # Deliver the native focus transition without resizing/minimising the
        # GPU surface; this test owns input routing, not swapchain recreation.
        u.PostMessageW(hwnd, 0x8, 0, 0)
        wait(lambda: not diag()['play_cursor']['captured'])
        u.PostMessageW(hwnd, 0x7, 0, 0)
        u.SetForegroundWindow(hwnd)
        if not isolated:
            wait(lambda: u.GetForegroundWindow() == hwnd)
        centre()
        check('focus loss releases without automatic recapture', False)
        click()
        check('click after focus loss recaptures', True)
        command('editor.simulation.stop')
        check('Stop releases cursor', False)
        authored = call('query')['entities']
        command('editor.viewport.immersive')
        check('fullscreen Play hides and confines cursor', True, True)
        escape()
        check('fullscreen physical Esc restores editor and cursor', False)
        command('editor.simulation.stop')
        restored = call('query')['entities']
        assert authored == restored, 'Fullscreen Stop did not restore the authored scene'
        checks.append({'case': 'fullscreen Stop restores authored scene exactly'})
    finally:
        command('editor.simulation.stop')
        u.SetCursorPos(original.x, original.y)
        if previous != hwnd and not isolated:
            u.SetForegroundWindow(previous)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps({'checks': checks}, indent=2), encoding='utf-8')

if __name__ == '__main__':
    main()
