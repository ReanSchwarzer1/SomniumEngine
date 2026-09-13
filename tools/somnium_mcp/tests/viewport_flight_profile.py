"""Profile native editor flight without saving scene changes (Windows only)."""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
from pathlib import Path
import statistics
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pipe_client import EditorClient


def editor_window(pid):
    user32 = c.WinDLL('user32')
    callback = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    user32.EnumWindows.argtypes = [callback, w.LPARAM]
    user32.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
    user32.ShowWindow.argtypes = [w.HWND, c.c_int]
    user32.SetForegroundWindow.argtypes = [w.HWND]
    user32.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
    user32.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
    user32.GetClientRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
    windows = []

    @callback
    def visit(hwnd, _):
        owner = w.DWORD()
        title = c.create_unicode_buffer(256)
        user32.GetWindowThreadProcessId(hwnd, c.byref(owner))
        user32.GetWindowTextW(hwnd, title, 256)
        if owner.value == pid and title.value == 'Somnium Engine':
            windows.append(hwnd)
        return True

    user32.EnumWindows(visit, 0)
    assert len(windows) == 1, f'Expected one editor window for PID {pid}'
    return user32, windows[0]


def authoring_ms(frame):
    # app.rs cumulative frame stages: renderer/UI complete -> authoring complete.
    stages = frame['cumulative_stage_ms']
    return stages[4] - stages[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('descriptor', type=Path)
    parser.add_argument('pid', type=int)
    parser.add_argument('output', type=Path)
    parser.add_argument('--authoring-budget-ms', type=float, default=12)
    args = parser.parse_args()
    client = EditorClient(args.descriptor, timeout=15)

    def call(method, params):
        result = client.call('authoring.' + method, params)
        assert result.get('ok') is not False, result
        return result

    deadline = time.monotonic() + 90
    while True:
        try:
            call('query', {'kind': 'diagnostics'})
            break
        except Exception:
            if time.monotonic() >= deadline:
                raise
            time.sleep(.5)

    user32, hwnd = editor_window(args.pid)
    user32.ShowWindow(hwnd, 9)
    user32.SetForegroundWindow(hwnd)
    call('execute', {'action': 'command', 'id': 'editor.simulation.stop'})
    rect = w.RECT()
    user32.GetClientRect(hwnd, c.byref(rect))
    xy = (rect.right // 2) | ((rect.bottom // 2) << 16)
    user32.PostMessageW(hwnd, 0x200, 0, xy)  # WM_MOUSEMOVE

    def key(name, pressed):
        scan = {'W': 0x11, 'S': 0x1f}[name]
        flags = 1 | (scan << 16) | (0 if pressed else 3 << 30)
        user32.PostMessageW(hwnd, 0x100 if pressed else 0x101, ord(name), flags)

    def sample(label, seconds, look=False):
        rows = []
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if look:
                call('execute', {'action': 'input', 'look': [.5, 0]})
            frame = call('query', {'kind': 'diagnostics'})
            frame['wall'] = time.monotonic()
            rows.append(frame)
            time.sleep(.01)
        distinct = list({row['frame']: row for row in rows}.values())
        timings = sorted(row['frame_cpu_ms'] for row in distinct)
        motion = [sum((a - b) ** 2 for a, b in zip(v['camera'], u['camera'])) ** .5
                  for u, v in zip(distinct, distinct[1:])]
        summary = {
            'label': label, 'samples': len(distinct),
            'cpu_ms_median': statistics.median(timings),
            'cpu_ms_p95': timings[int(len(timings) * .95)],
            'cpu_ms_max': max(timings),
            'authoring_ms_max': max(map(authoring_ms, distinct)),
            'frames_over_33ms': sum(ms > 33.3 for ms in timings),
            'camera_distance': sum(motion),
        }
        print(json.dumps(summary), flush=True)
        return {'summary': summary, 'frames': distinct}

    time.sleep(2)
    phases = []
    try:
        phases.append(sample('idle', 3))
        user32.PostMessageW(hwnd, 0x204, 2, xy)  # RMB down: native fly mode
        for index in range(4):
            direction = 'W' if index % 2 == 0 else 'S'
            key(direction, True)
            phases.append(sample(f'flight-{index}', 2, True))
            key(direction, False)
    finally:
        key('W', False)
        key('S', False)
        user32.PostMessageW(hwnd, 0x205, 0, xy)  # RMB up
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(phases), encoding='utf-8')

    assert all(phase['summary']['camera_distance'] > 1 for phase in phases[1:]), 'Camera did not fly'
    peak = max(authoring_ms(row) for phase in phases for row in phase['frames'])
    assert peak < args.authoring_budget_ms, f'Authoring hitch: {peak:.2f} ms'


if __name__ == '__main__':
    main()
