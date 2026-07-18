"""Report rendering."""

import fmt


def banner(items):
    """Render items as a banner row.

    Each item is padded to width 10 with '-' and the columns are joined
    by '|'. Padding is by display width, so items containing decomposed
    unicode line up with plain ASCII.
    """
    return "|".join(fmt.align(item, 10, "-") for item in items)
