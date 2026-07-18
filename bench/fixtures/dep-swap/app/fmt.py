"""Display-width aware string formatting.

Terminals measure display width, not code-point count: combining marks
(e.g. accents in decomposed unicode) add no width. align() pads by
display width, so "café" counts as 4 cells whether it is stored composed
or decomposed.
"""

import unicodedata


def display_width(s):
    """Display width of s: combining marks count as zero cells."""
    return sum(1 for ch in unicodedata.normalize("NFD", s) if not unicodedata.combining(ch))


def align(s, width, fill=".", side="left"):
    """Pad s with fill until its display width equals width.

    side="left" aligns the text left (fill on the right), side="right"
    prepends the fill, side="center" splits the fill around the text.
    Strings already at or beyond width are returned unchanged.
    """
    if side not in ("left", "right", "center"):
        raise ValueError(f"unknown side: {side!r}")
    gap = width - display_width(s)
    if gap <= 0:
        return s
    if side == "right":
        return fill * gap + s
    if side == "center":
        left = gap // 2
        return fill * left + s + fill * (gap - left)
    return s + fill * gap
