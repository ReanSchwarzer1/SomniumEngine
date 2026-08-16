#!/usr/bin/env python3
"""Regenerate the Somnium horizontal lockups.

    python tools/build_lockup.py

Writes `somnium-lockup-horizontal.svg` (dark backgrounds) and
`somnium-lockup-horizontal-light.svg` (light backgrounds) into
`crates/somnium_ui/assets/brand/`.

Why this is a script and not a hand-edited file
-----------------------------------------------
The brand sheet requires the wordmark to be **converted to outlines**, not set
live. That is not pedantry: GitHub renders README SVGs through an `<img>` tag,
where Inter is unavailable, so a live `<text>` element silently falls back to
whatever `system-ui` is on the reader's machine — different metrics, and the
letter-spacing lands somewhere nobody approved. Outlines make the lockup
byte-identical everywhere.

The viewBox is **measured from the artwork**, not hardcoded. An earlier hand-set
box was 250 × 64 while the art only occupied x 15–154, y 18–49; the dead space
made a centred `<img>` look off-centre and the logo look small for the room it
took. Clear space is the blade stroke on all four sides, which is what the brand
sheet specifies.

Requires `fonttools` (`pip install fonttools`) and the bundled Inter cuts.
"""

import io
import os

from fontTools.misc.transform import Transform
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FONTS = os.path.join(ROOT, 'crates', 'somnium_ui', 'assets', 'fonts')
BRAND = os.path.join(ROOT, 'crates', 'somnium_ui', 'assets', 'brand')

# ── Palette ──────────────────────────────────────────────────────────────────
ACCENT = '#7A86FF'   # accent.default — the lunar indigo the editor runs on
MIST = '#D8DCE8'     # text.primary, for dark backgrounds
INK = '#14161C'      # surface.window, for light backgrounds

# ── Mark geometry (Eclipse, route A) ─────────────────────────────────────────
# Two counter-rotating blades on tangent circles, drawn on a 64-unit grid and
# scaled into the lockup band.
MARK_SCALE = 0.76
MARK_DX, MARK_DY = 6.0, 8.0
MARK_OFFSET_Y = 1.0
BLADE_R = 13.2
BLADE_STROKE = 9.2
MARK_PATH = 'M45.07 20.84 A13.2 13.2 0 1 0 27.92 31.55'

# Clear space, per the brand sheet: one blade stroke on all four sides.
CLEAR = BLADE_STROKE * MARK_SCALE

# ── Wordmark ─────────────────────────────────────────────────────────────────
WORD = ('Inter-SemiBold.ttf', 'Somnium', 21.0, 0.4, 58.0, 34.0)
SUB = ('Inter-Medium.ttf', 'ENGINE', 10.0, 3.6, 59.0, 49.0)


def lay_out(face, text, size, tracking, x, y):
    """Return (svg path data, bounding box) for one outlined run.

    `tracking` is extra advance per glyph, matching how CSS letter-spacing
    measures. The y axis is flipped here: font space is y-up, SVG is y-down.
    """
    font = TTFont(os.path.join(FONTS, face))
    upem = font['head'].unitsPerEm
    cmap = font.getBestCmap()
    glyphs = font.getGlyphSet()
    hmtx = font['hmtx']
    scale = size / upem

    commands = []
    box = [None, None, None, None]  # x0, y0, x1, y1
    cursor = x
    for ch in text:
        name = cmap.get(ord(ch))
        if name is None:
            cursor += size * 0.5 + tracking
            continue

        pen = SVGPathPen(glyphs)
        glyphs[name].draw(TransformPen(pen, Transform(scale, 0, 0, -scale, cursor, y)))
        data = pen.getCommands()
        if data:
            commands.append(data)

        bounds = BoundsPen(glyphs)
        glyphs[name].draw(bounds)
        if bounds.bounds:
            gx0, gy0, gx1, gy1 = bounds.bounds
            corners = (cursor + gx0 * scale, y - gy1 * scale,
                       cursor + gx1 * scale, y - gy0 * scale)
            box = [
                corners[0] if box[0] is None else min(box[0], corners[0]),
                corners[1] if box[1] is None else min(box[1], corners[1]),
                corners[2] if box[2] is None else max(box[2], corners[2]),
                corners[3] if box[3] is None else max(box[3], corners[3]),
            ]
        cursor += hmtx[name][0] * scale + tracking

    return ' '.join(commands), box


def mark_bounds():
    """Outer extent of the two stroked blades, in lockup units."""
    half = BLADE_STROKE / 2.0
    lo = (32.0 - BLADE_R - half)
    hi = (32.0 + BLADE_R + half)
    return (
        (MARK_DX + lo) * MARK_SCALE,
        MARK_OFFSET_Y + (MARK_DY + lo) * MARK_SCALE,
        (MARK_DX + hi) * MARK_SCALE,
        MARK_OFFSET_Y + (MARK_DY + hi) * MARK_SCALE,
    )


def build(mark_color, word_color):
    word_d, word_box = lay_out(*WORD)
    sub_d, sub_box = lay_out(*SUB)
    mark = mark_bounds()

    x0 = min(mark[0], word_box[0], sub_box[0]) - CLEAR
    y0 = min(mark[1], word_box[1], sub_box[1]) - CLEAR
    x1 = max(mark[2], word_box[2], sub_box[2]) + CLEAR
    y1 = max(mark[3], word_box[3], sub_box[3]) + CLEAR
    w, h = x1 - x0, y1 - y0

    return (
        '<svg xmlns="http://www.w3.org/2000/svg" '
        'viewBox="%.2f %.2f %.2f %.2f" width="%.0f" height="%.0f" '
        'role="img" aria-label="Somnium Engine">'
        '<g transform="translate(0 %g) scale(%g) translate(%g %g)" '
        'stroke="%s" stroke-width="%g" fill="none">'
        '<path d="%s"/>'
        '<g transform="rotate(180 32 32)"><path d="%s"/></g>'
        '</g>'
        '<path fill="%s" d="%s"/>'
        '<path fill="%s" fill-opacity="0.62" d="%s"/>'
        '</svg>\n'
        % (x0, y0, w, h, round(w), round(h),
           MARK_OFFSET_Y, MARK_SCALE, MARK_DX, MARK_DY,
           mark_color, BLADE_STROKE, MARK_PATH, MARK_PATH,
           word_color, word_d, word_color, sub_d)
    )


def main():
    for name, word in (('somnium-lockup-horizontal.svg', MIST),
                       ('somnium-lockup-horizontal-light.svg', INK)):
        path = os.path.join(BRAND, name)
        io.open(path, 'w', encoding='utf-8').write(build(ACCENT, word))
        print('wrote', os.path.relpath(path, ROOT))


if __name__ == '__main__':
    main()
