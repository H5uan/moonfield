# Agent Notes

English | [中文](README.zh.md)

One kind of design doc lives here. An **Agent Note** records a decision or proposal
that affects this codebase — the *why* and *what we gave up*, the parts code and
docs can't carry. This file defines where Agent Notes live, when to write one, and
the in-file format.

## Layout and naming

Every Agent Note has two axes, both encoded in its path —
`{lifecycle}/{class}/yyyy-mm-dd-topic-title.md`:

- **Lifecycle** (the top-level folder) is the note's status:
  - `proposed/` — a proposal reviewed before implementation.
  - `implemented/` — the decision shipped, kept current with what actually shipped.
  - `rejected/` — considered and declined; keep it only while its rationale
    prevents a tempting, meaningful mistake.
- **Class** (the nested folder) is the kind of decision. The closed set below is
  enforced by `scripts/verify_agents.py`; unknown folders fail the gate. Adding a
  class requires updating the canonical set here and in the script.

| Class | What it covers |
|---|---|
| `feature` | A new user- or model-facing capability. |
| `bug-fix` | Corrects a defect or closes a gap a postmortem surfaced. |
| `simplification` | Removes code, behavior, or surface area without adding a capability. |
| `architecture` | A structural decision about the shipped source. |
| `process` | Tooling, policy, or workflow around the code — not runtime behavior. |
| `testing` | Test infrastructure and strategy. |

The date in the filename is when the topic was first proposed. Cross-references
between notes use relative markdown links — never bare prose — so they are
mechanically checkable.

## When to write one

Every non-trivial change MUST add or update at least one Agent Note in the same
commit (and pull request). Non-trivial means it changes behavior, architecture, a
contract across files or crates, process or tooling, testing strategy, or an
on-disk/wire/config format. A proposal for substantial future work starts in
`proposed/`; a decision already made starts in `implemented/`. Only a purely
mechanical or local edit is exempt. Updating the note that already owns the
decision satisfies the rule — never create a duplicate.

The CI gate only verifies that existing notes are well-formed; whether a change
was non-trivial is a judgment the author owns. `dsh-pre-push-checks` reminds the
writer.

## The file format

`scripts/verify_agents.py` enforces the format, classification, lifecycle
consistency, bilingual pairing, and relative links. CI runs it on every push and
pull request.

### The header block

The first three lines of every note are exactly:

```markdown
# Agent Note: <title>

Status: <status>
```

`Status:` agrees with the lifecycle folder: `proposed`, `implemented`, or
`rejected — <why, in one line>`.

### The body skeleton

- `proposed/`:

```markdown
## Problem
## Proposal
## Alternatives considered
## Acceptance criteria
## Risks
```

- `implemented/` — shipped reality in the present tense; spec-speak
  (`Proposal`/`Plan`/`Acceptance criteria`) is rejected by the gate:

```markdown
## Problem
## Decision
## Alternatives considered
## Consequences
```

- `rejected/` — the proposal, frozen; the verdict lives on the `Status:` line.

`## Alternatives considered` is mandatory in every note: record each genuine
alternative and why it lost, one bold-led paragraph per alternative. Never
invent alternatives after the fact.

### Chinese counterparts

Every note carries a `.zh.md` mirror (section-for-section) and a `.i18n.yaml`
consistency record (en/zh filenames + `sha256` of the English file). The
machine-checked header tokens — `# Agent Note: ` and `Status:` — stay English
verbatim in the Chinese file. Update all three files together.

## Moving between lifecycles

Moving a file between lifecycle folders means updating the `Status:` line and
re-satisfying that folder's skeleton in the same change. `proposed/` →
`implemented/` rewrites the proposal into present-tense shipped reality and folds
acceptance criteria and risks into consequences. `proposed/` → `rejected/` only
adds the reason to the `Status:` line.

## Verification

```sh
python3 scripts/verify_agents.py
```

Runs the full gate locally; CI runs it in the `agent-docs` job.