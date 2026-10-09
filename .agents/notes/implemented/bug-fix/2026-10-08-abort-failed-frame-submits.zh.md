# Agent Note: Abort failed frame submits

Status: implemented

[English](2026-10-08-abort-failed-frame-submits.md)

## Problem

一次失败的帧提交会冻结所有已 acquire 图像的窗口。`end_frame` 在任何可失败
的 Vulkan 调用之前就消费了帧计划；调用方记下错误提前返回，跳过了已获取图
像的唯一消费者——present 循环。图像簿记被搁浅后，该窗口的下一次 acquire 会
跳过它，而 swapchain 重建又被"无持有图像"守卫挡住——一次瞬态的
`vkEndCommandBuffer` / `vkQueueSubmit2` 错误就让窗口停在最后一帧，且每帧
刷一条错误日志。搁浅状态还伴随两个隐患：acquire 留下的
`image_available[slot]` 已 signal 且无人消费（二进制信号量无法复位），而
`end` 失败可能让命令缓冲停留在 recording 态——`begin` 不接受该状态。

## Decision

`end_frame` 失败时，`submit_window_frames` 中止该帧。
`FrameContext::abort_frame` 复位该槽位的命令缓冲并放弃 uploader 中尚未提
交的批次（`FrameUploader::abort_frame` 复位该批次的缓冲并释放其暂存
arena）；帧号与槽位保持不变——timeline 值从未被 signal，重试帧会 signal
它。`WindowSurfaceData::abort_frame` 丢弃 acquire 簿记、重建该槽位的
`image_available` 信号量（acquire 返回时其 signal 已完成且无人等待它，旧
信号量可立即销毁），并标记 swapchain 待重建——退役 swapchain 的延迟销毁会
释放该图像。另一槽位的信号量可能仍被在途的上一帧等待，保持不动。下一
tick 经普通的 `create_window_surfaces` 路径重建 swapchain 并从中 present，
一次瞬态失败的代价是每窗口丢一帧。acquire 时"窗口仍持有图像"的守卫是不
变式检查，不是恢复路径。

## Alternatives considered

**中止时推进帧号。** 放弃：未被 signal 的 timeline 值本身可复用，推进反而
让下一帧落到仍在途的上一帧的槽位上，白白等待它。

**通过一次恢复性提交把图像 present 出去。** 放弃：这需要一个等待搁浅信号
量的新命令缓冲——比退役 swapchain 更重的机制，而且要在刚让提交失败的主
机条件下执行。

**swapchain 重建时重建全部每帧信号量。** 放弃：另一槽位的信号量可能承载
在途上一帧的 pending wait；只有搁浅槽位的信号量需要替换。

**重新暂存被放弃的上传批次。** 放弃：该批次的拷贝指向的 GPU 分配，其渲染
侧缓存（prepared meshes、池化 view targets）在暂存完成后即视为已交付；重
暂存信号正是渲染器刻意不携带的 retry queue（见
[Renderer aligned with Bevy](../architecture/2026-08-24-renderer-bevy-alignment.md)）。
uploader 提交侧的失败会放弃该批次；丢失的写入表现为 GPU 数据陈旧，直到
源资产的 revision 前进或目标被重建。

## Consequences

失败的提交在下一 tick 恢复，而不是卡死窗口；命令缓冲复位依托
`CommandBuffer::reset`（帧命令池创建时带 `RESET_COMMAND_BUFFER`）。纯离
屏帧不会搁浅窗口状态，维持其一帧丢弃的语义。放弃上传批次会丢失其暂存拷
贝，失败帧中暂存的 GPU 数据可能保持陈旧——这是不携带重暂存状态的已接受
代价；现实触发条件是主机资源耗尽。uploader 的中止覆盖其两种失败子情形
（批次仍在 recording，或已 end 但未提交）：无论哪种，批次都被丢弃，下一帧
重新暂存。
