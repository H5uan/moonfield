# Agent Note: RHI-owned interface and private backend modules

Status: proposed

[中文](2026-10-10-rhi-interface-and-backend-layering.zh.md)

## Problem

The [backend sub-crate decision](../../implemented/architecture/2026-10-09-rhi-backend-subcrates.md)
isolates backend dependencies and keeps downstream consumers on concrete RHI
types. The facade selects backend re-export lists, but does not define a common
interface that both implementations must satisfy. The Vulkan and Metal lists
in [the facade](../../../../crates/moonfield-rhi/src/lib.rs) differ, and matching
names do not guarantee matching behavior or signatures.

`Memory` illustrates the mismatch: in
[Vulkan](../../../../crates/moonfield-rhi/vulkan/src/memory.rs) it is an allocation
class (`Default`, `Gpu`, `Readback`); in
[Metal](../../../../crates/moonfield-rhi/metal/src/memory.rs) it owns a buffer.
`GpuAllocation` constructors, command-pool allocation methods, and command-buffer
`end` return types also differ. Upper layers cannot switch backends by
changing the facade feature alone.

Backend knowledge also reaches consumers: the render, editor, and ml manifests
enable `vulkan`, while
[shader preparation](../../../../crates/moonfield-render-feature/src/shader.rs)
chooses `ShaderTarget::Spirv`. Moving implementation files between crates and
modules does not resolve these contracts. Interface ownership and package
organization need separate decisions.

## Proposal

Make `moonfield-rhi` own the public interface and keep backend selection inside
its implementation. Prefer private backend modules once the common contracts
are established; retain backend sub-crates during that work so interface changes
can be verified independently of file relocation. This proposal does not replace
the implemented sub-crate decision until the migration is accepted and shipped.

The intended module responsibilities are:

```text
moonfield-rhi/src/
    lib.rs          explicit public exports
    types.rs        resource descriptions, memory classes, capabilities
    device.rs       public device interface and initialization
    command.rs      recording and submission contracts
    memory.rs       allocation and pointer contracts
    shared/         backend-independent implementation with proven reuse
    shader/         compilation, reflection, and caching
    backend/
        mod.rs      compile-time implementation selection
        vulkan/     private Vulkan implementation and conversions
        metal/      private Metal implementation and conversions
```

Public types such as `Device` and `GpuAllocation` wrap the selected concrete
backend implementation where adaptation or shared invariants are needed. Upper
layers keep concrete RHI types; neither backend generics nor trait objects become
a requirement for render, editor, or ml. Do not add forwarding wrappers to
already shared vocabulary or expose an internal interface solely for testing.

Define the common contracts before adapting implementations:

- Give memory classes and allocation objects distinct, backend-independent
  meanings. Specify host visibility, readback, ownership, and GPU-completion
  requirements rather than copying one backend's storage representation.
- Give command recording, submission, waiting, and reuse consistent signatures
  and ordering rules. Hide queue-family selection and native synchronization
  objects when the caller only needs a submission-completion token.
- Define shared `Stage`/`Access` semantics and root-data snapshot behavior. Each
  backend owns its native mapping. An unsupported operation must report an
  error or require an explicit capability check, not silently do nothing when
  the public contract promises an effect.
- Derive shader compilation targets from the selected device or backend. Keep
  explicit targets for offline compilation tools, not ordinary render setup.
- Put optional rendering capabilities in a shared capability description;
  backend choice alone must not imply that every device supports them.

For the platform-selected deployment described by the
[Metal proposal](2026-10-09-metal-4-backend.md), select Metal on macOS and Vulkan
on Windows/Linux inside the RHI through target-specific dependencies and `cfg`.
Consumers depend on `moonfield-rhi` without fixing a backend. Keep dependency
availability separate from implementation selection: Cargo features are
additive, so multiple consumers must not have to coordinate mutually exclusive
backend features. Runtime selection and same-platform backend overrides are not
requirements of this proposal.

