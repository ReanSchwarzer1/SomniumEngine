#!/usr/bin/env python3
"""Flatten one Poly Haven model manifest into a plain record file.

    polyhaven_files.py <manifest.json> <resolution> <asset> <out>

Writes three lines per file — relative path, URL, MD5 — with no delimiters and
no separators to get wrong. `tools/fetch_foliage.sh` reads it three lines at a
time.

Why a file rather than a pipe of tab-separated lines: Python translates
newlines on Windows, so a piped line ends CRLF, the shell's `read` leaves the
CR on the last field, and the last field is the MD5. Every hash then compares
unequal — every already-correct file is re-downloaded and every fresh download
is deleted as corrupt. That destroyed four committed assets once. Opening the
output in binary is the fix that cannot be undone by a careless edit to an
escape sequence.

Why follow the `include` map rather than build URLs: a model's textures do not
sit beside its glTF, and the `.bin` is served from the 8k variant whatever
resolution was asked for.
"""

import json
import sys


def main(argv: list[str]) -> int:
    if len(argv) != 5:
        print(__doc__, file=sys.stderr)
        return 2
    manifest_path, resolution, asset, out_path = argv[1:]

    with open(manifest_path, encoding="utf-8") as handle:
        manifest = json.load(handle)

    try:
        gltf = manifest["gltf"][resolution]["gltf"]
    except (KeyError, TypeError):
        print(f"{asset}: no glTF at {resolution}", file=sys.stderr)
        return 1

    records = [(f"{asset}_{resolution}.gltf", gltf["url"], gltf.get("md5", ""))]
    records += [
        (name, entry["url"], entry.get("md5", ""))
        for name, entry in gltf.get("include", {}).items()
    ]

    # Binary, LF, explicitly. See the module docstring.
    with open(out_path, "wb") as handle:
        for relative, url, digest in records:
            for field in (relative, url, digest):
                handle.write(field.encode("utf-8"))
                handle.write(b"\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
