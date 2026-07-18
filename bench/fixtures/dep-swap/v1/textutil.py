"""Legacy text helpers.

The original v1 formatting utility. Superseded by fmt.py, kept only so
old imports don't break. pad() measures plain len(), which miscounts
strings containing decomposed unicode.
"""


def pad(s, width, fill=" "):
    """Pad s on the right with fill until len(s) == width."""
    if len(s) >= width:
        return s
    return s + fill * (width - len(s))
