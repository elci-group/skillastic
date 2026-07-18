"""User-facing service helpers over the data API."""

import api

_client = api.Client()


def get_user_email(user_id):
    """Return the canonical email for the user, or None if unknown."""
    row = _client.query("user", user_id)
    return row.email if row is not None else None


def get_user_display_name(user_id):
    """Return the canonical display name for the user, or None if unknown."""
    row = _client.query("user", user_id)
    return row.display_name if row is not None else None
