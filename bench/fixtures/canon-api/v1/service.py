"""User-facing service helpers over the data API."""

import api


def get_user_email(user_id):
    row = api.query("user", user_id)
    return row["email"] if row is not None else None


def get_user_display_name(user_id):
    row = api.query("user", user_id)
    return row["display_name"] if row is not None else None
