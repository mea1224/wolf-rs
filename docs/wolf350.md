# WOLF350 归档解码与验证

实现位于 `crates/wolf-assets/src/wolf350/`。以纯 Rust 完成地址解混淆、WOLF350 密钥派生/XOR、变体 AES-CTR、元数据 Huffman/DX LZ、目录索引及成员完整/分段 Huffman/DX LZ。运行时不调用 UberWolf、DLL 或外部程序。默认平台目标为 Windows、Linux、macOS 桌面；本次仅验证 Windows。

`Archive::open(Vec<u8>, Limits)` 拥有归档字节并构建索引；`entries()` 按路径排序；`read(path)` 返回成员原始字节、源归档及内容 BLAKE3，解码版本为 `dxa8-wolf350-v1`。查找要求精确名称，保留 CP932 原名字节。大小、展开总量、记录数、目录深度及数据范围受限；拒绝路径穿越、大小写/NFC 冲突、重复文件记录、目录循环及 reparse 属性。每次读取均解码，无缓存或导出副作用。

仅接受 DXA v8、WOLF350、CP932、flags=0x015e0000。小于 1088 字节的归档未验证，明确拒绝。未压缩元数据/NO_KEY 配置未验证，拒绝而不套用未经证实的分支。默认归档上限 512 MiB、压缩与展开元数据各 32 MiB、单成员 256 MiB、累计声明展开大小 2 GiB、100000 条目录项、64 层目录。归档名字节使用 CP932，内部地图/事件文本按各文件版本独立解码；归档压缩与内层 `.mps/.dat` LZ4 是不同层。

## 解码顺序与边界

1. 验证头部 profile、长度和 CP932；保留原始归档 BLAKE3。
2. 从头部 15 字节密码派生 MSVC 风格 RNG，恢复头部地址；对归档数据执行外层 XOR。
3. 用 WOLF AES 变体依次处理头部之后的指定数据区和元数据区。它具有自定义密钥扩展，普通 AES crate 不能直接替代；计数器跨区连续，但每个区尾部未用完的密钥流字节丢弃。
4. 元数据执行内层 XOR、Huffman、DX LZ，要求展开长度等于头部声明。压缩元数据在复制前受限。
5. 遍历目录和 72 字节文件记录；检查 32 字节目录记录及文件记录对齐、范围、循环、重复项和规范路径冲突。
6. 读取成员时，按声明原始大小作为 XOR 起始位置；如需分段 Huffman，重建首尾解码段和原样中段，再解 DX LZ。

Huffman 声明存储大小必须与完整输入切片一致；成员各阶段的输出上限取自已验证的索引大小。DX LZ 在写入前检查输出容量，并验证回引用、重叠拷贝、保留编码和最终长度。元数据偏移、归档存储偏移和解压后偏移分别使用，不混作同一寻址域。

## 当前语料复核（2026-11-18）

当前 12 个原包的 SHA-256 全部不同于 2026-11-15 留存基线，因此旧报告不能直接充当当前字节对照。此次将原包复制到新的 `validation/resume-20261118/archive-reference-current/`，使用下述固定 SHA-256 的独立 UberWolfCli 解包，再用当前 Rust CLI 对原包逐成员解码比较。未启动 GamePro.exe，原包在检查前后 SHA-256 一致；历史参考和报告保留。

结果仍为 **12 包、2,023 成员；2,019 个完全一致，4 个参考工具移除特定 62 字节前缀后相等**，无其他差异。独立参考的路径集合也与 Rust 索引完全相等。新旧解包内容只发现 `BasicData/CommonEvent.dat`、`BasicData/Game.dat`、`MapData/Map028.mps` 三个文件不同，其余 2,020 个文件相同，无新增或删除。归档字节变化不能直接解释为全部资产内容变化。

当前输入、历史输入、逐成员结果及验证器身份见 [archive-current-summary.json](../validation/resume-20261118/archive-current-summary.json) 和同目录各 `archive-*.json`；参考提取命令及输入身份留存在新参考目录的 `reference-provenance.json`。源码快照与 CLI 可执行文件分别记录哈希；验证脚本不负责构建，也不将源码哈希冒充构建证明。

