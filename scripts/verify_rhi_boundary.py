#!/usr/bin/env python3
"""Verify that moonfield-rhi's public API exposes no backend types.

The RHI boundary rule (crates/moonfield-rhi/AGENTS.md): nothing public may
mention `ash`, `vk::`, or `gpu_allocator` — capabilities must be exposed
through first-class crate-vocabulary APIs instead. Enforced by CI
(boundary job) and runnable locally:

    python3 scripts/verify_rhi_boundary.py

Zero dependencies, Python 3 stdlib only. Exit code 0 = pass.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
RHI_SRC = ROOT / "crates" / "moonfield-rhi" / "src"
# In-crate test modules may touch internals freely.
EXEMPT_DIRS = {"gpu_tests"}
FORBIDDEN = re.compile(r"\bash::|\bvk::|\bgpu_allocator\b")
# A restricted-visibility opener (pub(crate), pub(super), pub(self)) is not
# public API, hence the negative lookahead for `(`.
PUB_ITEM_RE = re.compile(r"^\s*pub\s+(?!\()(?:unsafe\s+)?(?:fn|struct|enum|type|const|static|use|trait)\b")
PUB_FIELD_RE = re.compile(r"^\s*pub\s+[a-zA-Z_][a-zA-Z0-9_]*\s*:")
IMPL_FROM_RE = re.compile(r"^\s*impl.*\bFrom\s*<")

errors: list[str] = []


def strip_comment(line: str) -> str:
    return line.split("//", 1)[0]


def check_file(path: pathlib.Path) -> None:
    lines = path.read_text(encoding="utf-8").splitlines()
    i = 0
    while i < len(lines):
        raw = strip_comment(lines[i])
        if PUB_ITEM_RE.match(raw) or IMPL_FROM_RE.match(raw) or PUB_FIELD_RE.match(raw):
            # Accumulate the signature: continuation lines until the body `{`,
            # a `;` terminator, or a `where` clause that opens the body.
            block = [raw]
            balance = raw.count("(") - raw.count(")") + raw.count("<") - raw.count(">")
            while "{" not in "".join(block) and not raw.rstrip().endswith((";", "{")) and (
                balance > 0 or raw.rstrip().endswith((",", "(", "<"))
            ):
                i += 1
                if i >= len(lines):
                    break
                raw = strip_comment(lines[i])
                block.append(raw)
                balance += raw.count("(") - raw.count(")") + raw.count("<") - raw.count(">")
            text = "\n".join(block)
            # For const/static items the public contract is the type
            # ascription, not the initializer (an initializer is an
            # implementation detail, like a fn body).
            if re.match(r"^\s*pub\s+(?:const|static)\b", block[0]):
                text = text.split("=", 1)[0]
            # Restricted-visibility fields/segments are not public API;
            # drop `pub(crate) <tokens>` spans before scanning.
            text = re.sub(r"pub\s*\([^)]*\)\s*[^,;){}\n]+", "", text)
            if FORBIDDEN.search(text):
                errors.append(f"{path.relative_to(ROOT)}:{i + 1}: public item mentions a backend type")
        i += 1


def main() -> int:
    if not RHI_SRC.exists():
        print(f"skip: {RHI_SRC} does not exist")
        return 0
    for path in sorted(RHI_SRC.rglob("*.rs")):
        if any(part in EXEMPT_DIRS for part in path.relative_to(RHI_SRC).parts[:-1]):
            continue
        check_file(path)
    if errors:
        print(f"FAIL: {len(errors)} public API leak(s) in moonfield-rhi")
        for e in errors:
            print(f"  - {e}")
        return 1
    print("ok: moonfield-rhi public API exposes no ash/vk/gpu_allocator types")
    return 0


if __name__ == "__main__":
    sys.exit(main())
