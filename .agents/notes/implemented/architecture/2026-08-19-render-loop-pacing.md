# Agent Note: Redraw-driven frame loop and pacing

Status: implemented

[中文](2026-08-19-render-loop-pacing.zh.md)

## Problem

A winit app must decide when to produce a frame: render continuously and waste
energy/idle GPU, or sleep between events and risk dropping input or UI updates
in continuous scenarios (editor repaint, animation). The backend also needs a
deterministic place to run the per-frame ECS and render work, and a way to
shut down from inside the loop for automated smoke tests.

## Decision

The frame loop is **redraw-driven**: `App::about_to_wait` only decides
`ControlFlow` and requests redraws; the whole frame (`App::update` →
`sync_windows` → `App::render` → frame-state clearing → exit check) runs inside
`WindowEvent::RedrawRequested`.

- Pacing is governed by the `WinitSettings` resource (`focused_mode` /
  `unfocused_mode`: `UpdateMode::Continuous` or `Reactive { wait, react_to_* }`,
  presets `game()` default / `desktop_app()` / `continuous()`), re-read on every
  frame decision so systems can change it at runtime.
- An idle Reactive loop can be woken from external threads and UI toolkits via
  the `EventLoopProxyWrapper` resource (`wake_up()`, sends
  `WinitUserEvent::WakeUp`).
- Smoke-test exit: setting `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` signals exit
  through the shared `WindowControl` after N rendered frames, so startup and
  shutdown can be exercised headlessly on a machine with a display.

## Alternatives considered

- **Continuous render unconditionally.** Rejected: burns CPU/GPU and battery
  while idle; `Reactive` mode exists precisely to avoid it.
- **Run the frame inside other winit events (e.g. `MainEventsCleared`).**
  Rejected: tying work to arbitrary events couples pacing to the backend's event
  distribution; `RedrawRequested` is the single winit-sanctioned per-frame hook
  and matches the request-draw contract.
- **A separate `WindowRequests::exit` channel.** Rejected: exit is a lifecycle
  concern of the shared `WindowControl`; routing it through a second channel
  would split the exit policy in two.

## Consequences

- Systems relying on frame timing must anchor to `App::update` / `App::render`,
  never to winit events directly — the event may arrive more than once per frame
  on some platforms.
- Reactive mode is the default posture; anything that animates or redraws
  per-frame must either set `WinitSettings::continuous()` or send
  `wake_up()` to stay alive.
- `MOONFIELD_EDITOR_AUTO_CLOSE` is a test seam, not a product feature; keep it
  cheap and side-effect free outside the editor.