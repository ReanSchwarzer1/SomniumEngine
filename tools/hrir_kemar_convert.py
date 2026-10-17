"""Pack the MIT KEMAR compact HRTF set into crates/somnium_audio/assets/hrir_mit_kemar.bin (stdlib only).

    python -I tools/hrir_kemar_convert.py EXTRACTED_COMPACT_DIR

Source: https://sound.media.mit.edu/resources/KEMAR/compact.zip (Bill Gardner and Keith Martin, MIT Media Lab, 1994):
"provided free with no restrictions on use, provided the authors are cited when the data is used in any research or
commercial application". 368 positions, elevations -40 to 90, azimuths 0 to 180 (a source on the right; the left side
is the mirror), 128-point responses at 44.1 kHz, left and right interleaved.

Layout (little endian): b"HRIR", u16 version 1, u16 taps, u32 sample rate, u16 count, then per position
i16 elevation, i16 azimuth, taps x (i16 left, i16 right). Sorted by elevation, then azimuth.
"""
import re
import struct
import sys
import wave
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "crates/somnium_audio/assets/hrir_mit_kemar.bin"


def main():
    root = Path(sys.argv[1])
    entries = []
    for path in root.rglob("H*a.wav"):
        m = re.fullmatch(r"H(-?\d+)e(\d+)a\.wav", path.name)
        if not m:
            continue
        with wave.open(str(path), "rb") as w:
            assert w.getnchannels() == 2 and w.getsampwidth() == 2 and w.getframerate() == 44100, path
            taps = w.getnframes()
            frames = w.readframes(taps)
        entries.append((int(m.group(1)), int(m.group(2)), taps, frames))
    entries.sort(key=lambda e: (e[0], e[1]))
    taps = entries[0][2]
    assert all(e[2] == taps for e in entries)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with OUT.open("wb") as f:
        f.write(b"HRIR" + struct.pack("<HHIH", 1, taps, 44100, len(entries)))
        for elevation, azimuth, _, frames in entries:
            f.write(struct.pack("<hh", elevation, azimuth) + frames)
    print(f"{len(entries)} positions, {taps} taps -> {OUT} ({OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
