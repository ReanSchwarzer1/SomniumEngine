"""Minimal PNG read/write, standard library only.

GHOSTFENCE compares images the engine wrote and writes a diff beside them. It
must not acquire a third-party image dependency to do that: the gate has to run
in CI before anything else is installed, and a gate that can be skipped because
its dependency is missing is not a gate.

Scope is deliberately narrow — 8-bit greyscale/RGB/RGBA, non-interlaced, which
is everything `crates/somnium_renderer/src/capture.rs` produces through the
`image` crate. Anything else raises rather than guessing.
"""

from __future__ import annotations

import struct
import zlib
from dataclasses import dataclass
from pathlib import Path

PNG_MAGIC = b"\x89PNG\r\n\x1a\n"

# Channel count per PNG colour type. 3 (palette) and the 16-bit depths are
# absent on purpose: the engine never writes them, and silently mishandling one
# would make a golden-image pass meaningless.
CHANNELS = {0: 1, 2: 3, 4: 2, 6: 4}


class PngError(RuntimeError):
    """The file is not a PNG this module is willing to interpret."""


@dataclass(frozen=True)
class Image:
    width: int
    height: int
    channels: int
    #: Row-major, `height * width * channels` bytes.
    pixels: bytes

    def rgb(self, x: int, y: int) -> tuple[int, int, int]:
        i = (y * self.width + x) * self.channels
        if self.channels == 1:
            v = self.pixels[i]
            return (v, v, v)
        if self.channels == 2:
            v = self.pixels[i]
            return (v, v, v)
        return (self.pixels[i], self.pixels[i + 1], self.pixels[i + 2])


def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def _unfilter(raw: bytes, width: int, height: int, channels: int) -> bytes:
    stride = width * channels
    out = bytearray(stride * height)
    previous = bytearray(stride)
    pos = 0
    for row in range(height):
        filter_type = raw[pos]
        pos += 1
        line = bytearray(raw[pos : pos + stride])
        pos += stride
        if filter_type == 0:
            pass
        elif filter_type == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif filter_type == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif filter_type == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif filter_type == 4:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                upper_left = previous[i - channels] if i >= channels else 0
                line[i] = (line[i] + _paeth(left, previous[i], upper_left)) & 0xFF
        else:
            raise PngError(f"unknown row filter {filter_type} on row {row}")
        out[row * stride : (row + 1) * stride] = line
        previous = line
    return bytes(out)


def read(path: Path) -> Image:
    data = path.read_bytes()
    if not data.startswith(PNG_MAGIC):
        raise PngError(f"{path} is not a PNG")
    pos = len(PNG_MAGIC)
    header: tuple[int, int, int, int, int, int, int] | None = None
    idat = bytearray()
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        pos += 12 + length  # length + type + body + crc
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
    if header is None:
        raise PngError(f"{path} has no IHDR")
    width, height, depth, colour, compression, filter_method, interlace = header
    if depth != 8:
        raise PngError(f"{path}: only 8-bit images are supported, got {depth}-bit")
    if interlace != 0:
        raise PngError(f"{path}: interlaced PNGs are not supported")
    if compression != 0 or filter_method != 0:
        raise PngError(f"{path}: unexpected compression/filter method")
    if colour not in CHANNELS:
        raise PngError(f"{path}: unsupported colour type {colour}")
    channels = CHANNELS[colour]
    pixels = _unfilter(zlib.decompress(bytes(idat)), width, height, channels)
    return Image(width=width, height=height, channels=channels, pixels=pixels)


def write_rgb(path: Path, width: int, height: int, pixels: bytes) -> None:
    """Write an 8-bit RGB PNG. Used only for diff images."""
    stride = width * 3
    raw = bytearray()
    for row in range(height):
        raw.append(0)  # filter: none — diffs are small and clarity beats size
        raw += pixels[row * stride : (row + 1) * stride]

    def chunk(kind: bytes, body: bytes) -> bytes:
        return (
            struct.pack(">I", len(body))
            + kind
            + body
            + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
        )

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(
        PNG_MAGIC
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 6))
        + chunk(b"IEND", b"")
    )
