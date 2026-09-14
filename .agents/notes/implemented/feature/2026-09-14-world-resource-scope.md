# Agent Note: World resource scope

Status: implemented

[中文](2026-09-14-world-resource-scope.zh.md)

## Problem

`AssetServer::load` needs `&mut AssetServer` and `&mut Assets<T>` at once,
but the world's resource storage hands out one borrow per resource. Every
caller hand-rolled the same workaround — `remove_resource`, use,
`insert_resource` — in `HandleTemplate::build`, the editor's scene loading,
and the UI's scene-load button; an early return on an error path that
skipped the reinsert left the resource silently out of the world.

## Decision

- `World::resource_scope<R, U>(&mut self, f: impl FnOnce(&mut World, &mut R)
  -> U)` temporarily removes `R` from the world's storage, runs `f` with
  the world and the resource, then reinserts. The reinsert runs after `f`
  returns on every path, so an early `return` cannot leave the resource
  out of the world; an `R` inserted inside `f` is replaced. Panics when the
  resource does not exist; `try_resource_scope` is the `Option` form.
- The scoped resource is out of the world for the duration of `f`, so `f`
  holds `&mut R` while mutably using other resources through the world —
  the double-`&mut` shape the workaround existed for.
- The three call sites (`load_with_server`, `HandleTemplate::build`, the
  scene-load button) each became one scope; the manual take/use/put-back
  dances and their error-path comments are gone.

## Alternatives considered

- **Interior mutability in `AssetServer` (queue loads, insert later).** The
  reference implementation's `AssetServer::load` takes `&self` because
  loading is asynchronous and the asset data lands in `Assets<T>` later;
  the double borrow never arises. moonfield's loads are synchronous by
  decision ([bsn-style scene templates](../architecture/2026-08-21-bsn-style-scene-templates.md)),
  so the borrow is inherent and the scope is the fit.
- **A `ParamSet`-style system parameter for conflicting resources.** That
  answers conflicts inside *system signatures*; the affected call sites are
  service-layer code holding a `&mut World`, where one scope is enough.

## Consequences

- Nested scopes compose N-resource access naturally.
- The editor's `EditorMainState` slot take/put-back is a different shape (a
  state held out for a whole frame phase, not a double borrow) and keeps
  its explicit block.
