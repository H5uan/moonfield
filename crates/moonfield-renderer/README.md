# Lunaris

Moonfield's scene rendering & algorithms crate — the unified home for every high-level rendering technique that sits on top of the Lunar Mare RHI (`moonfield-render`): 3D Gaussian splatting (`splat`), and future ray tracing (`rt`) and global illumination (`gi`).

Roughly the role of Unreal's `Renderer` module or Godot's `RenderingServer`: it owns per-scene GPU data, frame orchestration (a simplified Bevy-style extract → prepare → render phase abstraction), and shared compute utilities, while the low-level Vulkan objects stay in the RHI below.

Part of the [moonfield](https://github.com/H5uan/moonfield) engine.
