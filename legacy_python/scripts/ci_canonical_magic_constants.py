#!/usr/bin/env python3
"""FB-CAN-067 — guardrail against new untracked float literals in canonical paths.

Scans Python modules under decision_engine/, risk_engine/, execution/, app/config/ for
direct float constants in assignments and comparisons (AST). Known lines are listed in
``ci_canonical_magic_constants_allowlist.txt`` (path:line). New magic numbers must either
move to canonical YAML / settings or add the line to the allowlist with review, or mark the
line with ``# noqa: canonical-magic``.

Run from repo root: ``python3 scripts/ci_canonical_magic_constants.py``
"""

from __future__ import annotations

import ast
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ALLOWLIST_PATH = Path(__file__).resolve().parent / "ci_canonical_magic_constants_allowlist.txt"

SCAN_ROOTS = [
    ROOT / "decision_engine",
    ROOT / "risk_engine",
    ROOT / "execution",
    ROOT / "app" / "config",
]

# Structural / math floats that are not "policy thresholds" for allowlist noise reduction.
_WHITELIST_VALUES: frozenset[float] = frozenset(
    {
        0.0,
        1.0,
        -1.0,
        2.0,
        0.5,
        -0.5,
        0.25,
        -0.25,
        0.75,
        -0.75,
    }
)


def _rel(p: Path) -> str:
    return str(p.relative_to(ROOT)).replace("\\", "/")


def _load_allowlist() -> set[str]:
    if not ALLOWLIST_PATH.is_file():
        return set()
    out: set[str] = set()
    for ln in ALLOWLIST_PATH.read_text(encoding="utf-8").splitlines():
        s = ln.strip()
        if not s or s.startswith("#"):
            continue
        out.add(s)
    return out


class _FloatLiteralVisitor(ast.NodeVisitor):
    def __init__(self, path: Path) -> None:
        self.path = path
        self.hits: list[tuple[int, str]] = []

    def visit_Assign(self, node: ast.Assign) -> None:
        self._check_value(node.value, node.lineno)

    def visit_AnnAssign(self, node: ast.AnnAssign) -> None:
        if node.value is not None:
            self._check_value(node.value, node.lineno)

    def visit_Compare(self, node: ast.Compare) -> None:
        for comp in [node.left, *node.comparators]:
            self._check_value(comp, node.lineno)

    def _check_value(self, node: ast.AST, lineno: int) -> None:
        if isinstance(node, ast.Constant) and isinstance(node.value, float):
            v = float(node.value)
            if v in _WHITELIST_VALUES or v != v:  # nan
                return
            self.hits.append((lineno, f"float literal {v!r}"))
        elif isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
            if isinstance(node.operand, ast.Constant) and isinstance(node.operand.value, float):
                v = -float(node.operand.value)
                if v in _WHITELIST_VALUES:
                    return
                self.hits.append((lineno, f"float literal {v!r}"))


def _line_has_noqa(lines: list[str], lineno: int) -> bool:
    if lineno < 1 or lineno > len(lines):
        return False
    return "noqa: canonical-magic" in lines[lineno - 1]


def scan_file(path: Path, lines: list[str], allowlist: set[str]) -> list[str]:
    rel = _rel(path)
    try:
        tree = ast.parse("".join(lines), filename=str(path))
    except SyntaxError as e:
        return [f"{rel}: syntax error: {e}"]
    v = _FloatLiteralVisitor(path)
    v.visit(tree)
    seen_lines: set[int] = set()
    errors: list[str] = []
    for lineno, _msg in v.hits:
        if lineno in seen_lines:
            continue
        seen_lines.add(lineno)
        key = f"{rel}:{lineno}"
        if key in allowlist:
            continue
        if _line_has_noqa(lines, lineno):
            continue
        errors.append(f"{key} — untracked float literal (use config / allowlist / # noqa: canonical-magic)")
    return errors


def iter_py_files() -> list[Path]:
    out: list[Path] = []
    for base in SCAN_ROOTS:
        if not base.is_dir():
            continue
        for p in base.rglob("*.py"):
            if "__pycache__" in p.parts:
                continue
            out.append(p)
    return sorted(out)


def main() -> int:
    allowlist = _load_allowlist()
    all_errors: list[str] = []
    for path in iter_py_files():
        text = path.read_text(encoding="utf-8")
        lines = text.splitlines(keepends=True)
        all_errors.extend(scan_file(path, lines, allowlist))
    if all_errors:
        print("ci_canonical_magic_constants: FAIL", file=sys.stderr)
        for e in all_errors:
            print(e, file=sys.stderr)
        print(
            f"\nAllowlist: {ALLOWLIST_PATH.relative_to(ROOT)} (path:line per line).",
            file=sys.stderr,
        )
        return 1
    print("ci_canonical_magic_constants: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
