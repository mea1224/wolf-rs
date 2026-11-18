# 本地验证与复现

## 当前检查入口（2026-11-18）

当前结果、复现命令和失败边界见 [续作记录](runtime-status-20261118.md) 与 `validation/resume-20261118/`。workspace测试400通过、15 ignored，Clippy通过；当前归档/VM/通用Host对照通过；镜子转移集成停在selector10，fmt检查仍报告45文件格式差异。各阶段结果分开记录，不用单一“通过”掩盖集成阻塞。

## 历史记录（2026-11-15）

以下统计与工具参数针对旧解包数据。旧原件哈希、81,879条语料和历史日志均不能直接用于当前归档验收；新独立基线生成方法见 [wolf350.md](wolf350.md)。

WOLF350 增量验证：12 包、2023 成员，2019 个精确匹配、4 个参考工具移除 62 字节前缀后匹配。详见 [解密器验证](wolf350.md) 和 [机器可读摘要](../validation/archive-summary.json)。下文原有内层格式 corpus 统计与归档层统计分开记录。

验证日期：2026-11-15。宿主 `x86_64-pc-windows-msvc`，`rustc 1.92.0`、`cargo 1.92.0`。依赖固定在 Cargo.lock：wgpu/naga 30.0.1、bytemuck 1.25.2、serde 1.0.229、serde_json 1.0.154、blake3 1.8.7、unicode-normalization 0.1.24。wgpu/naga manifest 声明 Rust 1.87；本项目只实测 1.92，不宣称整个 lockfile 的最低工具链已经验证。

## 命令

在 `wolf-rs/` 执行，Python 3.10+：

```powershell
python tools/check_workspace.py --assets ../docs/evidence/data_copies --evidence ../docs/evidence
```

等价关键检查：

```powershell
cargo fmt --all -- --check
cargo test --workspace --offline --locked
cargo clippy --workspace --offline --locked --all-targets -- -D warnings
python tools/verify_corpus.py --assets ../docs/evidence/data_copies --evidence ../docs/evidence
```

新机器先 `cargo fetch --locked`。工具脚本相对于自身定位 Cargo manifest 与输出目录；资产和证据是必需显式参数。构建产物位于本项目 `target/`，可重新生成，不作为交付源码。

## 单元检查

workspace 测试覆盖下列行为；精确数量与结果保存在 `validation/tests.log`，归档实物 corpus 测试默认 ignored，需另行显式运行：

| 模块 | 验证内容 |
| --- | --- |
| wolf-assets（9） | 路径/身份/尺寸、WOLF AES 独立两区向量、Huffman/DX LZ 截断/越界/重叠、错误 profile |
| wolf-format（4） | LZ4 literals/重叠回引/损坏与输出 cap、命令扩展与独立 route 空间/全前缀截断、AST/字符串预算、v100 无压缩/尾部/截断/未支持事件及版本拒绝 |
| wolf-ir（13） | 独立真实命令 fixture、原始记录/来源保留、分支/循环/标签与动态首匹配边界、局部结构 trap、表达式/负载限制、变量类型 |
| wolf-vm runtime（15） | 调用/循环/等待/预约、全局预算与轮转、过期/重复 token、故障原地保留、存档恢复/损坏拒绝、积压和资源边界 |
| wolf-vm native_control（9） | 111 七种比较/立即数/首匹配、continue/零循环、121 普通整数/饱和/零除数、小 clamp、commonself 共享/恢复 |
| wolf-vm replay（11） | 初始与逐 tick 哈希、跨会话、过期/重复完成、无效输入原子性、分歧定位、受限严格解码 |
| wolf-vm host（5） | 消息/选择/资产/转场、去重/错误、配对恢复和剩余等待 |
| wolf-vm scene / wolf-media | 显式页策略、网格碰撞/移动桥接；确定 RGBA 合成/PCM 混音；不验证 raw WOLF 映射 |
| wolf-render（1） | Naga 解析/验证 WGSL、CPU instance=56 B 与 camera=16 B |

shader 静态验证不创建 adapter、device 或 surface，不代表图像正确或设备能呈现。

## 真实 corpus 对照

`verify_corpus.py` 调用当前 Rust CLI 的 `--details`，与原探测证据 `data_maps.json/data_common_events.json/data_manifest.json` 对照。

