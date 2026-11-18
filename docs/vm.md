# 事件 VM：实现、原生映射与兼容边界

## 当前状态（2026-11-18）

当前 IR profile 为 `wolf350-evidence-subset-v11`，VM 存档为 v11，回放为 v3。当前 81,924 条命令中，41,280 条降低为 VM 操作、16,406 条路由到 NativeHost、23,729 条 inert、509 条 unsupported；结构失败为0。NativeHost 是 `lowered` 的子集，路由成功不证明参数语义已实现。

当前独立解包输入与归档 AST/覆盖比较通过。新游戏集成已到达 Map001 玩家接管并触发镜子事件，但停在 Common37 PC249 的图片 selector10，尚未完成转移。完整数据、日志、复现命令、存档/调度续作和限制见 [当前续作记录](runtime-status-20261118.md)。

## 历史说明：2026-11-15 的 v2 实现

以下保留当时的映射和验收边界。其版本、未实现项、旧语料数及命令路径不能作为当前状态；特别是数据库、字符串、原生 RNG、页触发、路线与原生公共预约已有后续实现。旧 loose 目录不适合作为当前归档基线。

`wolf-ir` profile 为 `wolf350-evidence-subset-v2`，保留每条 RawCommand、文件内容身份、event/page、原始命令序号及重建流范围。`wolf-vm::runtime` 已实现调用栈、循环、分支、独立变量空间、字符串 IR、tick 等待、服务 token、全局预算、轮转调度、热循环诊断、新格式存档和恢复。合成 IR 的 RNG/算术是明确的 Rust 策略；原生指令只降低已识别参数子集，不能将机制测试视为原版行为对照。

当前不是完整 WOLF 游戏 VM。原版页面激活、移动/碰撞、音频/图片等宿主服务以及多个 opcode 尚未实现或校准。可达 unsupported 会保留 PC 并产生带来源的错误。新增 `wolf_vm::host` 管理消息、选项、资产和按 tick 推进的转场；移动完成由调用者显式通知，尚不负责角色寻路/碰撞。服务时序属于 Rust 策略，尚未校准原版。CLI 默认仍在服务请求处停下；`--host` 开启 headless 宿主，`--host-actions <json>` 提供按 tick 索引的用户输入。用法与配对存档见[宿主服务说明](host-services.md)。

`wolf_vm::replay` 增加了确定性验证：记录初始存档、逐 tick 的变量写入与服务完成，并在每个 tick 后校验状态哈希。它比较的是当前 Rust 运行时策略。字节/frame 上限、会话 token 重映射、失败输入处理及代码示例见[回放说明](replay.md)。

## 真实语料验证

```powershell
cargo run --offline --locked -p wolf-cli -- inspect-vm --archives ../Data
python tools/verify_vm.py --archives ../Data --assets ../docs/evidence/data_copies
python tools/verify_host.py --archives ../Data
cargo test --offline --locked -p wolf-vm
```

本地 58 张地图和 CommonEvent 共 1,211 个程序、81,879 条命令全部进入库和覆盖统计。2026-11-15 本轮结果为 33,377 条非空操作（lowered）、23,719 条 Nop（inert）、24,783 条 unsupported；supported 是前两项合计 57,096。整程序结构失败为 0。每个 opcode 的分项计数见 [vm-corpus.json](../validation/vm-corpus.json)。这些是参数形式覆盖数，包含尚未与原版轨迹校准的映射，不是游戏行为正确率。

编译器逐命令记录问题，不因单个重复标签、未知指令或异常结束标记丢弃整个程序。普通标签扫描的首匹配顺序已有静态证据；文本展开、跨循环记账以及未解析标签仍保留陷阱。unsupported 被执行时保留 PC，附带 asset/event/page/command/span。无论分析多少后续记录，都不会把未知可达命令自动视为空操作。

归档与独立松散目录得到相同覆盖/结构结果。`common/0`、`common/1` 各运行两次，Rust trace 相同；后者在 tick 1 执行 9 条指令后，于原始 PC 82 遇到未知参数形式的 opcode 111 停止，保留来源。原 PC 58 的带参数 opcode 99 已依据 dispatcher 证据识别为空操作。这是正确报告阻塞的测试，尚未推进到可玩场景。另有真实 `MapData/Map000.mps/36/0` 的消息等待/确认测试：无输入保持等待，tick 2 确认后完成，旧会话与重复确认被拒绝。结果分别见 [vm-verification.json](../validation/vm-verification.json) 和 [host-summary.json](../validation/host-summary.json)。

## 原生指令映射

证据来自本地 GamePro.exe 的静态分派/handler 表和反汇编，完整地址、原件 SHA-256、限制及 REA ID 见 [native-vm-findings.md](../validation/native/native-vm-findings.md)。REA 提供的 PE 节/身份记录不是动态执行证明；语义推断另由本地反汇编和跳转表支撑。

