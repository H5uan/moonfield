---
name: dsh-code-review
description: Use when reviewing a pull request in the moonfield repo — orients the reviewer to this codebase's standing rules and the review-specific checks code alone can't show
---

# Reviewing a moonfield PR

Guidance, not a checklist. Review the live base and head, then walk the diff
with the code around it. Prioritize correctness, lifecycle, ownership, and
broken required behavior over style; a short review with one substantiated
blocker beats a list of nits.

## Sources of truth

- [AGENTS.md](../../../AGENTS.md) — standing repo rules.
- [crates/AGENTS.md](../../../crates/AGENTS.md) and
  [crates/moonfield-rhi/AGENTS.md](../../../crates/moonfield-rhi/AGENTS.md) —
  workspace and Vulkan-specific rules.
- [docs/architecture.md](../../../docs/architecture.md) — runtime mechanisms.
- [Agent Notes](../../notes/README.md) — design rationale. Treat disagreement
  with an implemented Agent Note as a design discussion, not an automatic veto:
  verify the claim first.

## Blocking requirements

1. **Agent Note present or exemption justified.** Non-trivial change (behavior,
   architecture, cross-crate contract, process, testing strategy, on-disk/wire/
   config format) carries a new or updated note in the same diff; purely
   mechanical edits are exempt. A proposal for future work starts in
   `proposed/`; an implemented decision is present-tense in `implemented/` with
   no spec-speak sections.
2. **Bilingual triplet complete.** New/edited notes keep `.zh.md` + `.i18n.yaml`
   in sync (headings mirror, machine tokens English, sidecar hash current).
3. **Docs match the code.** Config, defaults, errors, wire fields, events, and
   public behavior update README/JSDoc-level docs in the same diff; mechanisms
   moved this way land in [docs/architecture.md](../../../docs/architecture.md).
4. **Boundaries respected.** `moonfield-rhi` stays the only `ash`-linked
   crate; Vulkan objects stay main-thread; clip conventions (Y-up, reverse-Z)
   are adjusted only at the Vulkan boundary.
5. **Evidence matches surface.** Focused tests for behavior, snapshots or e2e
   for visible output, `verify_agents.py` for docs. Don't demand the full suite
   locally — CI owns it.

## Manual checks

- Trace every changed interface to both sides, including errors, ownership, and
  destruction order (Vulkan).
- Follow every denial path to the operation that enforces it; exercise alternate
  callers that could bypass validation.
- Check window-state mutations go through the `Window` component (no invented
  request channel) and that `WindowEvents` consumers drain each frame.
- Verify ECS change-tick use matches the schedule-run window; don't advance the
  clock mid-run.
- Verify tests assert external state or events, not implementation restatement;
  a green test that only trusts the writer's report is not evidence.