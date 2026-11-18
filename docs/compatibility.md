# 兼容范围与未知项

## 当前状态（2026-11-18）

当前实现/验收见 [VM 与 WOLF350 续作记录](runtime-status-20261118.md)。当前语料为81,924条命令；IR profile v11，VM存档v11，通用回放v3。已有数据库/Game/TileSet parser、原生页条件/触发、路线/媒体、窗口及原版存档的部分适配。400项测试与Clippy通过；完整镜子转移仍被文字显示进度查询阻塞，不能称为完整替代引擎。

## 历史兼容清单（2026-11-15）

下表保留早期阶段的支持边界和证据更正。表中“未实现/未连接”、旧VM计数和版本均属于当时状态，不覆盖后续实现；当前支持范围仍须按具体参数与测试判断。

## 已实现的格式配置

当前 `wolf-loose-map100-map103-commonCC-v1` 为内层格式配置，不代表所有 WOLF 版本。`inspect-corpus` 使用已解包目录；独立的 `dxa8-wolf350-v1` 解密器和 `verify-archive` 可读取本地 `.wolf` 包，`inspect-vm --archives` 可直接检查 BasicData/MapData。各工具不会把已重建流当作压缩原文件再读。

| 输入 | 支持范围 | 拒绝/未知范围 |
| --- | --- | --- |
| loose assets | 显式根目录、精确 Unicode 名称、规范分隔符、内容身份 | WOLF 查找兼容规则、overlay/缓存未实现；CP932 原始名由 Archive 层保留 |
| `.mps` v103 | 本地 57 张：25 B 头、raw LZ4、显式三层、事件/页/命令、精确 EOF | 其他版本不自动套用；tile ID、x/y 展开语义未验证 |
| `.mps` v100 | 本地 SampleMap：未压缩、无 v103 两字段、隐含三层、零事件 | 有事件的 v100 明确拒绝；旧版命令扩展布局未在该样本验证 |
| no-tile | `Option::None`，保留 -1 哨兵范围 | 不能推断无事件、无碰撞或无角色 |
| CommonEvent CC | 本地 160 个事件、全部 metadata/不透明字段、0x92 尾部、精确 EOF | 其他 profile、未知 marker 明确报错 |
| 主事件命令 | 显式 u32 opcode、i32 参数、原始字符串、缩进、terminator、移动数据、扩展长度；部分参数形式可编译执行 | raw 结构保真不表示全部 opcode 语义已实现，见 [VM 映射](vm.md) |
| route 命令 | 独立 u8 opcode/args/01 00、独立 span | 不与主事件 opcode 合并 |
| 数据库/TileSetData/Game/MapTree | 原证据有部分结构线索 | 尚无当前 Rust parser；不以空数据代替 |
| WOLF350 DXA v8 | Rust 原生解密、CP932 名称、完整/分段 Huffman 与 DX LZ；12 包 2023 成员经独立工具对照 | 仅 flags=0x015e0000；小于 1088 B、其他加密配置拒绝；通用 WOLF、查找兼容规则、跨平台宿主运行未验证 |
| 实验性事件 VM | IR 来源、tick/fiber/调用/等待 token/预算/存档/回放/Host；1211 程序、81879 命令完整保留，0 整程序失败 | 当前 33377 lowered、23719 inert、24783 unsupported；数值/循环部分有静态证据，原版整体轨迹未对照，非完整游戏 VM |

原始 AST 保留有符号 tile words、unknown blobs、字符串末尾 NUL 和重建偏移。严格 UTF-8 只对应当前 profile；错误文本不经替换字符悄然修复。extension 长度即使为零也必须读取并保留。

## 语义与运行状态

| 能力 | 状态/缺口 |
| --- | --- |
| 事件解释与 World | 已有实验性 IR 执行器与变量世界；完整游戏语义未达成，见 [VM 状态](vm.md) |
| 参数寻址/变量/RNG/算术 | 已确认部分寻址和 121 普通整数路径；commonself 跨调用共享。RNG、优化路径、double、字符串/数据库寻址未闭合 |
| 页条件/触发/并行调度 | 已有显式 Rust 场景策略；raw 37 B 页条件、trigger 与原版调度尚未适配 |
| 分支/标签/公共调用 | 111 首匹配/比较表、循环、213 字面首标签；跨循环/文本展开与 common 参数元数据仍拒绝 |
| 原循环 | 已定位静态 dispatcher `A090C0` 与数个 handler；不据此声称已测得原版每帧时序 |
| tick | 暂定 60 Hz 的架构设计，未测得原作频率/当帧扣减规则 |
| TileResolver/自动元件/碰撞 | raw tile/TileSetData 适配未实现；另有显式网格碰撞/移动策略，未与原作校准 |
| renderer | 实例布局、管线与 WGSL 通过编译/静态校验；无真实 GPU 呈现/截图对照 |
| 音频/字体/窗口/Steam | 离线 RGBA/PCM 输出可用；codec/设备/原生媒体指令、字体和窗口未连接。DLL 线索不等于 MIDI 后端 |
| 存档/回放 | VM 存档 v2、Host 配对恢复与回放 v2；不支持旧 v1/原 `.sav`，场景/移动/媒体状态尚未纳入 |
| Windows | 已完成本机 Rust workspace 检查；无图形后端运行验证 |
| Linux/macOS | 为设计目标；未运行宿主构建或图形/输入/音频验证 |

后续每个语义实现必须携带真实命令 fixture、参数解释来源及 tick trace。遇到可达未知 opcode 应暂停并给出资产/event/page/PC/span；不得用 NOP 掩盖。原始游戏输入和本地验证副本均继续作为只读来源。

## 证据纠正

此前三份报告把 58 张地图统称为 v103。2026-11-15 的 Rust 验证在 SampleMap 上明确拒绝版本，复核原 JSON 和 51 B 原文件后确认实际为 **57×v103 + 1×v100**。三份报告已更正，Rust 新增有限 v100 分支及截断/尾部/未支持事件测试。

结构对照使用原探测脚本的独立输出和解压 SHA-256；两套实现仍参考同一份格式来源，不能证明未知字段的语义。历史取证状态见 [原清理记录](../../docs/evidence/rea_final_cleanup.json)。本轮新增静态 VM 分析和 [REA 证据包](../validation/native/rea-vm-evidence-bundle.json)，owned session 已关闭并 reset 保存；语义结论见 [原生分析](../validation/native/native-vm-findings.md)，不作为动态观察。
