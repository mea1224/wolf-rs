# 宿主服务

`wolf_vm::host::Host` 在无窗口环境将 VM 请求与外部输入连接起来。它不依赖线程、操作系统时钟或 GPU。该模块实现新运行时服务协调策略，不是原版视觉/音频后端。

| 请求 | 当前行为 |
| --- | --- |
| Message | 保持等待，直到对应 token 的 Acknowledge |
| Choice | 检查 Select 的选项下标；仅允许有取消目标的 Cancel |
| Asset | 调用加载闭包一次，成功返回 Done，错误返回 Failed |
| Transition | 观察到请求后等待 `max(ticks, 1)` 个逻辑 tick；style 保留供渲染层使用 |
| Movement | 交给外部后端，通过 MovementFinished 或 Failed 明确回复 |

在每个 VM tick 之间调用 `poll(&vm, actions, loader)`，把 `Poll.completions` 送入下一个 VM tick，或作为 `replay::Input.completions` 记录。`requests()` 暴露尚未完成的 UI/后端请求。多个请求按 token 稳定排序，取消/页面替换后请求被清理；旧会话、非法选择和重复回复计入 rejected_actions。同一 tick 重复 poll 不推进时间，也不会重复加载资产；尚未被 VM 消费的回复会再次返回，调用者不能将这些重试回复拼接成一个重复批次。

加载闭包必须执行真实读取/验证并限制路径和字节数，例如调用 `LooseMount::read` 或 `Archive::read`。Host 只记载完成状态，不持有资产缓存；恢复后的渲染/资源后端须自行重建资产。移动命令的执行、碰撞和视觉转场仍由后端实现；消息文本中的控制码不会被解析，URL 不会被打开。

每次 poll 结束后可用 `host.save(&vm, cap)` 与 `vm.save()` 成对保存。Host 快照包含 VM 状态哈希、转场截止 tick 和待消费回复。先恢复 VM，再用 `Host::restore(bytes, &vm, cap)` 恢复并重绑定 token；状态或请求不匹配会拒绝。新建空 Host 会重新计时，不能替代配对恢复。序列化在写出时执行字节上限。错误文本最多 4096 字节且保持 UTF-8 边界。

## CLI

```powershell
cargo run --offline --locked -p wolf-cli -- inspect-vm --archives ../Data --run common/1 --ticks 60 --host
cargo run --offline --locked -p wolf-cli -- inspect-vm --assets <loose-root> --run <program-id> --ticks 60 --host-actions actions.json
```

脚本是 JSON 数组，tick 表示输入生效的 VM tick（从 1 起）。token 必须来自该执行的请求；以下仅为格式示例，不保证 common/1 会发出该消息：

```json
[{"tick":2,"actions":[{"Acknowledge":{"session":1,"sequence":1}}]}]
```

选择使用 `{"Select":[{"session":1,"sequence":2},0]}`。消息不会自动确认，移动不会自动成功。脚本限 1 MiB，tick 不得重复或越过运行上限，每 tick 最多 16384 个动作。无宿主参数时 CLI 保持原来的遇服务停止行为；启用后会等待到给定 tick 上限或所有 fiber 完成/故障。JSON trace 记录实际提交的服务完成与拒绝动作数量。归档模式目前只挂载 BasicData、MapData；其他资产可使用完整松散目录，缺失资产明确失败。

5 项宿主集成测试覆盖消息→资产→转场执行链、加载去重、过期 token、非法选择、取消、上限、资源失败，以及转场剩余时长与未消费回复的跨会话恢复。

`python tools/verify_host.py --archives ../Data` 另外运行真实 `MapData/Map000.mps/36/0`：没有确认时保持消息等待，tick 2 的确认使程序完成，同时拒绝旧会话和重复确认。结果写入 `validation/host-summary.json`，输入脚本保存在 `validation/host-actions.json`。这验证实际 opcode 101 与宿主连接，不验证窗口显示、控制码或原版时间。完整游戏画面、声音和原版时序尚未验证。
