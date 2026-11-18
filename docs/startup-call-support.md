# 当前归档启动链与公共调用实现

日期：2026-11-17。本次为静态脚本/原生证据核对与公共调用实现；此文件不声称已运行原游戏或已完成启动兼容。

## 当前材料与 45 条指令差异

当前 `Data/BasicData.wolf` SHA-256 为 `231002c9ce9746661b1adc7b4076fe6bff4373371ff325d3e2b145774602a1a7`；`Data/MapData.wolf` 为 `8526fcf63dfc96b9b107306aa42e321b14ba54224b3611db9573c3496b0bbb7f`。前者与历史 `docs/evidence/data_manifest.json` 的 `79ca749e...` 不同，不能混用归档身份。

[逐程序差分](../validation/startup-corpus-diff.json) 对比历史 81,879 条与当前 81,924 条指令。字段覆盖 opcode、整数参数、缩进、字符串与 terminator，不包含 movement 子指令和 extension 内容：

- Common1 在旧 PC48 前新增 11 条，读取 opcode124 `[1600024,12352,0]`，检查结果46..51后预留 Common101。旧 PC48及以后顺延11。
- Common101 在原唯一结束指令前新增34条，包含活动结束提示、物品处理、迁移至地图28等逻辑。
- 其它程序上述字段完全相同，包含全部地图、Common8/17/22/23/89。因此本报告的启动 PC 与旧启动报告仍一致。

复现：`cargo run -p wolf-cli --example startup_corpus -- ../Data validation/current-startup-corpus.json`，再执行 `tools/audit_startup_corpus.py`。导出只读归档，不执行游戏或脚本。

## 已实现：普通公共调用参数绑定

[native_startup.rs](../crates/wolf-ir/src/native_startup.rs) 与编译器、VM 新增的 `NativeCommonCall` 支持 210/300 的普通公共调用。IR profile 为 v4；运行状态字段未增加，save/replay版本仍为3，library identity区分新的IR。

证据为 [210 handler](../validation/native/call-handlers.asm) `A1C03F..A1C0E2` 和 [300 handler](../../docs-append/evidence/rea-vm-a1cfa0.c) `A1D997..A1DB31`，后者 REA Evidence `ev_4ab874b58a513fdbcb3403fe5f29b3a41f382a2c3333ccd8ac8fab50bb93ded4`。

`args[1]` 低4位为数值参数数，下一组4位为字符串数；数值操作数从 `args[2]` 开始，字符串操作数紧随。数值参数按顺序写目标公共事件 self0..4；字符串写 self5..9。字符串第i项的 literal 位为 `flags & (1 << (12+i))`，literal时使用 `strings[i+1]`，否则读取对应整数操作数编码的字符串地址。引用参数不要求存在未用的文本占位：实际语料有300的 `strings.len()==1` 和210的 `strings.len()==0`。

参数写入不读取编辑器 metadata。原生是逐项读取调用者值、立即写目标槽，再处理下一项；不是先求值整个参数数组。实现保留这个顺序，并先绑定全部数值，再绑定字符串。递归到同一common时，后续实参能读到前面刚写的self槽。未传入的self槽保留原值，callee不建立数值Local实参。map event owner沿用调用者。

字符串参数目前只执行无需展开的文本；反斜线或 `<` 内容会在调用处带源位置停止，防止把 `\cself[...]` 等原文误当展开结果。未实现94FC50展开/懒加载写回、返回值标记、动态目标、带参数211或超过5个数值/字符串的形式，仍明确拒绝。调用栈恢复已识别 NativeCommonCall。

测试：[参数布局](../crates/wolf-ir/tests/native_startup.rs) 6项；[VM调用行为](../crates/wolf-vm/tests/native_call.rs) 3项。包含实际标题配置调用和keyguide参数形态、同common数值及字符串别名顺序、省略槽保留、活动递归调用栈save/restore、owner继承与未支持展开停止。

