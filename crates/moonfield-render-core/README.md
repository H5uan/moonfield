# Selene

Moonfield's render engine layer: per-frame extraction, camera snapshots and
view targets, the window frame loop, and `RenderPlugin`. It sits between the
Lunar Mare RHI (`moonfield-rhi`) and the feature crates
(`moonfield-render-feature`, Lunaris) and keeps no `ash` dependency of its own.

Part of the [moonfield](https://github.com/H5uan/moonfield) engine.