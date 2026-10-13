---
name: dsh-translate-docs
description: Use when writing or updating the Chinese (.zh.md) counterpart of an Agent Note in the moonfield repo — keeps the bilingual triplet in sync
---

# Translating Agent Notes to Chinese

Every Agent Note in `.agents/notes/` is a triplet: the English `.md`, the
Chinese `.zh.md`, and the `.i18n.yaml` consistency record. `scripts/verify_agents.py`
enforces the pairing; this skill is how to write the Chinese file.

## Rules

- **Structure mirrors section-for-section.** The `.zh.md` uses the exact same
  `## ` headings as the English file, in the same order; the script compares
  the heading sets.
- **Machine-checked tokens stay English verbatim**: the `# Agent Note: <title>`
  line and the `Status:` line and value.
- **Body is a real translation**, not a transliteration: formal written Chinese,
  no AI-flavored filler, no metaphor or analogy, read naturally for a Chinese
  reader. Keep code identifiers, paths, and crate names in their original form.
- Links in the Chinese file point at the corresponding files in the same
  directory (English sidecar link at the top mirrors the en file's Chinese link).
- Update the triplet together. Every change to the English file must be mirrored
  in the Chinese file in the same commit.

## After editing

1. Recompute the sidecar hash from the current English file:

```sh
python3 -c "import hashlib;print(hashlib.sha256(open('PATH.md','rb').read()).hexdigest())"
```

and update `sha256:` in the matching `.i18n.yaml`.

2. Run `python3 scripts/verify_agents.py` — it validates heading parity,
   header tokens, the sha256, and relative links.