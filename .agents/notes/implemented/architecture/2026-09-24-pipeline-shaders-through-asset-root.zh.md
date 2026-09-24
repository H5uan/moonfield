# Agent Note: Pipeline shaders load through the asset pipeline, one reflection per pipeline

Status: implemented

[English](2026-09-24-pipeline-shaders-through-asset-root.md)

## Problem

三个缺陷同根同源:shader 资产管线已经存在,但并非所有管线都在使用它。

1. splat 排序 pass 用 `include_str!` 嵌入 `radix_sort.slang`,
   挂一个假的路径标签(`crates/moonfield-render-feature/src/splat/sort_pass.rs`),
   绕过了 `AssetServer` 加载、路径去重、revision 跟踪,以及 mesh 和
   egui 管线走的 `PipelineShaders` → extract → `prepare_shaders` 流程。
   同一份文件曾有三种加载路径(两处 `include_str!`,一处 asset-server 测试)。
2. `PreparedShaders` 链接 reflection 程序时只包含模块与请求的
   `reflect_entry`,因此 reflection 只能回答那一个 entry 的 root-binding
   查询。多 entry 管线——radix sort 的
   `histogram`/`scan`/`scatter` 三个 compute entry,各自带独立的
   `uniform Params` root blob——无法从一个 prepared shader 完成绑定。
3. 每个在运行时或测试中需要仓库资产的消费方都手写
   `env!("CARGO_MANIFEST_DIR")` + `"../../assets"`——编辑器的 shader 加载与
   默认 mesh、四个集成测试、三个 rhi reflection 探针——且没有为在源码
   checkout 之外运行的二进制提供覆盖手段。

## Decision

- `moonfield-rhi` 每个管线链接一个程序:
  `Compiler::compile_source_to_reflection` 与
  `ShaderCache::compile_source_reflection` 接受 `entry_points: &[&str]`,
  构造"模块 + 全部所列 entry"的 composite
  (`ISession::createCompositeComponentType` 对 entry point 取并集),
  因此一个 `Reflection` 能对每个 entry 回答逐 entry 查询——
  `root_parameters`、线程组尺寸。单元素切片与旧的单 entry 链接完全等价。
- `PreparedShaders::compile_request` 对请求声明的每个 entry 做 reflection
  (当请求把 `reflect_entry` 放在 `entries` 之外时也一并纳入,
  保证任何合法请求编译的内容不少于从前)。
- `RadixSort` 新增 `from_prepared`——从 `PreparedShader` 构建三条管线与
  root placement——`new` 共用同一构建器,编译一个多 entry reflection
  而非三个。
- splat 排序 pass 像 `core_3d_shader` 一样声明 `SPLAT_SORT_SHADER` /
  `splat_sort_shader(handle)`;`prepare_splat_sort` 遵循
  `prepare_core_3d_pipeline` 的形状:请求或 prepared shader 缺失时以
  one-shot 日志跳过,prepared revision 前进时重建。生产代码中不再有
  `include_str!`;验收测试通过 `AssetServer` + `SlangLoader` 注册请求,
  与 `tests/radix_sort.rs` 同形。
- 编辑器在 `splat` feature 下加载 `util/radix_sort.slang`
  (与 `core_3d.slang`、`egui.slang` 并列);插件本身仍是可选加入——
  发行版编辑器不运行合成排序。
- `moonfield_asset::assets_dir()` 是仓库资产根的唯一定义:设置了
  `MOONFIELD_ASSETS_DIR` 环境变量时用它,否则用按 `moonfield-asset`
  在 workspace 中的位置编译进二进制的路径。编辑器、集成测试与
  rhi 探针(import 解析用的虚拟模块名)全部经它解析;`moonfield-rhi`
  以仅测试的 dev-dependency 引入 `moonfield-asset` 供探针使用。
  清理之后,`assets_dir()` 之外不再有任何 `env!("CARGO_MANIFEST_DIR")`
  资产引用或指向 shader 树的 `include_str!`。

## Alternatives considered

- **沿用现有单 entry API 做 per-entry reflection**(每个 entry 链接一次
  模块,保留 `&str` 签名):否决——同一管线要把同一模块编译链接三次,
  且偏离 Slang 的本意:程序的 reflection 本应覆盖其全部 entry point。
- **在排序 pass 保留 `include_str!`,仅去重常量**:否决——嵌入仍然绕过
  revision 跟踪与 prepare 流程,shader 改动后要重启进程排序才会重建。
- **在 rhi 编译器中加入 shader 搜索路径 / include 根机制**,支持用户在
  shader 目录之外做 `import`:暂缓——探针的虚拟模块名仍依赖仓库布局,
  但那是 import 解析整体上的已知欠账;本次改动不触碰它。
- **把资产树嵌入发行二进制**:暂缓——`MOONFIELD_ASSETS_DIR` 已是可部署
  的逃生门;嵌入是打包层面的决策,落地时另写笔记。

## Consequences

- `ShaderCache` 的 reflection 缓存键以 `','` 连接 entry 名——无损,
  因为 Slang entry-point 名是标识符。
- 每管线一个链接程序意味着全局作用域 shader 参数只布局一次、跨 entry
  共享;当前内置管线都没有声明全局作用域参数,行为不变。
- `moonfield-rhi` 新增了对 `moonfield-asset`(零依赖叶子)的
  `[dev-dependencies]` 条目——仅测试,不影响 rhi 的对外依赖图与
  边界检查。
- 编辑器默认加载两个 shader,开启 `splat` 后为三个;pipeline-shader
  测试按 feature 组合断言数量。
- 本地验证:`moonfield-render-feature` 在 `splat` 下全部 42 个测试通过
  (含 GPU 验收测试,现在经 prepared 资产完成排序);`moonfield-rhi`
  49 个测试、`moonfield-ml`、`moonfield-editor`(`splat`)通过;
  `cargo clippy --workspace --all-targets -- -D warnings`
  在开与不开 `splat` feature 时均干净。
