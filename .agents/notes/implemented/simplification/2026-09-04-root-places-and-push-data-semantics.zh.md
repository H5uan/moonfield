# Agent Note: Per-draw root encoding without allocation, and push-data semantics corrected

Status: implemented

[English](2026-09-04-root-places-and-push-data-semantics.md)

## Problem

每个 draw 都要克隆管线的 `RootBinder`——两个 `Vec`（内含 `String` 名字）加一次
线性查找——只为写 8 字节（core 3D）或 24 字节（egui）；egui 那 24 字节里有
16 字节是帧常量，每个 mesh 都重推一遍。`Core3dFrame` 与 `RenderTargetSizes`
每帧深拷贝一次，以绕开 `World` 的借用——而内部可变性本来允许这些借用共存。
`Texture::bindless` 硬编码每像素 4 字节。此外 crate 的 push-data 文档声称
push data "与 push-constant bank 是同一块内存"——扩展规范并没有这句话。

## Decision

- `RootBinder::pointer_param`/`uniform_param` 在管线构建时一次性解析出
  `RootParamPlace`（offset、size、kind）。draw 时指针根在栈上编码
  （`RootParamPlace::pointer_bytes`）并按 place 的 offset 推送——零分配、
  零名字查找。
- `EguiRoot` 的变化字段（texture、sampler）挪到结构体尾部；pass 推送一次
  16 字节静态前缀，每个 draw 只在其 offset 推 8 字节尾部。尾位置与结构体
  大小在管线构建时双重守卫。
- `Core3dFrame` 与 `RenderTargetSizes` 改为借用而非克隆——不同资源的
  `Ref`/`RefMut` 可以共存。
- push-data 文档改为规范措辞：push data 是 descriptor-heap 管线的根数据
  接口，shader 经既有的 `PushConstant` storage class 读取；push constants
  依赖 set layout 状态、与 heap 管线不兼容（两类命令在同一命令缓冲上互相
  失效）。写入范围之外字节的持久性规范未明说——由
  `push_data_ranges_persist_across_writes` 做 GPU 验证（三个 range、
  uniform 最后写入、一次 dispatch 全部读到）。
- `Texture::bindless` 的上传校验改按 `Format::bytes_per_pixel` 计算。

## Alternatives considered

- **每 pass 克隆一次 root blob（而非每 draw）。** 每个 draw 仍有一次分配和
  一次拷贝；place 解析把最后这次分配也消掉了。
- **每 draw 推完整 `EguiRoot`。** 三倍字节量，帧常量每帧重推几百遍。

## Consequences

- `DrawMesh` 每 draw 的根数据工作是一次栈上编码加一次 `push_data`；egui
  是 8 字节加一个 scissor。`set_pointer`/`set_bytes` 保留为一次性 blob API
  （测试在用）；place 是热路径 API。
- 静态前缀模式押在 bank 持久性上，而规范没有写明——GPU 测试是守卫，清空
  未覆盖字节的驱动会让它响亮地失败。
