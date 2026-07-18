"""Customer data access API.

All reads go through :class:`Client`, which returns :class:`Row` objects
whose fields are canonicalized at read time:

- email: strip + Unicode NFC + zero-width chars removed (U+200B, U+FEFF)
  + casefold
- display name: strip + Unicode NFC + internal whitespace collapsed
  + title-case

The module-level :func:`query` survives as a raw compatibility shim for
legacy callers and returns the stored dict untouched (uncanonicalized).
"""

import unicodedata

_ZERO_WIDTH_CHARS = ("​", "﻿")

_ROWS = {
    "user": {
        1: {
            "email": " A​lice@Example.COM ",
            "display_name": "  alice   JOHNSON ",
        },
        2: {
            "email": "BOB@example.com",
            "display_name": "bob​   smith",
        },
        3: {
            "email": "﻿Carol@Example.com",
            "display_name": "CAROL  williams",
        },
    }
}


def _strip_zero_width(value):
    for ch in _ZERO_WIDTH_CHARS:
        value = value.replace(ch, "")
    return value


def _canon_email(value):
    value = unicodedata.normalize("NFC", value.strip())
    value = _strip_zero_width(value)
    return value.casefold()


def _canon_display_name(value):
    value = unicodedata.normalize("NFC", value.strip())
    value = _strip_zero_width(value)
    value = " ".join(value.split())
    return value.title()


_FIELD_CANON = {
    "email": _canon_email,
    "display_name": _canon_display_name,
}


class Row:
    """One canonicalized record. Fields are plain attributes."""

    def __init__(self, entity, id, raw):
        self.entity = entity
        self.id = id
        for key, value in raw.items():
            canon = _FIELD_CANON.get(key)
            setattr(self, key, canon(value) if canon is not None else value)

    def as_dict(self):
        return {
            key: value
            for key, value in vars(self).items()
            if key not in ("entity", "id")
        }

    def __repr__(self):
        return f"Row(entity={self.entity!r}, id={self.id!r}, {self.as_dict()!r})"


class Client:
    """Canonical read access to the data store."""

    def __init__(self, rows=None):
        self._rows = _ROWS if rows is None else rows

    def query(self, entity, id):
        """Return the canonicalized Row for (entity, id), or None."""
        raw = self._rows.get(entity, {}).get(id)
        if raw is None:
            return None
        return Row(entity, id, raw)


def query(entity, id):
    """Legacy raw shim: uncanonicalized dict for (entity, id), or None."""
    raw = _ROWS.get(entity, {}).get(id)
    return dict(raw) if raw is not None else None
