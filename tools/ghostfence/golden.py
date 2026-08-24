"""Golden-image comparison for GHOSTFENCE.

The Stride model (`Stride.Graphics.Regression/ImageTester.cs`,
`ImageThreshold.cs`, `TestResultImage.cs`), read for pattern only: a fixed
camera, a fixed frame index, a stored reference, a perceptual threshold, and a
failure that writes the diff so a human can see *what* moved rather than being
told a number changed.

Somnium has 945-plus tests and, before this file, **zero image assertions**.
Every visual claim in every phase record rested on somebody looking at a
screenshot. This is the cheapest quality mechanism in Phase MORROWIND and it is
why §10 makes it the first GHOSTFENCE row.

Two thresholds, and both matter:

- **`max_channel`** catches a small area moving a lot — a widget shifting a
  pixel, a glyph losing its snap. A mean-only test sleeps through it.
- **`failing_fraction`** catches a large area moving a little — a tone-map or
  gamma drift that a max test would flag on a single lucky pixel and an
  eyeball would miss entirely.

A pixel counts as *failing* when its per-channel difference exceeds
`channel_tolerance`; the image fails when the failing fraction exceeds
`failing_fraction`, or when any channel moves by more than `max_channel`.
Encoder noise and a genuine regression are separated by those two numbers and
by nothing else, so they are arguments rather than constants.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from . import png


@dataclass(frozen=True)
class Threshold:
    """Perceptual tolerance for one comparison."""

    #: Per-channel 0..255 difference below which a pixel is considered equal.
    channel_tolerance: int = 2
    #: Fraction of pixels allowed to exceed `channel_tolerance`.
    failing_fraction: float = 0.001
    #: Absolute per-channel difference no pixel may exceed, whatever the count.
    max_channel: int = 24

    @staticmethod
    def exact() -> "Threshold":
        """Byte-identity. Used where the contract says byte-identical."""
        return Threshold(channel_tolerance=0, failing_fraction=0.0, max_channel=0)


@dataclass(frozen=True)
class Comparison:
    passed: bool
    reason: str
    total_pixels: int
    failing_pixels: int
    max_channel_delta: int
    mean_channel_delta: float
    diff_path: Path | None = None

    @property
    def failing_fraction(self) -> float:
        return self.failing_pixels / self.total_pixels if self.total_pixels else 0.0


def compare(
    reference: Path,
    candidate: Path,
    threshold: Threshold = Threshold(),
    diff_path: Path | None = None,
) -> Comparison:
    """Compare two PNGs, writing a diff image on failure.

    A size mismatch is a failure and not an error: a sub-phase that changed the
    capture resolution has changed the evidence, and the right response is to
    re-approve the reference deliberately rather than to crash the gate.
    """
    ref = png.read(reference)
    cand = png.read(candidate)

    if (ref.width, ref.height) != (cand.width, cand.height):
        return Comparison(
            passed=False,
            reason=(
                f"size changed: reference is {ref.width}x{ref.height}, "
                f"candidate is {cand.width}x{cand.height}"
            ),
            total_pixels=ref.width * ref.height,
            failing_pixels=ref.width * ref.height,
            max_channel_delta=255,
            mean_channel_delta=255.0,
        )

    total = ref.width * ref.height
    failing = 0
    max_delta = 0
    sum_delta = 0
    # Kept only when the comparison fails, so a passing run allocates nothing
    # beyond the two decoded images.
    diff = bytearray(total * 3)

    for y in range(ref.height):
        for x in range(ref.width):
            r0, g0, b0 = ref.rgb(x, y)
            r1, g1, b1 = cand.rgb(x, y)
            dr, dg, db = abs(r0 - r1), abs(g0 - g1), abs(b0 - b1)
            worst = max(dr, dg, db)
            sum_delta += dr + dg + db
            if worst > max_delta:
                max_delta = worst
            i = (y * ref.width + x) * 3
            if worst > threshold.channel_tolerance:
                failing += 1
                # Magenta scaled by severity: visible against every scene
                # Somnium ships, and brighter where the drift is worse.
                weight = min(255, 64 + worst * 3)
                diff[i], diff[i + 1], diff[i + 2] = weight, 0, weight
            else:
                # Darkened reference, so the unchanged image is still legible
                # underneath the highlight and the diff reads as a location.
                diff[i], diff[i + 1], diff[i + 2] = r0 // 4, g0 // 4, b0 // 4

    fraction = failing / total if total else 0.0
    mean = sum_delta / (total * 3) if total else 0.0

    reasons = []
    if fraction > threshold.failing_fraction:
        reasons.append(
            f"{failing:,} of {total:,} pixels ({fraction:.4%}) exceed "
            f"±{threshold.channel_tolerance}, over the {threshold.failing_fraction:.4%} budget"
        )
    if max_delta > threshold.max_channel:
        reasons.append(
            f"peak channel delta {max_delta} exceeds the {threshold.max_channel} ceiling"
        )

    if not reasons:
        return Comparison(
            passed=True,
            reason=f"within threshold (peak {max_delta}, {failing:,} pixels off)",
            total_pixels=total,
            failing_pixels=failing,
            max_channel_delta=max_delta,
            mean_channel_delta=mean,
        )

    written = None
    if diff_path is not None:
        png.write_rgb(diff_path, ref.width, ref.height, bytes(diff))
        written = diff_path

    return Comparison(
        passed=False,
        reason="; ".join(reasons),
        total_pixels=total,
        failing_pixels=failing,
        max_channel_delta=max_delta,
        mean_channel_delta=mean,
        diff_path=written,
    )
