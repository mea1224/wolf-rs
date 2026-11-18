# 跨图执行与地图变量归属

本轮修复针对当前 GamePro.exe，SHA-256 `13e7cf22b8b2e700f492794b4dd1b6f28ebe76d59165c1c188b95ac31b0c3b9d`。

## 原生静态证据

- `validation/native-scene-movement/context-map-setter.asm`：A05440 / A0560F / A05627，数值 setter 仅在地图上下文或版本小于 317 时指定创建地图；字符串 setter A0570F 同规则。
- `validation/native-scene-movement/transfer-0xA05800.c`：REA `ev_a000279b293744b719a136616b744bb57abd4f2d3acb81b8d2c854f8c4f26c09`。A058AB 对 getter 使用同一条件。
- `validation/native-scene-movement/map-self-getter.asm` 及根目录 `docs-append/evidence/host-numeric-read.asm`：9E4A41 消费一次显式地图选择，负值退回当前地图，随后重置选择。
- A053D0 创建执行上下文时捕获地图；A07386 发布地图和事件 owner。公共事件的调用帧与地图事件帧必须分开处理，不能把整个 fiber 的变量访问永久绑定到原地图。

## 实现

`wolf-vm::runtime::World` 保存当前地图事件变量及非当前地图变量 bank。切图只移动 bank，不重写运行中的 fiber。Fiber 捕获创建地图；每次指令执行、输入等待恢复和公共事件返回写值，都按当前帧选择正确的地图变量。独立公共上下文仍保存创建地图，但本游戏 >=317 的公共帧读写使用当前地图。

这使传送后仍执行的地图指令访问原地图，而嵌套公共事件按原版规则访问目的地图。再次进入原地图能读到尾部指令写入的值。宿主的角色事件 ID 不被编码或替换成复合 ID。

VM snapshot 版本由 9 升为 10；应用 snapshot 由 3 升为 4。旧应用快照在反序列化 Scene 前报告不兼容，不使用缺省字段伪造执行状态。原版 `.sav` 独立解析，恢复所有地图变量 bank。

## 验证

`cargo test --offline --locked -p wolf-vm --test native_map_context` 通过。测试覆盖地图事件等待期间切图、嵌套公共事件读写当前图、返回原地图帧写值、活动栈保存恢复后逐 tick 状态哈希一致，以及返回旧地图后变量保持。

上述测试验证变量/执行上下文规则；真实镜子传送、菜单、存档及性能验收另见对应运行日志，不能用这一个测试代表全部地图流程。
