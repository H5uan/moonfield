# Agent Note: ComponentMeta's debug-only field is read through one cfg-bridging accessor

Status: implemented

[中文](2026-09-21-componentmeta-debug-only-field-accessor.zh.md)

## Problem

`ComponentMeta.type_name` exists only under `#[cfg(debug_assertions)]`, but the
panic messages in `Archetype::borrow`, `Archetype::borrow_raw`, and
`Archetype::borrow_mut` read `self.metas[column].type_name` unconditionally.
Dev-profile builds compile because `debug_assertions` is on; `cargo build
--release` fails with E0609, no field `type_name`. CI's clippy and test jobs
run in the dev profile, so nothing in the gate builds without
`debug_assertions`.

## Decision

All reads of the field go through one accessor,
`ComponentMeta::type_name()`, which returns the stored name under
`debug_assertions` and `"<unknown>"` otherwise. The duplicate-component panic
in `assert_component_meta` collapsed from two `cfg`-selected arms to a single
arm using the same accessor.

## Alternatives considered

- **Call `core::any::type_name::<T>()` at the sites where `T` is in scope.**
  Rejected: exact in every profile, but only `borrow` and `borrow_mut` have
  `T`; `borrow_raw` is type-erased and would keep its own `cfg` arm, leaving
  two mechanisms for the same message.
- **Keep per-site `cfg` arms, the pattern `assert_component_meta` used.**
  Rejected: every new panic message re-implements the release fallback; the
  accessor owns it once.

## Consequences

- Release builds compile; their borrow-conflict and duplicate-component panic
  messages name components as `"<unknown>"`.
- Adding another `debug_assertions`-gated field to `ComponentMeta` means
  extending the accessor, not adding call-site `cfg` arms.
- The gate still builds no release target; release-only breakage remains
  invisible to CI.
