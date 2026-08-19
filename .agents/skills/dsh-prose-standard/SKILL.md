---
name: dsh-prose-standard
description: Use when writing or reviewing Markdown, comments, JSDoc, prompts, or visible strings in the moonfield repo — the style and slop rules for durable prose
---

# Prose standard

Applies to durable prose: Agent Notes, docs, comments, JSDoc, prompts, and
visible strings. English and Chinese mirrors follow the same rules with the
machine-checked tokens staying English.

## State current fact, never change history

- No "previously/now/no longer", PR or commit references, or stack positions in
  durable prose. Change stories belong in commits, PRs, and Agent Notes.
- No implementation-status annotations ("implemented!", "future: …").
- No reasoning transcripts: step-by-step narration, test walkthroughs, or
  rejected local alternatives. Keep the resulting contract or rationale.

## One home per fact

Every rule, fact, or contract has one home; elsewhere link there. Root
[AGENTS.md](../../../AGENTS.md) holds standing rules, mechanism descriptions
live in [docs/architecture.md](../../../docs/architecture.md), decisions in
[Agent Notes](../../notes/README.md). Do not restate what a link already owns.

## Concrete over metaphorical

Name the actual type, API, operation, or behavior. Reserve `seam`/`boundary`/
`gate` for their precise uses; a `gate` is a specific check, not a vibe. Avoid
emphatic filler ("critically", "very", bold everywhere) — reserve emphasis for
the clause that changes behavior.

## Slop checklist

Hunt these in any text; delete or relocate them:

- The same rule in more than one home.
- Narrated history or war stories.
- Hand-restated catalogs or JSDoc where source or a generator is authoritative.
- Reasoning transcripts and implementation narration.
- Rationale repeated beside sibling mentions instead of once at the owner.
- Paragraph walls — split or demote the detail to its home.
- Spec-speak in implemented Agent Notes ("should", migration plans, acceptance
  checklists) — an implemented note describes what is.

## Coverage

Behavior, failure, timing, ownership, modality, exceptions, and non-obvious
orientation belong in the local contract or note; reasoning does not. If a fact
matters for a later decision, record it in the Agent Note — that is the one
place "what we gave up" survives.