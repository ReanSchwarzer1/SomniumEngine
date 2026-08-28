"""Tests for the GHOSTFENCE golden-image comparator.

The comparator is the mechanism every visual claim in Phase MORROWIND is about
to rest on, so it gets tested rather than trusted. Each case below is a way the
comparator could be wrong in a direction that matters:

- a comparator that always passes is the failure mode this whole row exists to
  prevent, so `a_shifted_pixel_fails` is the load-bearing test;
- a comparator that fails on encoder noise gets switched off within a week, so
  `tiny_noise_passes` matters as much;
- and the two thresholds catch opposite shapes of regression, so each gets a
  case where the *other* one would sleep through it.

    python -m pytest tools/ghostfence/test_ghostfence.py
    python tools/ghostfence/test_ghostfence.py          # no pytest needed
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from ghostfence import golden, png  # noqa: E402


def solid(width: int, height: int, colour: tuple[int, int, int]) -> bytes:
    return bytes(colour) * (width * height)


def write(path: Path, width: int, height: int, pixels: bytes) -> Path:
    png.write_rgb(path, width, height, pixels)
    return path


def test_round_trip_survives_write_then_read() -> None:
    """Every other test is meaningless if the codec loses data."""
    with tempfile.TemporaryDirectory() as tmp:
        pixels = bytes(range(256)) * 12  # 4096 bytes, not a multiple of 3 by luck
        pixels = pixels[: 32 * 32 * 3]
        path = write(Path(tmp) / "rt.png", 32, 32, pixels)
        image = png.read(path)
        assert (image.width, image.height, image.channels) == (32, 32, 3)
        assert image.pixels == pixels


def test_identical_images_pass_even_at_exact_threshold() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        pixels = solid(16, 16, (10, 200, 30))
        a = write(Path(tmp) / "a.png", 16, 16, pixels)
        b = write(Path(tmp) / "b.png", 16, 16, pixels)
        result = golden.compare(a, b, golden.Threshold.exact())
        assert result.passed, result.reason
        assert result.max_channel_delta == 0


def test_a_shifted_pixel_fails() -> None:
    """One pixel moving a lot. A mean-only comparator sleeps through this.

    This is the widget-drifted-a-pixel case and the glyph-lost-its-snap case,
    and it is the reason `max_channel` exists beside the fraction budget.
    """
    with tempfile.TemporaryDirectory() as tmp:
        reference = solid(16, 16, (10, 10, 10))
        candidate = bytearray(reference)
        candidate[3 * 100 : 3 * 100 + 3] = b"\xff\xff\xff"
        a = write(Path(tmp) / "a.png", 16, 16, reference)
        b = write(Path(tmp) / "b.png", 16, 16, bytes(candidate))
        diff = Path(tmp) / "diff.png"
        result = golden.compare(a, b, diff_path=diff)
        assert not result.passed
        assert result.max_channel_delta == 245
        assert diff.exists(), "a failure must write the diff, not just a number"
        assert png.read(diff).width == 16


def test_tiny_noise_passes() -> None:
    """Encoder noise below tolerance must not fail, or the gate gets disabled."""
    with tempfile.TemporaryDirectory() as tmp:
        reference = solid(16, 16, (100, 100, 100))
        candidate = bytes((v + (1 if i % 2 else 0)) for i, v in enumerate(reference))
        a = write(Path(tmp) / "a.png", 16, 16, reference)
        b = write(Path(tmp) / "b.png", 16, 16, candidate)
        result = golden.compare(a, b)
        assert result.passed, result.reason


def test_broad_small_drift_fails_on_the_fraction_budget() -> None:
    """A large area moving a little — a tone-map or gamma shift.

    Below `max_channel`, so only the fraction budget can catch it. If this test
    passes for the wrong reason, check that `max_channel` is not doing the work.
    """
    with tempfile.TemporaryDirectory() as tmp:
        reference = solid(32, 32, (100, 100, 100))
        candidate = solid(32, 32, (108, 108, 108))
        a = write(Path(tmp) / "a.png", 32, 32, reference)
        b = write(Path(tmp) / "b.png", 32, 32, candidate)
        threshold = golden.Threshold(channel_tolerance=2, failing_fraction=0.001, max_channel=24)
        result = golden.compare(a, b, threshold)
        assert not result.passed
        assert result.max_channel_delta == 8 <= threshold.max_channel
        assert result.failing_fraction == 1.0


def test_size_change_is_a_failure_not_a_crash() -> None:
    """Changing capture resolution changes the evidence. Re-approve deliberately."""
    with tempfile.TemporaryDirectory() as tmp:
        a = write(Path(tmp) / "a.png", 16, 16, solid(16, 16, (0, 0, 0)))
        b = write(Path(tmp) / "b.png", 8, 8, solid(8, 8, (0, 0, 0)))
        result = golden.compare(a, b)
        assert not result.passed
        assert "size changed" in result.reason


# ── MORROWIND-E2b: regions ──────────────────────────────────────────────────


def test_a_region_ignores_drift_outside_it() -> None:
    """The whole point: the viewport moves, the chrome must not."""
    with tempfile.TemporaryDirectory() as tmp:
        pixels = bytearray(solid(32, 32, (20, 20, 20)))
        a = write(Path(tmp) / "a.png", 32, 32, bytes(pixels))
        # Repaint the right half white — a stochastic viewport, in miniature.
        for y in range(32):
            for x in range(16, 32):
                i = (y * 32 + x) * 3
                pixels[i] = pixels[i + 1] = pixels[i + 2] = 255
        b = write(Path(tmp) / "b.png", 32, 32, bytes(pixels))

        whole = golden.compare(a, b)
        assert not whole.passed, "a half-white image must fail a whole-image compare"

        left = golden.compare(a, b, region=golden.Region(0, 0, 16, 32))
        assert left.passed, f"the untouched left half failed: {left.reason}"
        assert left.total_pixels == 16 * 32, "the region did not shrink the pixel count"


def test_a_region_still_catches_drift_inside_it() -> None:
    """A gate that cannot fail is not a gate."""
    with tempfile.TemporaryDirectory() as tmp:
        pixels = bytearray(solid(32, 32, (20, 20, 20)))
        a = write(Path(tmp) / "a.png", 32, 32, bytes(pixels))
        # One glyph's worth of drift, inside the region under test.
        for y in range(4, 8):
            for x in range(4, 8):
                i = (y * 32 + x) * 3
                pixels[i] = pixels[i + 1] = pixels[i + 2] = 200
        b = write(Path(tmp) / "b.png", 32, 32, bytes(pixels))

        inside = golden.compare(a, b, region=golden.Region(0, 0, 16, 16))
        assert not inside.passed, "16 changed pixels in a 256-pixel region must fail"
        assert inside.max_channel_delta == 180, inside.max_channel_delta

        elsewhere = golden.compare(a, b, region=golden.Region(16, 16, 16, 16))
        assert elsewhere.passed, "a region away from the change must not fail"


def test_a_region_larger_than_the_capture_clamps() -> None:
    """A mis-typed region is a bad reference, not a crash."""
    with tempfile.TemporaryDirectory() as tmp:
        a = write(Path(tmp) / "a.png", 16, 16, solid(16, 16, (9, 9, 9)))
        b = write(Path(tmp) / "b.png", 16, 16, solid(16, 16, (9, 9, 9)))
        result = golden.compare(a, b, region=golden.Region(8, 8, 999, 999))
        assert result.passed
        assert result.total_pixels == 8 * 8, result.total_pixels


def main() -> int:
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    failures = 0
    for test in tests:
        try:
            test()
        except AssertionError as error:
            failures += 1
            print(f"FAIL {test.__name__}: {error}")
        else:
            print(f"ok   {test.__name__}")
    print(f"\n{len(tests) - failures} passed, {failures} failed")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
