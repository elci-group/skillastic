# Skill: configuration (state-format)

How application settings are stored and loaded.

## Conventions

- App settings live in `settings.ini`, parsed with `configparser`
  (stdlib). Do not introduce other formats or parsers.
- Sections and keys:
  - `[server]`: `host` (str), `port` (int)
  - `[features]`: `debug` (bool)
- Always convert with the typed accessors: `parser.getint("server",
  "port")` and `parser.getboolean("features", "debug")`. Never hand-roll
  `int()` / `bool()` on raw strings — `bool("false")` is True.
- `load_settings()` flattens the sections into `{"host", "port",
  "debug"}` for the rest of the app.

## Patterns

```python
import configparser

parser = configparser.ConfigParser()
parser.read("settings.ini")
port = parser.getint("server", "port")
debug = parser.getboolean("features", "debug")
```


---

## Migration Notes (app v2.0.0)

_Migrated from app v1.0.0 on 2026-07-18. Run `skillastic verify` after reviewing these assumptions._

### Breaking changes
- `e3517ed3` feat(config)!: switch settings from INI to nested settings.json (server/features sections)
