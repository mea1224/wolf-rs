# 图片 selector10：文字显示进度证据

2026-11-18。目标 GamePro.exe SHA-256：`13e7cf22b8b2e700f492794b4dd1b6f28ebe76d59165c1c188b95ac31b0c3b9d`。以下为已有静态 REA 导出与对齐汇编的补证，没有启动原版或取得新的原版运行轨迹。

此前 [字段清单](../validation/native-selector10-evidence-map.md) 已记录 selector 表达式，但没有闭合嵌入文本对象的偏移。这里补齐对象关系，并保留尚未实现的时序边界。

本轮另通过 REA/IDA 核对 `973530` 的59条汇编：`973539` 保存传入的ECX，`97356B` 将同一指针传给 `973B30`。完整记录保存在 [REA bundle](../validation/resume-20261118/rea-text-progress-bundle.json)，函数 dossier ID 为 `ev_b743745d38516d11d1db2c78523845017fb07bc6daf998fdf685212c9f0a3eb6`。这是当前分析数据库的静态观察；适配器未提供完整CFG/typed references，不能当成执行轨迹。owned binary session 已关闭，证据已导出并 reset。

## 对象关系与消费者

`98E2F0` 是 opcode150 variant2 的文字图片创建路径。`98E79C` 将图片对象 `+0x2FC` 设为 1；`98F0C0` 明确 `lea ecx,[edi+8]` 后调用 `973530`，后者用同一接收对象调用 `973B30` 测量/遍历文字。因此 `973B30` 的 `v6` 是 **picture+8 的嵌入文本对象**，不是四角坐标对象。

| 图片偏移 | 文本对象偏移 | 静态证据支持的含义 |
| --- | --- | --- |
| `+0x2FC` | 非文本对象成员 | 文字图片 gate；为 0 时 selector10 返回 -1 |
| `+0x18` | `+0x10` / `v6[4]` | 编码文本字符串长度；973B30 的遍历索引与其比较 |
| `+0x78` | `+0x70` | 显示速度；98F26C 从全局155952C写入 |
| `+0x80` | `+0x78` | 显示累计值；9737C0 按 `step * rate / 30` 推进，上限50,000,000 |
| `+0x88` | `+0x80` | tick 更新前的累计副本，由9737D2写入 |
| `+0xE0` | `+0xD8` / `v6[54]` | 遍历中的字节补偿：973E83清零，974010/97401A加入字符编码长度超出2的部分；975D10另增加控制包长度 |

这修正了将 `+0x80` 简称“前帧累计”的歧义：它是会被 tick 和文字遍历更新的累计字段；保存旧值的是 `+0x88`。selector10 的阶段补偿直接写在表达式中，不能仅凭字段名字替代。

`993B90` case10 / `99569A..995707`：

```text
gate == 0 => -1
否则：trunc(picture.accumulator_80) + picture.byte_correction_E0
      - global_step * picture.rate_78 / 30 + 1 >= picture.text_length_18
```

`973B30:973FCF..97401A` 先计算下一字符编码字节长度，将超过2的字节计入补偿，再在 `974055..974059` 比较下一索引与 `trunc(accumulator)+correction`，决定是否停止遍历。`975D10:975EAD` 及各控制分支继续调整补偿。控制 type0x20（源码 `\>`）设置即时显示标志；type0x0B 清除该标志。`9737C0` 还受暂停标志和等待计数限制。因此用 Unicode 字符数量或最终图像尺寸直接推算完成时间不够。

## 实现边界

当前 `native_text` 返回最终排版图像，`NativeMedia::Entry` 主要保存几何、图片效果和动画；尚无上述文本累计/补偿状态。`offset_effect` 是图像位置插值，不能用于回答 selector10。把该查询恒定设为1会跳过 Common37 的实际等待路径。

正确接入需要同时覆盖：创建时的编码文本及速度、绘制遍历产生的补偿、暂停/等待与即时显示控制、tick/查询的阶段关系、保存/恢复。测试应至少覆盖 ASCII/多字节字符、控制包、显示未完成/完成边界、非文字图片返回-1、同ID替换、读档后的相同剩余时间。需要避免由 GPU 呈现频率改变模拟推进。

证据文件：

- [picture-render.asm](../validation/native-save/picture-render.asm)：98E79C、98F0C0、98F26C、99569A。
- [973530.c](../../docs-append/evidence/startup-native/973530.c)：文本测量入口。
- [973B30.c](../../docs-append/evidence/startup-native/973B30.c)：编码字节遍历与显示 gate。
- [9737C0.c](../../docs-append/evidence/startup-native/9737C0.c)：逐 tick 累计、等待和暂停。
- [975D10.c](../../docs-append/evidence/startup-native/975D10.c)：控制包消费与补偿。
- [993B90.c](../../docs-append/evidence/startup-native/993B90.c)：selector10 分支。
