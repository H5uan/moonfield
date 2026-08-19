#!/usr/bin/env python3
"""Verify .agents/notes structure, format, and bilingual pairing.

Enforced by CI (agent-docs job) and documented in .agents/notes/README.md.
Zero dependencies, Python 3 stdlib only. Exit code 0 = pass.
"""
import hashlib
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
NOTES = ROOT / ".agents" / "notes"

LIFECYCLES = {"proposed", "implemented", "rejected"}
CLASSES = {
    "architecture",
    "bug-fix",
    "feature",
    "process",
    "simplification",
    "testing",
}
HEADER_RE = re.compile(r"^# Agent Note: .+$")
STATUS_RE = re.compile(r"^Status: (.+)$")
FILENAME_RE = re.compile(r"\d{4}-\d{2}-\d{2}-.+\.md")

PROPOSED_REQUIRED = [
    "## Problem",
    "## Proposal",
    "## Alternatives considered",
    "## Acceptance criteria",
    "## Risks",
]
IMPLEMENTED_REQUIRED = [
    "## Problem",
    "## Decision",
    "## Alternatives considered",
    "## Consequences",
]
IMPLEMENTED_FORBIDDEN = [
    "## Proposal",
    "## Plan",
    "## Migration plan",
    "## Acceptance criteria",
]
REJECTED_REQUIRED = [
    "## Problem",
    "## Proposal",
    "## Alternatives considered",
]
SIDECAR_RE = re.compile(r"^([A-Za-z0-9_]+):\s*(.+)$", re.MULTILINE)

LINK_RE = re.compile(r"\]\(([^)#]+?)(?:#[^)]*)?\)")

errors: list[str] = []


def fail(msg: str, path: pathlib.Path | None = None) -> None:
    errors.append(f"{path}: {msg}" if path else msg)


def sections(text: str) -> set[str]:
    return {line.strip() for line in text.splitlines() if line.strip().startswith("## ")}


def valid_md_target(target: pathlib.Path) -> bool:
    return target.exists() and target.is_file()


def check_links(path: pathlib.Path, text: str) -> None:
    """Basic relative-link reachability: file targets must exist; anchors skipped."""
    for m in LINK_RE.finditer(text):
        target = m.group(1)
        if not target:
            continue
        if "://" in target or target.startswith(("mailto:", "#")):
            continue
        resolved = (path.parent / target).resolve()
        if not valid_md_target(resolved):
            fail(f"broken relative link to {target!r}", path)


def read_sidecar(path: pathlib.Path) -> dict[str, str]:
    """Parse the tiny i18n.yaml sidecar (en/zh/sha256 lines) without PyYAML."""
    data = {}
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        return data
    for m in SIDECAR_RE.finditer(text):
        key, value = m.group(1), m.group(2).strip().strip("\"'")
        if key in {"en", "zh", "sha256"}:
            data[key] = value
    return data


def main() -> int:
    if not NOTES.exists():
        print(f"skip: {NOTES} does not exist yet")
        return 0

    en_files = sorted(
        p
        for p in NOTES.rglob("*.md")
        if p.name not in {"README.md", "README.zh.md", "AGENTS.md", "CLAUDE.md"}
        and not p.name.endswith(".zh.md")
    )
    zh_files = sorted(p for p in NOTES.rglob("*.zh.md") if p.name != "README.zh.md")

    for path in en_files:
        rel = path.relative_to(NOTES)
        parts = rel.parts
        # 1. Path shape: {lifecycle}/{class}/yyyy-mm-dd-slug.md
        if len(parts) != 3:
            fail(f"path must be {{lifecycle}}/{{class}}/yyyy-mm-dd-slug.md, got {rel}", path)
            continue
        lifecycle, cls, stem = parts
        if lifecycle not in LIFECYCLES:
            fail(f"unknown lifecycle folder {lifecycle!r} (must be one of {sorted(LIFECYCLES)})", path)
        if cls not in CLASSES:
            fail(f"unknown class folder {cls!r} (must be one of {sorted(CLASSES)})", path)
        if not FILENAME_RE.fullmatch(stem):
            fail(f"filename must match yyyy-mm-dd-slug.md, got {stem!r}", path)

        text = path.read_text(encoding="utf-8")
        lines = text.splitlines()

        # 2. Header block: '# Agent Note: <title>' / blank / 'Status: <status>'
        if len(lines) < 3 or not HEADER_RE.match(lines[0].strip()) or lines[1].strip() or not STATUS_RE.match(lines[2].strip()):
            fail("header must be: '# Agent Note: <title>' / blank / 'Status: <status>'", path)
            continue
        status = STATUS_RE.match(lines[2].strip()).group(1)

        # 3. Status agrees with the lifecycle folder
        if lifecycle == "rejected":
            if not status.startswith("rejected"):
                fail(f"rejected note must start 'Status: rejected — <why>', got {status!r}", path)
        elif status.strip() != lifecycle:
            fail(f"Status {status!r} does not match lifecycle folder {lifecycle!r}", path)

        # 4. Body skeleton
        secs = sections(text)
        if lifecycle == "proposed":
            missing = [s for s in PROPOSED_REQUIRED if s not in secs]
            if missing:
                fail(f"proposed note missing required sections {missing}", path)
        elif lifecycle == "implemented":
            missing = [s for s in IMPLEMENTED_REQUIRED if s not in secs]
            if missing:
                fail(f"implemented note missing required sections {missing}", path)
            forbidden = [s for s in IMPLEMENTED_FORBIDDEN if s in secs]
            if forbidden:
                fail(f"implemented note contains spec-speak sections {forbidden} (write present-tense shipped reality)", path)
        elif lifecycle == "rejected":
            missing = [s for s in REJECTED_REQUIRED if s not in secs]
            if missing:
                fail(f"rejected note missing required sections {missing}", path)

        # 5. Bilingual triplet: en + zh mirror + sidecar consistency record
        zh_path = path.with_name(stem.removesuffix(".md") + ".zh.md")
        sidecar = path.with_suffix(".i18n.yaml")
        if not zh_path.exists():
            fail(f"missing Chinese counterpart {zh_path.relative_to(ROOT)}", path)
        elif not sidecar.exists():
            fail(f"missing sidecar {sidecar.relative_to(ROOT)}", path)
        else:
            meta = read_sidecar(sidecar)
            sha = hashlib.sha256(text.encode()).hexdigest()
            if meta.get("sha256") != sha:
                fail("sidecar sha256 does not match en file content (run the translate step)", path)
        zh_text = zh_path.read_text(encoding="utf-8") if zh_path.exists() else ""
        if zh_text:
            zh_lines = zh_text.splitlines()
            if len(zh_lines) >= 3 and not HEADER_RE.match(zh_lines[0].strip()):
                fail("zh file must keep the English '# Agent Note:' header token", zh_path)
            if sections(zh_text) != secs:
                fail("zh section headings do not mirror the en sections", path)

    # 6. Every zh file must have an en counterpart
    for zh in zh_files:
        en = zh.with_name(zh.name.removesuffix(".zh.md") + ".md")
        if not en.exists():
            fail(f"missing en counterpart {en.relative_to(ROOT)}", zh)

    # 7. Basic relative-link checks on all notes
    for p in en_files + zh_files:
        if p.exists():
            check_links(p, p.read_text(encoding="utf-8"))

    if errors:
        print(f"FAIL: {len(errors)} problem(s) in agent notes")
        for e in errors:
            print(f"  - {e}")
        return 1
    print(f"ok: {len(en_files)} note(s), {len(zh_files)} zh mirror(s) verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())