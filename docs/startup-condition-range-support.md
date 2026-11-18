# 启动条件与范围赋值

当前IR profile为 `wolf350-evidence-subset-v5`。这轮补齐标题启动可达的两种原始记录形式，保留单条命令的PC、源范围及raw参数。

数值分支111的 `A0D052/A0D055` 对第一个参数执行 `& 0xF`，随后按该数量遍历条件。整个普通handler不再读取该参数高位。当前编译器因此支持已观测的0x10标记与1..4的低位计数，例如Common23 PC5/89的17表示一条条件。比较参数自己的0x10位仍是强制RHS立即数，二者不能混同。更高的未知计数标记仍拒绝。

范围赋值121：`A0EB27` 检测flags的0x10000，`A0EB30..A0EB45` 取第五参数并将负值归零。`A0EB61` 使用起始原始地址加offset；`A101B1..A101C4` 递增offset并在 `offset <= maximum` 时返回 `A0EB50`。因此第五参数是包含末端的最大偏移，而非元素数量。每次返回均重新读lhs/rhs，之前的目的变量写入会影响后续读取；实现不能先算出一个值再批量填充。

新增 `Op::ComputeRange` 在一个VM步骤中按序处理目的地址，采用已有普通整数运算规则。参数长度、地址类型及最大10000个目的变量均校验；字符串槽、间接、double、RNG及复合数据库目的地址不因范围支持而自动放行。最后一项限制仍需另行恢复对应原生路径。

证据沿用 `validation/native/condition-handler.asm` 与 `arithmetic-handler.asm`。新增测试覆盖高位计数下的真假分支、范围包含末端、写后重读源地址的别名效果、commonself范围、负数最大偏移以及非法地址/长度/数量拒绝。`cargo test -p wolf-ir --test compile -p wolf-vm --test native_control` 23项通过。