## 从真实脚本到启动画面的必要顺序

下表是当前语料的控制流要求，宿主命令是否已实现需检查当前运行代码。它不是固定展示流程；各步骤必须由相应指令触发。分支条件不同可改变配置路径。

| 源位置 | 必需行为与依赖 |
|---|---|
| Map002/event1/page0 PC0..2 | `270 [0,1]` 后按名称300进入Common23，然后结束；270的原生handler为A2A570 |
| C23 PC1..16 | self12从0到99扫描存档，221 selector9000024，首次存在置self23=1退出；PC5条件countword为0x11 |
| C23 PC17..40 | 无存档时按sys141设置sysS60标题并调用C8 mode0；随后总是调用C8 mode1，再按DB组合地址1109000700设置标题 |
| C8 PC1..8 | self5=`Save/system.sav`，查询存在；缺失时220保存。不能一律把220变空操作 |
| C8 PC87..231 | game_config存在检测；模式10→11创建或12读取。用DB4配置定义、DB9当前值，分割key=value、0..100边界检查；写sys100/102/101 |
| C8 PC259..430 | key_config存在检测、创建/读取；DB3定义、DB8当前键位，键盘100..299/手柄300..315验证，更新sys52/55、sys53/56 |
| C23 PC42..46 | 清全局字符串0并写currentsav.txt；self19=10000；121 flags0x10000、范围1同时写self10/self11 |
| C23 PC47..55 | kuro.png与krachware.png、logo音频；等待45、图片删除过渡20、等待20；创建guideline.png后再等待20 |
| C23 PC56 | 300 flags0x3022，数值[10,0]、字符串["[決定]OK","[決定]OK"]，名称解析到 **Common89** keyguide |
| C89 输入提示生成 | 读取DB8记录名(-3)、DB3键码；121范围self36..39、间接dst flags0x10；112包含判断；122替换操作0x900，生成icon与文本格式控制串 |
| C23 PC62..74 | 每轮wait1，检查V2000005=1且V2000003=10才退出；需要Common22并行发布输入状态 |
| C23 PC75..80 | C89 mode1关闭提示；图片与音频操作后wait20 |
| C23 PC86..98 | self12=0；调用 **Common17** 粒子mode10/基数10100；设置转场160/161；创建title_loop.mp4；162等待转场完成 |
| C17 粒子分支 | 124图片属性查询、121随机mode6、150图形/动画参数和290效果。不能直接跳过该调用 |
| C23 PC99之后 | 语言分支logo、菜单图片/效果、继续游戏有效性及循环输入；依赖相同变量、图片和输入语义 |

零次循环仍必须跳过。C8旧二进制配置读写在PC14..34与40..68中，不能因其中存在221/222/250就要求普通启动执行这些指令。C23 PC57..61的临时图片、PC81..84的清除记录查询、PC292..309的评测选项也被零次循环禁用。

## 仍需闭合的112/122/250证据

[原生VM语义报告](../validation/native/native-vm-findings.md) 已给112 packed operand与122 source/update位域。112低24位是左侧字符串地址，位24..27选择右侧变量/文本，位28..31选择比较；但字面和引用还有展开路径。C8不仅需要相等，还需要包含/分割；不能只把所有字符串条件当成Eq。

250分派到A1FF30。C8可达形态包括0x11200类型记录数、0x51200定义字段读取、0x51000当前数据读取、0x50000当前数据写入、0x71000按记录名读取。以上形态来自脚本及已有报告，完整名称查找/字段分类应依原生handler闭合。数据库组合地址1109000700与250 handler必须使用同一数据存储，才能使配置修改影响标题语言。

Common22输入最终写V2000003/5/7/17，有原始代码转换、DB8键位/DB3逻辑码映射及调用次数门控。按键值不可直接写成确认键10来绕过脚本。详见 [输入存档分析](../../docs-append/input-save-script-report.md)。
