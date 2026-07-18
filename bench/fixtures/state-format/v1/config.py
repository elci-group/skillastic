"""Configuration loading: reads the app settings from settings.ini."""

import configparser
import pathlib

_SETTINGS_PATH = pathlib.Path(__file__).resolve().parent / "settings.ini"


def load_settings():
    """Return the app settings as a flat dict: host, port, debug."""
    parser = configparser.ConfigParser()
    parser.read(_SETTINGS_PATH)
    return {
        "host": parser.get("server", "host"),
        "port": parser.getint("server", "port"),
        "debug": parser.getboolean("features", "debug"),
    }
