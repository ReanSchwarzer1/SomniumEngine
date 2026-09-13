"""Create a compact, credential-free coverage inventory from real editor evidence."""
import argparse,json
from pathlib import Path

def generate(source,output):
    inventory=json.loads((source/'capability-inventory.json').read_text(encoding='utf-8'))
    receipts=json.loads((source/'semantic-acceptance.json').read_text(encoding='utf-8'))
    lines=['# Editor capability evidence','','Generated from a running Hello Engine session by the main and workspace acceptance probes. These are native semantic edits, not an emulated editor. Full machine receipts remain in the ignored evidence directory.','','## Live cases','','| Family | Result | Evidence |','|---|---|---|']
    for name,case in receipts['cases'].items():
        details=', '.join(f'{key}: {value}' for key,value in case.items() if key!='pass') or 'query succeeded'
        lines.append(f'| {name.replace("_"," ")} | {"Pass" if case.get("pass") else "Fail"} | {details} |')
    lines+=['','## Declared semantic operations','','The following operations come from the live discovery response. A passing family case does not mean every individual operation or screen gesture was replayed.','','| Owner | Operations |','|---|---|']
    for name,data in inventory.get('coverage',{}).items():
        if isinstance(data,dict):
            values=data.get('operations',data.get('actions',data.get('tools',data.get('queries'))))
            if values:lines.append(f'| {name.replace("_"," ")} | {", ".join(f"`{value}`" for value in values)} |')
        elif isinstance(data,str):lines.append(f'| {name.replace("_"," ")} | {data} |')
    lines+=['','Scene component schemas, commands, source validators and runtime diagnostics are discovered from the current editor. Preferences retain their own persistence semantics. Native picking tests transformed mesh/proxy bounds; positive ray distance and clear-capsule results are recorded above. Clearance considers registered physics colliders. Renderer preview knobs and brush settings restore their previous values; source files are separate from scene undo.','','See [designer controls and contracts](../../docs/editor/automation.md). Rerun after changing the relevant owner or adapter:','','```powershell','python -B tools/somnium_mcp/editor_acceptance.py --connection target/hello-editor/runtime/authoring-connection.json --output target/hello-editor/semantic-acceptance.json','python -B tools/somnium_mcp/editor_gap_acceptance.py --connection target/hello-editor/runtime/authoring-connection.json --output target/hello-editor/semantic-acceptance.json','python -B tools/somnium_mcp/generate_capabilities.py --source target/hello-editor --output tools/somnium_mcp/CAPABILITIES.md','```','']
    output.write_text('\n'.join(lines),encoding='utf-8')
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--source',required=True,type=Path);p.add_argument('--output',required=True,type=Path);a=p.parse_args();generate(a.source,a.output)
