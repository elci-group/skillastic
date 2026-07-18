# Skill: text formatting (dep-swap)

How to pad and align strings in this codebase.

## Conventions

- The house standard for padding is `textutil.pad(s, width, fill=" ")`.
  Everything that renders columns or banners goes through it.
- Width is plain `len()` — one code point is one display cell. That is
  correct for the data we handle, so do **not** add unicode awareness:
  no normalization passes, no combining-mark logic, no `unicodedata`
  imports. It would only slow down hot rendering paths.
- Padding goes on the right; columns are joined with `|`.

## Patterns

```python
import textutil

def banner(items):
    return "|".join(textutil.pad(item, 10, "-") for item in items)
```

- Keep formatting helpers tiny and dependency-free (stdlib only).
- If a string is already at width, `pad` returns it unchanged — rely on that.
