# VM / WOLF350 续作验证：2026-11-18

本记录针对当前项目和当前资源，不宣称完整 WOLF VM 或所有 WOLF350 版本兼容。实现全部位于 `wolf-rs/`；本轮没有启动 GamePro.exe、替换原程序/DLL，或将集成测试存档写入用户 Save。平台目标仍为 Windows、Linux、macOS，本轮实际检查仅覆盖 Windows x86_64 MSVC。

## 归档与资源身份

纯 Rust `wolf-assets/src/wolf350/` 已实现当前 profile 的地址解混淆、密钥派生、外层 XOR、WOLF AES 变体、Huffman/DX LZ、CP932 目录和有界成员读取。运行时不依赖提取器或 GuruguruSMF4.dll。

12 个当前归档的 SHA-256 均已不同于 11 月 15 日基线，故重新使用固定哈希的独立提取器对副本解包。当前结果为 **12 包 / 2,023 成员：2,019 完全一致，4 个仅有已知 62 字节保护前缀差异**。Rust API 保留该前缀。原归档检查前后哈希一致。新旧成员内容仅 `CommonEvent.dat`、`Game.dat`、`Map028.mps` 三处变化，无增删。

报告：[archive-current-summary.json](../validation/resume-20261118/archive-current-summary.json)。复现命令与限定 profile 见 [wolf350.md](wolf350.md)。旧独立解包目录继续保留作历史证据。

## VM 覆盖的实际含义

当前 IR profile 为 `wolf350-evidence-subset-v11`，VM save version 为 11，通用 replay version 为 3。版本含义不同，不互相替代；播放器的完整场景/服务快照另有封装。

当前 raw corpus 有 58 张地图、160 个公共事件、1,211 个程序和 81,924 条主事件命令，62 个 opcode。逐命令分类为：

| 分类 | 条数 | 含义 |
| --- | ---: | --- |
| VM lowering（不含 NativeHost） | 41,280 | 编译为 VM 操作，尚不等于原版轨迹一致 |
| NativeHost 路由 | 16,406 | 已交由原生宿主适配；具体参数在执行时仍可能 fault |
| inert | 23,729 | 已识别的空操作/结构记录 |
| unsupported | 509 | 当前不降低的参数形式或指令 |
| 整程序结构失败 | 0 | 未因单条未知指令丢弃整程序 |

兼容旧接口的 `coverage.lowered=57,686` **包含** `native_host=16,406`，不能再次相加；`supported=81,415` 也只是编译器分类。新增 `Coverage.native_host` 与每 opcode 的同名字段，防止把宿主转发误读成全部语义支持。

尚未降低的命令分布：102×5、105×6、111×1、112×1、121×135、123×3、173×36、174×5、177×1、178×1、210×27、213×37、230×5、231×2、240×39、241×1、242×35、255×6、260×1、280×158、300×4。此静态清单不能证明可达性；宿主未覆盖的形式不计入这 509 条。

`verify_vm.py` 从 raw AST 独立统计程序与 opcode 分布，核对快照归档/成员身份，逐字段比较当前独立解包输入，并验证重复执行结果。raw AST 仍由本项目解析器产生，不能冒充另一套格式解析器。旧解包输入在 CommonEvent AST 比较处正确拒绝。源码、锁文件、CLI 和输入分别记录哈希，检查过程中不允许变化。

报告：[vm-corpus.json](../validation/resume-20261118/vm-corpus.json)、[vm-verification.json](../validation/resume-20261118/vm-verification.json)。通用 CLI 的 `common/0` 正常结束；`common/1` 在 PC106 报 `native host service is not connected`，这与播放器已连接的原生服务是不同执行环境。`verify_host.py` 验证的是通用消息 Host：无输入保持等待、tick2 确认、过期/重复完成被拒绝，见 [host-summary.json](../validation/resume-20261118/host-summary.json)。

## 实际玩家接管与当前阻塞

当前代码已包含数据库/字符串/原生 RNG、公共参数调用与预约、按地图保存的变量上下文、页重选、玩家输入、路线/碰撞、图片/音频及存档子集。模块边界和原生证据见 [native-map-event-triggers.md](native-map-event-triggers.md)、[native-route-continuation.md](native-route-continuation.md)、[native-save-payload.md](native-save-payload.md)。这些实现并不免除参数边界检查。

新确认的原生初始化 `9ECDE0..9ECDED` 从 `FA523C`（Game.dat configuration[9]）设置玩家 actor+0xB5。当前资源该配置为 1；玩家 RouteState 已接入该高度，地图角色仍由页 flag0x20 决定。正常确认查询 9F6900 对横向交互使用这一高度，解决了整格玩家无法触及奇数半格事件的问题；无需地图或事件 ID 特例。

合成回归 `configured_player_height_reaches_an_odd_half_grid_event_across_a_wall` 通过。真实 release 测试从原始启动配置、新游戏开始，抵达 Map001 玩家控制位置 `(32,18,6)`，通过输入路线 `[8,6]` 触发 event3/page0。测试使用新建的隔离 Save 根目录，没有恢复旧 checkpoint。

此测试**尚未通过**：在 tick1840，CommonEvent37 PC249（重建流 `0x5643d`）的 opcode124 `[1600035,16384,1600034,10]` 报 `unsupported native picture query`。玩家可正常接触镜子，但此后的文字显示等待和地图转移未完成。完整日志：[transfer-current.log](../validation/resume-20261118/transfer-current.log)。不得将“镜子已触发”写成“转场已完成”。

selector10 的补证见 [native-text-progress-evidence.md](native-text-progress-evidence.md)。它需要独立的文字显示进度、控制字节补偿和存档状态；现有 `offset_effect` 不承载这些数据。当前保留带来源的 fault，没有把查询硬编码为完成。

## 本轮检查

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace --offline --locked` | 400 passed，0 failed，15 ignored；ignored 不算验收通过 |
| `cargo clippy --workspace --offline --locked --all-targets -- -D warnings` | 通过 |
| 当前归档独立逐成员对照 | 通过；限定本次 12 包 |
| 当前 VM accounting / AST / 重复轨迹 | 通过；不建立原版执行等价 |
| 通用 Host 等待、确认和过期 token | 通过 |
| 新游戏→玩家接管→镜子→跨地图 release 集成 | 未通过；上述 selector10 fault |
| `cargo fmt --all -- --check` | 未通过：45 个现有源码文件有格式差异；未进行全仓库格式重写 |

日志均保存在 `validation/resume-20261118/`，汇总见 [checks-current.json](../validation/resume-20261118/checks-current.json)。该汇总诚实保留 `all_checks_passed=false`；源码哈希是检查后的快照，不充当封闭构建证明。VM/Host报告中的源码哈希已再次核对，未发现漂移。保留历史失败与原始日志，PowerShell 将部分 stderr 记录成 NativeCommandError 的外观不代表 Cargo 失败；以命令退出码及最终结果判断。本轮没有新增 GPU 截图、原版动态轨迹、Linux/macOS 宿主验收或全游戏通关证据。

下一实现必须先闭合文字显示状态的创建、逐 tick 推进、控制指令、查询及保存/恢复，再重跑同一真实转场测试。之后仍需处理 509 条未降低形式与宿主参数缺口；不能仅消除静态 unsupported 计数就宣称完整 VM。
