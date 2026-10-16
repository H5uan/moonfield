# Agent Note: Shader options and consumer ABI validation

Status: implemented

[English](2026-10-16-shader-options-and-consumer-abi.md)

## Problem

字节码和反射使用不同的 SPIR-V profile，反射也无法接收字节码编译使用的
capabilities 和宏定义。因此，受宏控制的 root 布局可能描述另一种 shader 变体。

Root 绑定校验地址的位置和部分存储大小，但若干消费方直接写入固定的 Rust
镜像。默认 shader 的布局测试无法保护运行时替换的 shader，不能阻止字段重排、
额外 root 参数或指针目标类型变化造成的不匹配。

## Decision

编译和反射共用 session 选项构建器。反射选项参与缓存标识。具有相同
capabilities 的 prepared 入口共享一个反射程序；消费方按入口选择反射，
而不是让所有管线阶段使用主入口所属的反射组。

消费方在构建管线时，使用实际入口参数或指针目标布局进行 ABI 校验。校验覆盖
root 名称及传递类型、完整字段列表及偏移、存储大小，以及图形阶段共享 root
的位置。内联数据录制在不分配内存的情况下校验字节长度。运行时契约见
[architecture.md](../../../../docs/architecture.md#assets)。

## Alternatives considered

- **将所有入口的 capabilities 合并到一个反射程序。** 拒绝：这样的 session
  与使用较少 capabilities 编译的入口不一致。分组既保留多入口共享反射，
  又不改变入口的编译选项。
- **只校验总大小。** 拒绝：字段重排可以保持总大小不变，却改变 shader
  读取的字节。指针地址也无法说明它指向的记录布局。
- **依赖默认 shader 的测试。** 拒绝：资产版本变化和应用提供的 shader 替换
  在运行时进入管线构建路径，测试无法枚举这些输入。

## Consequences

capabilities 不同的管线入口持有多个反射程序。主反射访问器只指向自身所属的
反射组；按入口访问会返回匹配的反射组或错误。未声明为编译入口的额外反射
入口使用空 capabilities。Prepared 管线使用空宏定义；RHI 的显式反射 API
接收与字节码编译相同的宏定义。

不兼容的宿主布局会使管线构建失败；对于原本在失败时保留管线的 pass，已有
有效管线继续保留。校验只保护存储布局，并非 shader 语义类型系统：相同大小
的标量替换和矩阵解释仍由宿主与 shader 的显式契约约束。

回归测试覆盖宏控制的布局、包含选项的缓存标识、反射分组、默认消费方布局、
字段重排、指针目标变化，以及无效的 root 和内联存储契约。