Move backend-independent algorithms into `shared/` only when their contracts
are genuinely common. Allocation offsets, frame upload policy, and retirement
bookkeeping are candidates; native allocation, copies, and completion queries
stay backend-owned. Preserve the
[RHI ownership and public-type rules](../../../../crates/moonfield-rhi/AGENTS.md).
Sharing policy must not weaken resource lifetime or synchronization guarantees.

The [core crate](../../../../crates/moonfield-rhi/core/Cargo.toml) also links
Slang. Separating vocabulary from compilation is useful if a runtime only loads
precompiled shaders. In that case, make compilation optional; retain or extract
a compiler crate only if offline tools need to reuse it. Reducing crate count
alone is not a reason to merge that dependency into every runtime build.

Migration order:

1. Define and test memory, command, synchronization, and shader-target contracts
   at the public RHI interface while retaining backend sub-crates.
2. Adapt Vulkan and Metal to those contracts, centralize backend selection, and
   compile the same consumers against each implementation on its platform.
3. Reassess package isolation with the unified interface in place. If independent
   backend packages have no concrete consumer, move them to private modules and
   update the public-type checker, workspace members, CI, and owning docs.

## Alternatives considered

**Retain backend sub-crates behind an RHI-owned interface.** This preserves
independent compilation units, dependency restrictions, and in-crate GPU tests.
It is the migration starting point and remains a valid final layout if those
properties justify the extra cross-crate interface. Private modules are preferred
when backends remain internal implementations with no independent consumers.

**Merge crates but keep backend re-exports as the interface.** This reduces
manifests without resolving semantic differences or upper-layer backend
knowledge. File relocation alone does not meet the proposal's objective.

**Propagate `Device<B>` and backend traits through upper layers.** Static
polymorphism can enforce an implementation contract, but exposing it to every
consumer broadens the interface they must understand. Keep any necessary traits
internal unless consumers have a concrete need to be generic over backends.

**Introduce runtime trait-object or enum dispatch throughout the RHI.** This
supports selecting multiple backends in one binary, but adds dispatch and
cross-backend resource-consistency concerns for a capability the platform-selected
deployment does not require. Revisit it if same-platform runtime selection
becomes a requirement.

## Acceptance criteria

- Shared public types retain the same meaning, signatures, ownership rules, and
  observable behavior under both implementations. Optional capabilities have
  explicit support checks and documented failure behavior.
- Render and ml consumers compile against the selected platform backend without
  backend-specific imports, hardcoded Vulkan dependency features, or hardcoded
  runtime shader targets. The editor's remaining platform-specific integration
  work is tracked by the Metal proposal, not assumed complete by this refactor.
- One contract-test suite exercises allocation/readback, recording and reuse,
  submission completion, synchronization, and root-data snapshots through the
  public RHI interface. Run it on qualifying hardware for each backend; a
  hardware skip is not evidence of behavioral equivalence.
- Keep backend-local tests for native implementation invariants. Formatting,
  Clippy, Agent Note checks, the public-type checker, and platform build checks
  pass for the changed layout.
- If backends become private modules, selected-target dependency graphs exclude
  unselected native bindings. Workspace and CI commands reflect the new package
  layout, with no new backend types in the public interface.

## Risks

Private modules give up separate backend compilation units and crate-level
dependency and visibility restrictions. Target gating, private module
visibility, and checks must carry that responsibility; retain sub-crates if the
tradeoff is not worthwhile. A smaller package list does not imply faster builds.

Adapting Vulkan-shaped contracts can force expensive or incorrect emulation on
Metal. Define required behavior from actual render and ml use, then validate both
implementations. Memory-class and synchronization changes need particular care
because matching method names cannot establish GPU visibility or completion.

An RHI-owned public layer can become a collection of shallow forwarding wrappers.
Require each adaptation to normalize behavior, enforce an invariant, or hide a
backend-specific requirement. Share algorithms only after proving compatible
lifetimes and synchronization, not merely because source files look alike.

Platform-only selection limits same-platform testing and backend overrides.
Adding D3D12 or runtime fallback would require revisiting selection while keeping
the public resource and command contracts stable.
