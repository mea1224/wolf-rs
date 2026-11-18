# 数据库驱动的启动配置

配置解析于2026-11-15实现；2026-11-17补充真实入口和窗口配置状态。默认目标保持 Windows、Linux、macOS，目前仅在 Windows 验证。

依据 `../../docs-append/game-map-relations-report.md` 的原生取值链及独立解析结果：

- `SysDatabase.dat` 类型 7、行 0、字段 0/1/2 是初始地图 ID 和地图坐标。
- 类型 0、行 map ID、字段 0 是地图资源路径。MapTree 不参与路径选择。
- 逻辑字段通过 1000/2000 系列编码索引整数/字符串存储，不能直接使用字段序号。

`wolf-format` 新增 C4 压缩数据库解析器，保留类型 tag、附加字符串、字段编码、行数据与重建流范围；检查存储/解压/AST 预算、计数、字符串、结束标记及精确 EOF。零字段行也计入 AST 分配预算。不支持其他数据库版本时明确失败。

`wolf-ir::startup` 实现此版本的配置语义，验证地图路径属于 MapData，拒绝路径穿越。地图 ID 3、5、25、43 分别映射 Map005、Map005_A、Map008_1、Map007_1，覆盖了不能拼接编号的情况。

播放器默认入口读取 Game.dat、系统数据库及初始地图，再通过 `wolf_ir::startup::initial_page` 对每个地图事件逆序选择首个匹配页。37字节条件块的启用位、四组条件、比较方式与事件自身变量重定位已有实现；首个禁用条件终止后缀检查。当前启动读取器仅提供已明确的初始变量范围，其他尚未初始化的值报错，不假装条件为真。选择完成后，对触发字段1/2的自动/并行页创建执行实例。当前地图2的实际结果是event0无匹配页、event1选page0，来自条件计算而非写死入口。

旧的“所有页均不启动”或“必须无条件单页才运行”诊断已不再描述当前入口。命令行也没有 `--mode evidence`、`--mode similar`、`--loose-single-page`。`--program PROGRAM_ID` 已开放为明确绕过默认页选择的诊断入口，地图仍由数据库初始配置选择；`--input-probe` 与 `--media-probe` 分别隔离输入指令和Common23 PC45..55媒体片段，不能当作完整启动。

窗口逻辑尺寸取自Game.dat word16/17，初始倍率结合word22与Game.ini读取，随后依据可用显示区域限制。INI缩放键虽含`[x0.1]`，原生消费者实际乘0.001；缺失或非正INI值回退Game.dat。当前配置得1280×720、1倍、普通窗口。显示器0按鼠标最近显示器选择，正数编号减1；更多原生地址及宿主限制见[窗口配置证据](window-configuration-evidence.md)。

Game.dat 文本字段保留以 NUL 结尾的原始字节，由 `text()` 显式请求 UTF-8 解码。地图和公共事件字符串的严格 UTF-8 校验保持原样。不输出保护元数据。当前真实归档标题为合法 UTF-8，因此此前报告中的标题编码疑点未在本实现的归档链路复现；损坏编码测试确认结构解析仍可保留字节。

## 2026-11-15配置解析验证（历史记录）

在 wolf-rs 目录执行：

```powershell
cargo test -p wolf-format -p wolf-ir -p wolf-cli -p wolf-player
cargo clippy --locked -p wolf-format -p wolf-ir -p wolf-cli --all-targets -- -D warnings
cargo run -p wolf-cli -- inspect-startup --archives ../Data
```

以上命令在2026-11-15记录中通过，不是本轮全量重跑结论。测试包含逐字节截断、预算拒绝、尾部多余数据、字段重排、负整数、非 UTF-8 Game.dat 文本、真实数据库重建流逐字节对照和异常地图编号映射。当时播放器测试构建存在诊断加载函数未使用的 dead_code 警告。

真实归档结果记录于 `../validation/startup-inspection.json`：28 类、54 条地图路径全部存在且可解析，初始地图 2、MapData/Map002.mps、坐标 (6, 6)。报告包含数据库和地图资产身份；不包含密钥字段。

## 2026-11-17执行与对照边界

本轮已将配置到地图资产选择接入初始条件选页和真实脚本入口，并连接有证据的公共调用、参数、条件及变量范围写入。相关记录见[调用支持](startup-call-support.md)、[条件与范围写入](startup-condition-range-support.md)、[数据库与字符串](../../docs-append/native-values-startup-report.md)。后续重选页、页锁覆盖、公共事件自动/并行重启及地图切换生命周期仍未闭合。坐标没有擅自换算成像素，完整可玩角色状态也未通过验证。

当前默认入口仍未完成原版启动流程。原版与Rust的实际截图、像素指标和阻塞位置见[启动窗口比较](../../docs-append/startup-window-comparison-report.md)。其中媒体探针成功呈现告知图，只证明限定片段的图片与SE路径；原版渐变及按键提示仍产生可量化差异。不能把这项探针成功，或历史数据库解析测试通过，写成完整引导或全游戏兼容。
