"""User-facing service helpers over the data API."""

import api


def get_user_email(user_id):
    """Return the canonical email for the user, or None if unknown."""
    raise NotImplementedError


def get_user_display_name(user_id):
    """Return the canonical display name for the user, or None if unknown."""
    raise NotImplementedError
