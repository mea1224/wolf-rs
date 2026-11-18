# wolf-rs

当前实现与验证以 [2026-11-18 续作记录](docs/runtime-status-20261118.md) 为准。早期 [scene-media.md](docs/scene-media.md) 描述的是通用场景策略；播放器现已另外接入有原生证据的页激活、输入、路线、媒体及存档子集。

基于本地 WOLF 逆向证据建立的 Rust + wgpu 桌面运行时项目。当前可运行 **WOLF350 归档/松散资产 → 有界格式解码 → 带来源的 IR → 无窗口事件 VM → 宿主服务与确定性回放**，并有可实际呈现 PNG、网格移动预览和可选 WAV/OGG 音频的窗口程序；它仍不是完整可玩替代引擎。

默认目标为 Windows、Linux、macOS；目前仅在 Windows x86_64 MSVC、Rust 1.92.0 上验证。纯 Rust WOLF350 解码器已通过本地 **12 包、2,023 成员**对照。VM 实现调用、循环、变量、预算、等待、存档和回放；原生指令仍是有证据边界的子集。

### 窗口预览

在本目录构建 `cargo build --offline --locked --release -p wolf-player`。默认从真实归档和初始地图页进入游戏脚本，遇到尚未实现的可达语义会报告具体来源并停止。窗口逻辑分辨率来自 `Game.dat`，初始缩放与显示模式结合 `Game.ini`。本次无窗口集成已从新游戏进入玩家可操作的 Map001；镜子交互随后停于文字图片进度查询，尚未完成该地图转移。历史窗口对照见 [启动窗口对照](../docs-append/startup-window-comparison-report.md)，不能与本次无窗口测试混作同一次验收。

### WOLF350 归档

```powershell
cargo build --offline --locked -p wolf-cli
python tools/verify_archive_identity.py --archives ../Data --reference validation/resume-20261118/archive-reference-current --output validation/resume-20261118
```

库入口是 `wolf_assets::wolf350::Archive::open`、`entries` 和 `read`，返回精确成员字节并保留内层前缀，不写入文件。CLI 输出索引、成员 BLAKE3 与对照结果；省略 `--compare` 可只解码检查。比较需要独立参考目录，按归档名分目录；已生成的完整游戏资产不作为源码交付。参考 EXE 文件名是 v0.6.4，实际自报 v0.5.4 Build #106，身份和前缀处理详见 [归档验证](docs/wolf350.md)。

## 使用

默认启动位置从 `SysDatabase.dat` 读取，地图 ID 通过数据库地图表解析为路径。初始页采用逆序首匹配，随后启动该页的自动/并行入口。后续重选页、公共并行事件生命周期仍在完善；没有 `--mode similar` 选项。无窗口复核：

```powershell
cargo run --offline --locked -p wolf-cli -- inspect-startup --archives ../Data
```

该命令验证全部地图表资源并输出启动坐标与资产身份；当前得到地图 2、`MapData/Map002.mps`、位置 `(6, 6)`。播放器的默认入口使用该配置。`--input-probe` 单独验证真实键盘等待指令；`--media-probe` 单独执行 Common23 PC45..55 的 Logo/音效/告知图指令块，均不代表完整启动兼容。

实验性 VM 可用 `cargo run --offline --locked -p wolf-cli -- inspect-vm --archives ../Data` 检查全部事件；加入 `--run <program-id> --ticks 60` 可执行有限 tick 并输出状态摘要。`--host` 启用服务协调，`--host-actions <json>` 提供按 tick 编排的输入。支持范围和未完成项见 [VM 状态](docs/vm.md)及[宿主服务](docs/host-services.md)。

在 `wolf-rs/` 运行；资产路径必须显式提供，目录应包含 `MapData/*.mps` 和 `BasicData/CommonEvent.dat`。

```powershell
cargo run --offline --locked -p wolf-cli -- inspect-corpus --assets ../docs/evidence/data_copies
cargo run --offline --locked -p wolf-cli -- inspect-corpus --assets ../docs/evidence/data_copies --details
```

首次在新机器构建，先执行 `cargo fetch --locked` 下载锁定依赖。命令输出 JSON 至 stdout，错误至 stderr；不启动游戏。默认输出身份与计数；`--details` 输出含原始字符串和未知字段的完整 AST，会使用更多内存，也可能包含游戏文本。

当前归档 corpus：58 张地图（57 张 v103、1 张无事件 v100）、779 个地图事件、1,051 个页、160 个公共事件、81,924 条主事件命令。旧解包副本为81,879条，不能作为当前归档的逐字节基线。识别或路由命令结构不表示完整支持该 opcode 的全部形式。

## 验证

Python 3.10+ 仅用于验证脚本，不是运行时依赖：

```powershell
cargo test --workspace --offline --locked
cargo clippy --workspace --offline --locked --all-targets -- -D warnings
python tools/verify_vm.py --archives ../Data --corpus validation/current-startup-corpus.json --assets validation/resume-20261118/archive-reference-current --output validation/resume-20261118
python tools/verify_host.py --archives ../Data --output validation/resume-20261118
```

VM 验证会核对原始 AST 快照与当前归档的身份、完整 opcode 分布，以及可选独立解包目录的完整 AST；宿主路由数单独列出。旧 `docs/evidence/data_copies` 与当前资源版本不同，严格比较应失败。`verify_corpus.py` 与 `check_workspace.py` 仍用于旧证据的历史格式复核，不能当作当前归档验收。新机器需先生成匹配版本的独立参考和 raw corpus，详见 [归档验证](docs/wolf350.md)。游戏素材仅供本地验证，不随源码分发。

## 代码与文档

| 路径 | 当前内容 |
| --- | --- |
| [wolf-assets](crates/wolf-assets/src/lib.rs) | WOLF350/DXA8 解密解压、CP932 索引、松散挂载、路径/展开预算、BLAKE3 身份 |
| [wolf-format](crates/wolf-format/src/lib.rs) | 有界 reader、raw LZ4、v103 与有限 v100、CommonEvent CC AST |
| [wolf-ir](crates/wolf-ir/src/lib.rs) | 原始命令保留、显式 opcode/参数 lowering、来源、结构校验与局部陷阱 |
| [wolf-vm](crates/wolf-vm/src/lib.rs) | 确定性执行、原生键盘/调用/数据库/字符串子集、宿主接口；VM 存档 v11、回放 v3；原版存档另经播放器适配 |
| [wolf-cli](crates/wolf-cli/src/main.rs) | `inspect-corpus`、`verify-archive`、`inspect-vm`，有界运行及 JSON trace |
| [wolf-render](crates/wolf-render/src/lib.rs) | wgpu 30.0.1 texture-array/实例化 quad 管线与 WGSL |
| [wolf-media](crates/wolf-media/src/lib.rs) | RGBA合成/PCM解码；player已连接原生图片及单次SE子集、Windows视频解码，更多效果仍待实现 |
| [architecture.md](docs/architecture.md) | 实际依赖、所有权、平台/缓存边界、阶段验收 |
| [compatibility.md](docs/compatibility.md) | 已观察格式、支持范围与未证实语义 |
| [verification.md](docs/verification.md) | 复现命令、实测结果与验证限制 |

输入证据为根目录的 [引擎架构](../docs/wolf_engine_architecture.md)、[数据规范](../docs/data_formats_spec.md) 与 [原始实现计划](../docs/rust_wgpu_implementation_plan.md)。新实现统一放在本目录；参考库留在原证据目录中，不是当前 workspace 的成员。