1. 原始输入逐文件验证 SHA-256 和尺寸；既有重建副本验证证据里的 SHA-256。
2. Rust AST 的命令 start/end、opcode、args、缩进、原始字符串和扩展对照原 JSON。
3. 从 AST 的全部字段重新拼接仅用于验证的数据流，校验字符串、route、command、page、event、tile 范围；最后与原重建副本逐字节相等。未知字段和元数据也必须还原，不能只比较计数。
4. 精确比较整个 opcode histogram，记录输入证据与 Rust 源码/锁文件 SHA-256。重建哈希用于对接原证据，Rust CLI 的 BLAKE3 用于其资产身份。

| 指标 | 已匹配结果 |
| --- | ---: |
| 数据文件 | 59：58 张地图 + CommonEvent.dat |
| 地图版本 | 57×v103；1×v100 |
| 地图事件/页 | 779 / 1,051 |
| 地图/公共命令 | 16,347 / 65,532 |
| 公共事件 | 160 |
| 主命令总数/opcode 种类 | 81,879 / 62 |
| 移动路线命令 | 6,216 |
| 原始字符串 | 46,641 |
| 无 tile 地图 | 4 |
| 字段/重建流差异 | 0 |

结果文件：[corpus-summary.json](../validation/corpus-summary.json)、[corpus-comparison.json](../validation/corpus-comparison.json)。后者包含每文件的重建 SHA-256 和本次源文件身份，未保存完整 AST 或复制游戏数据。检查总结果与日志见 [checks.json](../validation/checks.json)、[tests.log](../validation/tests.log)、[clippy.log](../validation/clippy.log)、[corpus.log](../validation/corpus.log)。

## WOLF350 归档对照

```powershell
python tools/verify_archives.py --archives ../Data --reference validation/archive-reference
```

12 包共 2,023 个成员中，2,019 个与独立提取器逐字节一致；另外 4 个 BasicData `.dat` 仅在参考工具移除固定 62 字节前缀后相等。Rust 归档 API 保留该前缀，CLI 在内层格式边界单独处理。未发现其他差异；源码不依赖提取工具或原 DLL。参考目录需预先准备，并排除在源码分发之外。对照程序实际版本、SHA-256、解码 profile 和直接 API corpus 测试见 [wolf350.md](wolf350.md)。

## VM 与 Host 实物流程

```powershell
python tools/verify_vm.py --archives ../Data --assets ../docs/evidence/data_copies
python tools/verify_host.py --archives ../Data
```

VM 检查覆盖 1,211 个程序和全部 81,879 条命令，逐 opcode 核验 lowered/inert/unsupported 之和，不丢弃未知记录；整程序结构失败为 0。当前 33,377 条 lowered、23,719 条 inert、24,783 条 unsupported。归档与独立松散输入的覆盖和结构结果一致。

`common/0`、`common/1` 各重复两次，输出状态与 trace 完全相等。`common/0` 在 tick 1 结束；`common/1` 在 tick 1 执行 9 条后，于 PC 82 的未知 opcode 111 参数形式暂停，并保留文件、事件、PC 与范围。此测试检验 Rust 确定性和错误定位，不是原作行为对照或可玩证明。每次运行记录源码 SHA-256，结果见 [vm-verification.json](../validation/vm-verification.json)。

Host 验证直接读取 `MapData.wolf` 的 `Map000.mps/36/0`：没有输入时消息连续 3 tick 保持等待；tick 2 有效确认后结束；一个过期会话和一个重复确认被拒绝，只产生一次完成。结果见 [host-summary.json](../validation/host-summary.json)。

## 验证限制

未启动 GamePro.exe、未执行或替换 DLL。归档验证读取原包，格式验证读取原有解包/重建证据，原生语义来自静态分析。两个二进制原件身份见 [binary-identities.json](../validation/binary-identities.json)。本轮 dispatcher/handler 表、变量 helper、文本与优化路径的限制见 [native-vm-findings.md](../validation/native/native-vm-findings.md)；REA bundle 已保存，owned session 已关闭并 reset。

已有 Rust 存档/回放和精确 CPU 媒体测试，尚无原作执行 trace/原 `.sav`、GPU 截图或 Linux/macOS 宿主验证。它们不建立视觉、音频、原版调度或整游戏兼容。验证用拼接器不是生产编码器。新的解析 profile/opcode schema 仍需独立 fixture 与错误用例。
