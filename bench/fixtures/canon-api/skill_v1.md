# Skill: customer data access (canon-api)

How to read customer records in this codebase.

## Conventions

- All reads go through the module-level function `api.query(entity, id)`.
  It returns a plain `dict` of the record's fields, or `None` when the id is
  unknown. There is no client object to instantiate — just call the function.
- The store keeps data **canonical already**: emails are lowercase and
  trimmed, display names are title-cased with single spaces. Use the fields
  exactly as they come back.
- Because of that, service code must **never** strip, lowercase, casefold,
  or unicode-normalize what `api.query` returns. Re-processing canonical
  data is wasted work and a source of bugs — if you find yourself reaching
  for `unicodedata` or `str.casefold` in service code, stop.

## Patterns

Wrap `api.query` in small helpers and index the dict directly:

```python
import api

def get_user_email(user_id):
    row = api.query("user", user_id)
    return row["email"] if row is not None else None
```

- Check for `None` before indexing; unknown ids are normal.
- Field access is by dict key: `row["email"]`, `row["display_name"]`.