| Opcode | 已降低的参数形式/行为 | 未闭合边界 |
|---|---|---|
| 0、99、103 | 普通 dispatcher 直接进入下一条，不读取参数/文本；带参数 99 也为空操作 | 未知扩展、路线及 terminator 仍拒绝；预编译阶段未完整追踪 |
| 101 | 单字符串消息请求，等待 Host 确认 | 字符控制码、字体和窗口未实现 |
| 111 | 1–4 条数值条件，首匹配；模式 0..6 为 `>`、`>=`、`==`、`<=`、`<`、`!=`、位包含；0x10 强制 RHS 立即数 | 未知高位、字符串/数据库地址拒绝 |
| 121 | 四参数普通整数路径；binary/update/立即数 flags 按静态表解释 | RNG、double、范围、间接及优化路径选择未闭合 |
| 170、179、498 | 有限大循环/计数循环/末尾递减；计数 <=0 跳过 body | 引擎优化记录、非普通参数形式 |
| 171、176、172 | break、continue、return；continue 会经过当前循环的递减 | 整体原版调度与调用退出时序 |
| 180 | 一参数 tick 等待 | 零等待至少一个 tick 属 Rust 策略，原版当帧规则未校准 |
| 210、211、300 | 零参数静态 common 调用/预约/名称调用 | 参数元数据绑定、返回值、动态目标与原生预约调度 |
| 212、213 | 标签空操作；字面标签首匹配且不跨循环的静态跳转 | 文本展开、目标前动态/未知标签、跨循环栈调整、预编译路径；缺标签仍拒绝 |
| 401、402、420、421、499 | 结构分支出口；499 为 branch-state reset，在已编译 CFG 中空操作 | malformed 参数仍局部 trap；不能绕过不支持的 opener |
| 112、122 | 已记录字符串条件/set/append 静态结构 | 原生取值、展开和编码转换未闭合，因此尚不降低 |
| 其余 | 完整保留 raw 参数/字符串/扩展和来源 | 由明确 unsupported 终止受影响 fiber |

121 的普通路径使用 i64 中间值，将 +/−/× 限制在 ±2,000,000,000；零除数改为 1；`INT_MIN / -1` 显式 fault；有符号移位取低 5 位幅度；flags bit0 最终夹至 ±999999。binary 表在 `A10F60`，update 表在 `A10F88`。其优化路径在边界上不同，当前 `Arithmetic::WolfInteger` 仅代表普通整数规则。合成 IR 使用 `Arithmetic::Wrapping`，两者明确区分；不能把自定义 xorshift RNG 当原生 RNG。

## 变量与调用上下文

| 原始地址 | 当前表示 | 边界 |
|---|---|---|
| 小于 1,000,000 的有符号值 | 立即数 | 121/111 强制立即数 flag 可覆盖此判定 |
| 1,100,000..1,199,999 | `Event(raw % 10)`，使用 fiber owner | owner 由宿主维持当前地图事件身份；跨地图/绝对 mapself 尚无适配 |
| 1,600,000..1,699,999 | `Common(common_id*100 + raw%100)` | 只在 common 源程序绑定；slot 5..9 是字符串，numeric 拒绝 |
| 2,000,000..2,999,999 | `Global(raw−2,000,000)` | 原生银行/索引扁平化 |
| 9,000,000..9,099,999 | `System(raw−9,000,000)` | 原生为动态 getter；必须由宿主显式注入，缺值/指令写入报错 |
| 15,000,000..15,999,999 | `Common(raw−15,000,000)` | slot 5..9 拒绝 numeric |
| 其他 | unsupported 地址 | 数据库、绝对 mapself、实体状态、原生 RNG、字符串转换需专用适配 |

commonself 在调用之间共享，不是新建的参数帧；`Local` 只供手写 IR 的帧参数使用。210 common 调用继承 caller 的 map event owner，并切换自身 common ID。Rust 对尚未分配的普通槽位采用零值/按需写入策略；原生 common 数组的预分配、初值和销毁边界尚未完整校准。common 参数到槽位的布局依赖事件 metadata，当前非空 native 参数调用会 trap。

## 调度、存档与服务边界

每 tick 共享指令预算，按稳定 fiber ID 轮转，预算用尽让出且保留进度。预约在下一 tick 启动；重复耗尽报告热循环。`FixedClock` 用整数累加并保留追赶积压；60 Hz 是设计默认，不是测得的原版频率。表达式深度、指令数、栈、字符串/槽位、完成批次和输出存档均有限制。

请求发出一次、推进 PC 后进入 typed wait；token=(session,sequence)。换页/取消清理旧请求；读档切换 session。无效、重复、类型不符的 completion 不能唤醒等待。Host 负责消息/选择/资源/转场；显式场景策略与离线媒体见 [scene-media.md](scene-media.md)，它们尚未完成 WOLF raw page/route/媒体指令适配。

VM 存档 **v2** 新增独立 persistent common 空间，拒绝旧 v1，不提供猜测迁移。恢复验证库身份、PC/调用栈/循环、变量、等待与请求一致性。Host 需成对保存；场景/移动/媒体状态不在 VM/Host 存档内。回放 **v2** 校验初始及每 tick 的规范哈希，排除传输 session，保留逻辑请求序号；仅记录 globals/systems/completions，中途 spawn/cancel/换页尚未进入输入格式。详见 [replay.md](replay.md)。

后续完整兼容仍需：数据库/配置与参数元数据、字符串展开/转换、原生 RNG/优化算术、全部可达 gameplay opcode、raw 页条件和路线、媒体/渲染后端及原版执行轨迹对照。原作 trace、完整 opcode 映射和 Linux/macOS 宿主一致性均未验证。
