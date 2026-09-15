# 样式免编译验证记录

## 实现状态

在 `main` 上完成，基线为 `abbd1b12fcfa521764953ef924bdfefab92df5fc`，修改尚未提交。
72 个样式 token 标记为运行时数值。普通列表文字默认仍为 `#555b60`，搜索高亮仍为蓝色 `#285f80`。
主卡、侧卡共用列表配色、字号和条目圆角；普通字重由窗口默认字体属性绑定。
窗口外框、定位、预算和动画策略保持静态。

运行方式及边界见 [runtime-styles.md](runtime-styles.md)。先从托盘退出旧实例，再运行 `echo.cmd dev`，保存已标记的数值即可热更新。

## 编译缓存：PASS

同一开发配置、同一 `native-test` 特性集合下，记录 Cargo JSON 的 UI library artifact：

| 改动 | 构建耗时 | `echo-desktop-ui` fresh |
| --- | ---: | --- |
| 无改动 | 0.42 s | true |
| 仅修改列表颜色数值 | 2.57 s | true |
| 仅修改间距数值 | 2.61 s | true |
| 桌面端 Rust 语义探针 | 3.56 s | true |
| 恢复所有探针修改 | 3.56 s | true |

所有临时源文件修改已恢复，生成结果已校验。生成器测试另行断言：数值修改不会改变 Slint 接口或 presentation 常量，也不会重写内容未变的文件。

接口迁移的首次开发构建约 8 分 11 秒，最终结构补齐后的开发构建约 8 分 7 秒。这些是完整迁移构建，不能当成单行结构修改的独立微基准。
结构修改依然需要 UI 编译。普通 Release 构建此次未重编译 UI，但桌面端优化/链接仍耗时约 7 分 10 秒；保留了原来的 ThinLTO、优化等级和代码生成配置。

原始证据：[缓存结果](../../.local/echo/style-iteration/cache-results.json)、同目录 `cache-*.jsonl`。

## 原生热更新：PASS

使用真实 Windows/Slint 程序、复制的 D0 合成数据、禁用捕获的隔离实例。检查来自 UI 线程实际属性和已存在的行模型，截图来自该窗口自身的渲染器。

| 修改 | 保存至属性确认 |
| --- | ---: |
| 普通文字颜色 | 78 ms |
| 字号 | 531 ms |
| 普通字重 | 500 ms |
| 内容间距 | 500 ms |
| 条目圆角 | 516 ms |
| 已有搜索高亮颜色 | 531 ms |

同一 PID 中完成，没有编译或重启。查询、选择、行键和**非零滚动位置**保持一致。
截断 JSON、非法字号均保留上一份有效样式；修复后再修改字号，能够继续更新。
浅色、深色、合成高对比度状态及六次空间切换通过。高对比度匹配片段只加粗，继承系统文字颜色；RGBA 高亮透明度另有单元测试。

截图为受控实验：临时改为灰色 `#777777`、红色高亮和较大字号，**不是发布默认值**。

- [更新前](../../.local/echo/style-iteration/native-03/before.png)
- [更新后，包含侧卡](../../.local/echo/style-iteration/native-03/after.png)
- [深色](../../.local/echo/style-iteration/native-03/dark.png)
- [合成高对比度](../../.local/echo/style-iteration/native-03/high-contrast.png)
- [断言和延迟记录](../../.local/echo/style-iteration/native-03/result.json)

首轮驱动遇到 Windows 响应文件替换时的共享冲突；驱动增加短暂读取重试后通过。最终结果以上述 `native-03` 为准。

## 启动和渲染测量

迁移前版本由上述基线归档并恢复原有深灰文字改动构建。两版本采用相同 Cargo 配置和 `native-test` 特性。
完成所有编译后，前后交替执行，每个配置各 5 轮、每轮 6 次空间切换。
启动值是进程创建至首次原生帧提交的中位数；帧值是每轮渲染/提交 P95 的中位数。

| 配置 | 启动：之前 → 之后 | 帧渲染/提交：之前 → 之后 |
| --- | ---: | ---: |
| Dev | 95.45 → 99.20 ms | 30.54 → 31.72 ms |
| Release | 73.22 → 76.29 ms | 14.82 → 14.83 ms |

这批样本中，启动增加约 3–4 ms，Dev 帧值增加约 1.18 ms，Release 帧值基本持平。数据不支持“零开销”的说法，也不是跨硬件或物理显示延迟认证。
性能数据来自带隔离测试桥的构建；不带测试特性的普通 Dev/Release 可执行文件另行构建通过。正式版的样式文件读取和监视代码由条件编译排除。

[汇总及可执行文件 SHA-256](../../.local/echo/style-iteration/performance-summary.json) · [全部 20 轮样本](../../.local/echo/style-iteration/performance-samples.json)

## 最终门禁

- `echo.cmd self-check`：PASS
- `echo.cmd format --check`：PASS
- `echo.cmd verify`：PASS，包含生成器、样式解析/恢复、透明度及现有工作区测试
- 普通 Dev 构建、普通 Release 构建：PASS
- 原生热更新、隔离空间切换：PASS
- 真实 Windows 高对比度切换、物理输入、安装包验收：NOT_RUN

[各步骤退出码](../../.local/echo/style-iteration/final-validation.json)。本地原始证据位于 `.local/echo/style-iteration`，不纳入版本控制。

单独复验热加载需先构建 `--features native-test` 的开发程序，再设置 `ECHO_WINDOWS_ACCEPTANCE=1`，运行：

```text
python tests/native/Invoke-StyleAcceptance.py --executable <exe> --template <synthetic-data> --source design/tokens/echo.tokens.json --evidence <new-directory>
```