复现时先构建 `cargo build --offline --locked -p wolf-cli`，再使用已有、带来源记录的当前参考：

```powershell
python tools/verify_archive_identity.py --archives ../Data --reference validation/resume-20261118/archive-reference-current --historical-reference validation/archive-reference --output validation/resume-20261118
```

首次创建参考时，指定新的空目录，附加 `--prepare-reference --reference-tool ../docs/tools/UberWolfCli-v0.6.4.exe`；脚本先校验提取器 SHA-256，只对归档副本解包。已有目录不会被覆盖。新参考内的游戏资产与归档副本仅供本地验证，不作为源码分发内容。`verify_archives.py` 仍可比较显式指定的目录，但不能把旧 `validation/archive-reference` 当作当前包的相同版本参考。

## 历史独立对照（2026-11-15）

2026-11-15 在 Windows x86_64 MSVC 上重新构建并解码全部 12 包，与独立 UberWolfCli 提取目录逐成员比较。参考文件名为 `UberWolfCli-v0.6.4.exe`，来自 v0.6.4 发布资产，但 `--help` 实际自报 `UberWolfCli v0.5.4 Build #106`；以 SHA-256 `0c9645733ae9544df11ee0c859a7f2cb51aa547d5d13f7935cb480bdab96fb3a` 标识实际对照程序，不据文件名推断内部版本。

- 总成员 2023，2019 个字节完全一致。
- BasicData 的 CDataBase.dat、CommonEvent.dat、DataBase.dat、Game.dat 在移除解码结果的特定 62 字节前缀后与参考相等。此比较规则只用于验证；解码器保留前缀。
- 无其他差异。该结果验证此语料，不证明全部 WOLF 版本通用兼容。

前缀为 ASCII `Extracting data from encrypted files violates the guidelines.` 加终止 NUL。机器可读结果见 [archive-summary.json](../validation/archive-summary.json)，包括每包源 BLAKE3、成员数和两类比较计数；先前保留的 [archives-summary.json](../validation/archives-summary.json) 还包含源归档和参考可执行文件 SHA-256。

复现（在 `wolf-rs/`）：

```powershell
python tools/verify_archives.py --archives ../Data --reference validation/archive-reference
cargo test --offline --locked -p wolf-assets
cargo clippy --offline --locked -p wolf-assets --all-targets -- -D warnings
```

另有直接调用 Rust API 的本地语料集成测试：

```powershell
$env:WOLF_ARCHIVE_ROOT = (Resolve-Path '../Data').Path
$env:WOLF_ARCHIVE_REFERENCE_ROOT = (Resolve-Path 'validation/archive-reference').Path
cargo test --offline --locked -p wolf-assets --test corpus_archives -- --ignored --nocapture
```

参考目录必须预先通过独立工具生成，按归档名分目录；这些解包资产不作为分发所需测试夹具。目录不存在时应先自行准备，不能仅凭已保留的摘要重新执行字节对照。脚本不运行提取工具，不下载或分发游戏资产。

`wolf-assets` 的 9 项单元测试和该本地语料集成测试均通过，Clippy 无警告。单元测试包含自制 Huffman 等权重/位序/截断输入、DX LZ 越界/重叠拷贝、错误 profile，以及与上游 C++ 独立生成器比较的 WOLF AES 两区向量（17 字节后接 31 字节）。向量生成器保存在 `crates/wolf-assets/tests/reference/wolf_aes_vector.cpp`，以 notices 中哈希固定的 `WolfAes.hpp` 为输入；正常 Cargo 构建不需要 C++ 或上游源码。

实现参考资料来自根目录三份架构/格式/实现计划文档及其引用的 UberWolf 源码。源码快照与对照可执行文件是分别固定的参考材料；语料匹配不证明所有版本支持。算法来源和尚未明确的 DXLib 许可范围见 [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)。
