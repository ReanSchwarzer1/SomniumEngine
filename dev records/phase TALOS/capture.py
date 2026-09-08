"""Reproduce one TALOS capture/timing run with matched simulation steps.

Build first: cargo build --release -p hello_engine -j 1
Example: python "dev records/phase TALOS/capture.py" balanced coastal-ground --name check
"""
import argparse
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('mode', choices=['native', 'balanced', 'performance'])
parser.add_argument('view', choices=['coastal-ground', 'coastal-overview', 'island-ground'])
parser.add_argument('--rail', default='', choices=['', 'coastal-walk', 'island-walk'])
parser.add_argument('--name', required=True, help='Unique evidence filename stem')
parser.add_argument('--resize-stress', action='store_true', help='Force DRS toward its floor to exercise repeated internal resizes')
args = parser.parse_args()
if args.resize_stress and args.mode != 'balanced':
    parser.error('--resize-stress uses the Balanced preset')
if Path(args.name).name != args.name or args.name in ('.', '..'):
    parser.error('--name must be a filename stem')
output = Path(__file__).resolve().parent / args.name
if any(Path(str(output) + suffix).exists() for suffix in ('.somtime', '.png', '.audit')):
    parser.error('Evidence already exists; use a new --name')
env = {k: v for k, v in os.environ.items() if not k.startswith('SOMNIUM_')}
env.update({
    'SOMNIUM_GRAPHICS_SCALABILITY': args.mode,
    'SOMNIUM_TIME_VIEW': args.view,
    'SOMNIUM_VIEWPORT_RES': '0',
    'SOMNIUM_MAXIMIZE': '1',
    'SOMNIUM_DYNRES': '0',
    'SOMNIUM_TIME_FIXED_STEP': '1',
    'SOMNIUM_TIME_WARMUP': '180',
    'SOMNIUM_TIME_FRAMES': '300',
    'SOMNIUM_TIME_QUIT': '1',
    'SOMNIUM_CAPTURE_FRAME': '480',
    'SOMNIUM_TIME': str(output) + '.somtime',
    'SOMNIUM_TIME_LABEL': args.name,
    'SOMNIUM_CAPTURE_DISPLAY_PNG': str(output) + '.png',
    'SOMNIUM_AUDIT_LOG': str(output) + '.audit',
})
if args.resize_stress:
    env['SOMNIUM_DYNRES'] = '1'
    env['SOMNIUM_DYNRES_TARGET_MS'] = '1'
if args.rail:
    env['SOMNIUM_DREAMS_RAIL'] = args.rail
result = subprocess.run([str(root / 'target/release/hello_engine.exe')], cwd=root,
                        env=env, timeout=240)
result.check_returncode()
for suffix in ('.somtime', '.png', '.audit'):
    path = Path(str(output) + suffix)
    if not path.is_file():
        raise RuntimeError(f'Capture did not write {path}')
if args.resize_stress:
    audit = Path(str(output) + '.audit').read_text(encoding='utf-8')
    if 'dynamic=true' not in audit or 'effective_scale=0.502500' not in audit:
        raise RuntimeError('Resize stress did not reach the Balanced × DRS floor')
print(output.name, 'captured')
