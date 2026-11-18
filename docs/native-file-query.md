# 原生特殊字符串文件查询

2026-11-17，当前启动在Common8 PC90到达 `122 [1600005,0x40200,0]`，文本为 `<<GET_FILE_EXIST>>Save/game_config.txt`。此前宿主未处理这一分支，导致配置读取流程停止。

`string-command.asm` 中A13617比较操作0x40200，A13622..A13649检查文本以 `<<` 开始，A13655调用A2F7C0。该函数A30668..A30686检查 `<<GET_FILE_EXIST>>` 前缀；A30693..A306A5取剩余路径；A3071F执行955A50内部显示控制转换；A30751经7496A0执行文件查找。句柄不等于−1时返回地址C8C3A0的字符串 `1` 并关闭句柄，失败时返回C8C3A4的字符串 `0`。这些字面量由当前EXE对应RVA直接读取确认。局部分支反汇编保存于 [file-exist-query.asm](../validation/native-values/file-exist-query.asm)。

`native_control.rs` 现在对这一明确形式展开源文本，规范化相对路径，查询本地文件系统，将字符串 `1` 或 `0` 写入目标。不是数值布尔赋值。通配符、`<<`特殊路径和内部显示控制保持明确unsupported；尚未声称实现归档成员搜索和所有原生文件后端。非NotFound的宿主I/O错误保留诊断。

测试直接从当前CommonEvent.dat提取Common8 PC90、PC144、PC90、PC168，在新临时目录经VM宿主依次执行查询、写入、查询、读取。查询得到 `0` 然后 `1`，写出BOM与CRLF，读取还原为内部LF。`cargo test --offline --locked -p wolf-player native_control::tests -- --nocapture` 三项通过。只删除测试自身创建的文件和空目录，没有触碰游戏现有配置。
