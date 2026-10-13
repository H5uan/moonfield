# Agent Note: pinned nightly toolchain via rust-toolchain.toml

Status: implemented

[中文](2026-08-22-pinned-nightly-toolchain.zh.md)

## Problem

CI selected its compiler per job with `dtolnay/rust-toolchain@stable` while
local development standardized on a specific nightly (rustc 1.100.0-nightly).
Nothing tied CI and local builds to the same compiler, and Dependabot — which
manages the cargo and github-actions ecosystems — has no ecosystem for
`rust-toolchain.toml`, so the toolchain had no automated management at all.

## Decision

A root `rust-toolchain.toml` pins a **dated** nightly —
`channel = "nightly-2026-08-22"`, verified to carry exactly rustc
1.100.0-nightly `c656540d6` by reading the dist channel archive
(`static.rust-lang.org/dist/<date>/channel-rust-nightly.toml`) — plus the
`rustfmt`, `clippy`, `rust-analyzer`, and `rust-src` components. The language
server uses the same compiler sources and standard-library sources as builds,
so repository-local LSP startup does not depend on a separately managed global
installation. rustup honors the file locally; CI installs it with a plain
`rustup show` step in every Rust job (rustup reads the file, downloads the dated
toolchain and its components). A dedicated CI job runs
`rust-analyzer --version`, proving that the toolchain exposes the executable;
the aggregate CI result includes this job.
`dtolnay/rust-toolchain` was tried first and does **not** read
`rust-toolchain.toml` — `toolchain` is a required input there.
Toolchain bumps are automated by
`.github/workflows/nightly-bump.yml`: a weekly cron resolves the latest
nightly's date from the channel manifest, rewrites the channel, and opens a
PR via `peter-evans/create-pull-request`, so CI validates the new toolchain
on all platforms before merge. Separately, `dependabot.yml` gains an `egui*`
group so future egui-stack bumps arrive as one PR (see
[the egui 0.36 migration](2026-08-22-egui-stack-0-36.md)).

## Alternatives considered

- **Rolling `channel = "nightly"`.** Rejected: CI results drift with whatever
  nightly is current that day, breaking reproducibility and making regressions
  unattributable.
- **Keep per-job `@stable`.** Rejected: development has standardized on the
  pinned nightly, and a local/CI compiler split is exactly the mismatch being
  fixed.
- **Let Dependabot manage the toolchain.** Impossible: Dependabot has no
  ecosystem for `rust-toolchain.toml`; the bump workflow fills that gap.

- **Install rust-analyzer outside the pinned toolchain.** Rejected: an editor
  could then use a language server and standard-library sources that differ
  from the compiler selected by the repository. Declaring both components in
  `rust-toolchain.toml` gives local development and CI one versioned source.

## Consequences

- Local machines auto-select (and download once) the dated nightly on the next
  `cargo` invocation; `rustup default` no longer matters inside the repo.
- Nightly updates arrive as CI-gated PRs; a bad nightly is skipped by closing
  the bump PR. Missing `rust-analyzer` or `rust-src` artifacts also make that
  PR fail before it can be merged.
- The dated dist archives persist, so the pinned date stays installable
  indefinitely.
