"""Customer data access API (v1).

The store keeps data canonical (emails lowercase/trimmed, display names
title-cased), so reads are plain dict lookups: no post-processing needed.
"""

_ROWS = {
    "user": {
        1: {"email": "alice@example.com", "display_name": "Alice Johnson"},
        2: {"email": "bob@example.com", "display_name": "Bob Smith"},
    }
}


def query(entity, id):
    """Return the record for (entity, id) as a plain dict, or None."""
    raw = _ROWS.get(entity, {}).get(id)
    return dict(raw) if raw is not None else None
