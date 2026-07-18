"""Configuration loading: reads the app settings from settings.json."""

import json
import pathlib

_SETTINGS_PATH = pathlib.Path(__file__).resolve().parent / "settings.json"


def load_settings():
    """Return the app settings as a flat dict: host, port, debug."""
    data = json.loads(_SETTINGS_PATH.read_text())
    return {
        "host": data["server"]["host"],
        "port": data["server"]["port"],
        "debug": data["features"]["debug"],
    }
