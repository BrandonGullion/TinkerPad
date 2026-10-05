"""TinkerPad's dump library for Python.

``dump(value, title=None)`` writes one ``tinkerpad-dump-v1`` value to stdout as
``ESC ] 1337 ; TinkerPad= <json> BEL`` (docs/PLUGINS.md §6) and returns
``value``, so it can be dropped into an expression without changing it.

This file decides the shape; the app only renders it:

- a non-empty list whose items are all mappings, dataclasses, or plain objects
  -> ``table``, columns from the union of their keys in first-seen order, at
  most ``MAX_ROWS`` rows;
- any other mapping, list, tuple, set, dataclass, or object -> ``tree``;
- everything else -> ``value``.

Nesting deeper than ``MAX_DEPTH`` becomes a short summary string, a reference
back to an enclosing object becomes ``"[circular]"``, and values JSON can't
hold (bytes, dates, Decimal, sets, functions) become sensible scalars.
"""

from __future__ import annotations

import base64
import dataclasses
import json
import sys
from typing import Any

__all__ = ["dump", "to_dump_node", "MAX_DEPTH", "MAX_ROWS"]

PREFIX = b"\x1b]1337;TinkerPad="
BEL = b"\x07"

#: Levels of nesting shown; containers below that are summarised.
MAX_DEPTH = 5
#: Rows kept in a table, and items kept in a list, set, or mapping.
MAX_ROWS = 1000


def dump(value: Any, title: str | None = None) -> Any:
    """Show ``value`` in the output pane and return it unchanged."""
    body = json.dumps(to_dump_node(value, title), ensure_ascii=False).encode("utf-8")
    # Bytes, not text: the sentinel is a byte sequence, and the payload is
    # already UTF-8. `sys.stdout` is whatever is running this — a terminal, or
    # the helper's stream — and both offer `.buffer`.
    out = getattr(sys.stdout, "buffer", None)
    if out is None:  # pragma: no cover - only when stdout is text-only
        sys.stdout.write((PREFIX + body + BEL).decode("utf-8", "replace"))
        sys.stdout.flush()
        return value
    out.write(PREFIX + body + BEL)
    out.flush()
    return value


def to_dump_node(value: Any, title: str | None = None) -> dict[str, Any]:
    """The dump node for ``value``, without writing it."""
    node = _describe(value, 0, set())
    if title is not None:
        node["title"] = str(title)
    return node


def _describe(value: Any, depth: int, ancestors: set[int]) -> dict[str, Any]:
    if _is_table(value):
        return _table(value, depth, ancestors)
    if _is_container(value):
        return {
            "v": 1,
            "kind": "tree",
            "value": _to_json(value, depth, ancestors),
            "type": _type_name(value),
        }
    return {"v": 1, "kind": "value", "value": _scalar(value), "type": _type_name(value)}


def _is_table(value: Any) -> bool:
    """A list of rows: every item is something with named fields."""
    if not isinstance(value, (list, tuple)) or not value:
        return False
    return all(_fields(item) is not None for item in value)


def _table(rows: Any, depth: int, ancestors: set[int]) -> dict[str, Any]:
    ancestors.add(id(rows))
    kept = list(rows)[:MAX_ROWS]
    columns: list[str] = []
    for row in kept:
        for name in _fields(row) or {}:
            if name not in columns:
                columns.append(name)
    out = {
        "v": 1,
        "kind": "table",
        "columns": [{"name": name} for name in columns],
        "rows": [
            [_to_json((_fields(row) or {}).get(name), depth + 1, ancestors) for name in columns]
            for row in kept
        ],
        "truncated": len(rows) > len(kept),
    }
    ancestors.discard(id(rows))
    return out


def _fields(value: Any) -> dict[str, Any] | None:
    """The named fields of a row, or ``None`` if it isn't row-shaped."""
    if isinstance(value, dict):
        return {str(k): v for k, v in value.items()}
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        return {f.name: getattr(value, f.name) for f in dataclasses.fields(value)}
    # A plain object with attributes, but not a scalar wearing a __dict__.
    if hasattr(value, "__dict__") and not callable(value):
        attrs = {k: v for k, v in vars(value).items() if not k.startswith("_")}
        return attrs or None
    return None


def _is_container(value: Any) -> bool:
    if isinstance(value, (str, bytes, bytearray)):
        return False
    return (
        isinstance(value, (dict, list, tuple, set, frozenset))
        or _fields(value) is not None
        or hasattr(value, "_asdict")
    )


def _to_json(value: Any, depth: int, ancestors: set[int]) -> Any:
    """``value`` as something JSON can hold, cycle- and depth-safe."""
    if not _is_container(value):
        return _scalar(value)
    if id(value) in ancestors:
        return "[circular]"
    if depth >= MAX_DEPTH:
        return _summary(value)

    ancestors.add(id(value))
    try:
        if hasattr(value, "_asdict"):  # namedtuple
            value = value._asdict()
        elif dataclasses.is_dataclass(value) and not isinstance(value, type):
            value = _fields(value) or {}
        elif not isinstance(value, (dict, list, tuple, set, frozenset)):
            value = _fields(value) or {}

        if isinstance(value, dict):
            kept = list(value.items())[:MAX_ROWS]
            out: Any = {str(k): _to_json(v, depth + 1, ancestors) for k, v in kept}
            if len(value) > len(kept):
                out[f"… {len(value) - len(kept)} more"] = None
        else:
            items = list(value)
            kept_items = items[:MAX_ROWS]
            out = [_to_json(item, depth + 1, ancestors) for item in kept_items]
            if len(items) > len(kept_items):
                out.append(f"… {len(items) - len(kept_items)} more")
        return out
    finally:
        ancestors.discard(id(value))


def _summary(value: Any) -> str:
    """What is shown in place of a container too deep to expand."""
    name = _type_name(value)
    try:
        return f"{name}({len(value)})"  # type: ignore[arg-type]
    except TypeError:
        return f"{name}(…)"


def _scalar(value: Any) -> Any:
    """A JSON scalar for something that isn't a container."""
    if value is None or isinstance(value, (bool, int, str)):
        return value
    if isinstance(value, float):
        # JSON has no NaN or infinity; say what it was instead.
        return value if value == value and abs(value) != float("inf") else repr(value)
    if isinstance(value, (bytes, bytearray)):
        return base64.b64encode(bytes(value)).decode("ascii")
    return str(value)


def _type_name(value: Any) -> str:
    return type(value).__name__
