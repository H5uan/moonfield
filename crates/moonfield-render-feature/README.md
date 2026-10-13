# Moonfield Render Feature (Lunaris)

Moonfield's high-level rendering feature crate. It builds on the Lunar Mare RHI
(`moonfield-rhi`), the Selene engine layer (`moonfield-render-core`), and the
scene-facing camera API (`moonfield-camera`) to provide mesh rendering by
default and optional 3D Gaussian splatting data, loading, and training support
through the `splat` feature.

It owns per-scene GPU data, feature-specific extraction and preparation, and
Core3d render phases — items queued into Selene's `RenderPhase<Opaque3d>` and
recorded by the registered `DrawMesh` draw function. Low-level Vulkan objects
remain in `moonfield-rhi`.

Part of the [moonfield](https://github.com/H5uan/moonfield) engine.
