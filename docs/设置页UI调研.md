# 设置页 UI 方案调研（windows-reactor）

> 日期：2026-10-27  
> 结论：**技术上可行，但不建议只为设置页引入 windows-reactor**。优先做“暗色 Win32 重绘”。

## 1. windows-reactor 现状

- 微软 `windows-rs` 仓库中的声明式 WinUI 3 库，基于原生 WinUI 3 控件，支持 Component、消息、主题（Light/Dark/System）、Mica/Acrylic。
- crates.io 最新正式版本为 **0.100.0**（2026-09-03，MIT/Apache-2.0，Rust 1.95+），文档明确标注为 **preview**，公共 API 仍可能调整。
- 默认（framework-dependent）部署需要机器上安装 **Windows App SDK 运行时**；要求 Windows 10 1809+ / Windows 11。
- 如果想单文件/自包含，需要 `windows-reactor-setup` 构建依赖并走 self-contained 部署，体积和打包复杂度都会上升。
- 适合“整机 UI 都用 WinUI 3”的应用；对现有纯 Win32 工具属于引入第二套 UI 栈。

来源：
- [windows-reactor 文档](https://github.com/microsoft/windows-rs/blob/master/docs/crates/windows-reactor.md)
- [crates.io windows-reactor](https://crates.io/crates/windows-reactor)
- [Rust for Windows – May 2026](https://github.com/microsoft/windows-rs/issues/4483)

## 2. 接入到本项目的成本

| 项目 | 影响 |
|---|---|
| 运行时依赖 | 便携包不再是“解压即用”单 exe，需要 Windows App SDK 运行时；否则走 self-contained，包体明显变大 |
| UI 架构 | 项目现有遮罩/工具栏/贴图全是 raw Win32 + GDI/GDI+；Reactor 自己接管 WinUI 线程/消息循环，设置窗需要单独窗口或进程，与常驻热键/托盘回传配置要额外桥接 |
| 构建环境 | 需要 Windows App SDK / 可能的 C++ 构建组件；CI 和打包脚本都要改 |
| 版本风险 | preview 阶段 API 可能变；与 Windows App SDK 版本强绑定 |
| 收益 | 设置页可获得现代 WinUI 3 外观、暗色主题、圆角、Mica；但只影响一个设置窗口 |

本项目当前发布目标是**便携 zip + 小体积 + 无运行时依赖**，与 framework-dependent WinUI 冲突较明显。

## 3. 备选方案比较

| 方案 | 外观 | 体积/依赖 | 工期（估） | 风险 |
|---|---|---|---|---|
| **A. 暗色 Win32 重绘（推荐）** | 深色卡片、圆角、暗色控件，可接近工具栏风格 | 0 新依赖，保持单 exe | 0.5–1 天 | 低 |
| B. windows-reactor / WinUI 3 | 最现代、原生 WinUI | 需要 Windows App SDK 运行时或 self-contained | PoC 1–2 天；完整接入 3–5 天 | 中高（preview + 打包） |
| C. egui / Slint 等 Rust UI | 自定义暗色，跨平台 | 增加数 MB 依赖，新事件循环 | 2–3 天 | 中 |
| D. 保持现状 | 原生浅色控件 | 无 | 0 | 用户不满意 |

### 方案 A 具体做法

1. DWM 暗色标题栏：`DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)`；
2. 窗口背景改为深色画刷，静态文本/复选框自绘；
3. `SetWindowTheme(hwnd, "DarkMode_Explorer", None)` 让 EDIT/滚动条走暗色；
4. 编辑框/按钮自绘圆角、hover、焦点边框，颜色沿用工具栏的 `#1B1B20 / #3A3A42 / #FF9B2E`；
5. 保留现有原生控件 ID/逻辑，只替换绘制和主题，风险最低。

## 4. 如果仍要 PoC windows-reactor

建议独立 scratch 工程，不动主仓库，验证：

1. 安装 Windows App SDK 2.0.1+ 运行时，`cargo add windows-reactor@0.100`，跑通 counter 示例；
2. 做一个设置页 mock（热键、目录、延迟、复选框、保存/取消）；
3. 测：冷启动时间、exe + 运行时总大小、便携包解压即用是否成立、与现有 Win32 进程/热键/托盘共存；
4. 决策门槛：如果必须保持“单 exe 便携、无额外运行时”，直接放弃；如果可以接受 MSIX/安装器 + 运行时，再考虑。

## 5. 推荐

先做 **A：暗色 Win32 重绘**，预计半天到一天，能解决“原生控件太丑”的主要观感问题，同时不破坏便携发布。

若之后计划把整套 UI（托盘、设置、历史记录等）现代化，再单独评估 windows-reactor + Windows App SDK 的长期路线。
