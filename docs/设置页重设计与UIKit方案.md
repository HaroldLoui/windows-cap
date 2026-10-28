# 设置页重设计与 UI Kit 拆分方案

> 日期：2026-10-28
> 状态：方案已定，未实施
> 前置调研：[设置页UI调研.md](设置页UI调研.md)（结论仍有效：不引入 windows-reactor / egui，坚持纯 Win32 + GDI+）

## 1. 背景

当前设置页（`src/ui/settings.rs`）是"原生控件 + 半自绘"的混合体，观感与遮罩工具栏割裂：

| 问题 | 原因 |
|---|---|
| 输入框硬直角、无 focus/hover 反馈 | 原生 `EDIT` + `WS_BORDER`，仅改了颜色（settings.rs:648） |
| 复选框渲染破图 | `BS_AUTOCHECKBOX` + `DarkMode_Explorer` 的已知问题 |
| 视觉层次弱 | 标签/输入/说明文字权重一样，没有分区 |
| 右下大片空白、比例失调 | 固定 `BASE_HEIGHT`，行距机械 |
| 标题栏与内容割裂 | 只开了暗色，未做 Win11 圆角 + 标题栏同色 |

同时，全项目 UI 基础代码重复严重（拆分调研结论）：

- `wide()` 完全相同的实现 ×3（pin.rs:173 / settings.rs:192 / tray.rs:275），另有 8 处裸 `encode_utf16`；
- `create_font(scale)` ×2（overlay.rs:1504 / settings.rs:196）+ 内联 CreateFontW ×2（text_edit.rs:93 / spike）；
- GDI+ graphics 创建 + 抗锯齿样板 ×3（render.rs:69 / toolbar.rs:332 / toolbar.rs:754）；
- DPI 缩放 4 种写法并存（settings 的 `s()` / toolbar 的局部 `s` / overlay 的 `ui_scale` 手乘 / 各处 `GetDpiForWindow/96.0`）；
- 主题色字面量（`0x00FF_9B2E` 橙、`0x0020_2020` 深底、`0x00FF_FFFF` 白）在 overlay / pin / toolbar 三处硬编码重复；
- `state(hwnd)`（GWLP_USERDATA 取状态）样板 ×4；
- `DrawTextW` 居中文字样板 ×5 处。

## 2. 决策记录

| 决策点 | 结论 | 理由 |
|---|---|---|
| UI 技术路线 | 纯 Win32 + GDI+ 自绘深化，不引入 UI 库 | 单 exe 便携、0 新依赖；设置页组件需求收敛（约 6 个控件），不值得引入第二套 UI 栈（见调研文档 §3） |
| 拆分形态 | **内部模块 `src/ui/kit/`（`pub(crate)`）**，暂不拆独立 crate | 单人维护单应用，crate 的硬边界暂无第二个消费者兑现，但 workspace/CI/feature 对齐的摩擦是实打实的；API 稳定后平移成 crate 是机械操作 |
| 设置页布局 | 卡片式、标签在上（方案一） | 消除左标签布局的右侧留白浪费；热键录制器在宽卡片内视觉更好；与工具栏 GDI+ 圆角同一套代码路径 |
| 热键交互 | 做"按键捕获"式录制器 | ShareX/Snipaste 标配；免手输；捕获逻辑任何方案都得写，纯自绘交互自由度最高 |
| 输入框策略 | 保留原生 `EDIT`，只自绘边框/底板 | IME/选中/复制等文本框最难的部分由原生承担，零输入体验回归；复用 text_edit.rs 已验证的叠加模式 |
| 文本输入 vs 自绘边界 | 难的用原生，好看才自绘 | 见 §3.2 组件策略表 |

## 3. 方案总览

### 3.1 设置页目标效果（卡片式 · 标签在上）

```
╭──────────────────────────────────────────╮
│  window-cap 设置              —  □  ×    │  ← Win11 圆角 + 标题栏同色
╰──────────────────────────────────────────╯
  快捷键
  ╭──────────────────────────────────────╮
  │  截图热键                             │
  │  ┌────────────────────────────────┐  │
  │  │ ▶ 按下新快捷键…                 │  │  ← 点击进入录制态
  │  └────────────────────────────────┘  │
  │  退出热键                             │
  │  ┌────────────────────────────────┐  │
  │  │ Ctrl+Alt+Q                     │  │
  │  └────────────────────────────────┘  │
  ╰──────────────────────────────────────╯

  输出
  ╭──────────────────────────────────────╮
  │  保存目录                    [浏览…] │  ← 标签行右侧放按钮，省一行
  │  ┌────────────────────────────────┐  │
  │  │ Pictures/Screenshots           │  │
  │  └────────────────────────────────┘  │
  │  延迟截图 (ms)                        │
  │  ┌──────────┐                        │
  │  │ 0        │                        │  ← 短字段，不撑满
  │  └──────────┘                        │
  ╰──────────────────────────────────────╯

  启动
  ╭──────────────────────────────────────╮
  │  开机自动启动                    ●── │  ← 自绘开关
  ╰──────────────────────────────────────╯

              保存目录支持 %USERPROFILE%…
                          [ 取消 ]  [ 保存 ]
```

配套改动：

1. **DWM**：`DWMWA_WINDOW_CORNER_PREFERENCE`（Win11 圆角窗口）+ `DWMWA_CAPTION_COLOR`（标题栏与背景同色），各一行；
2. **输入框**：父窗口 GDI+ 画 8px 圆角底板 + hover 边框 + focus 橙色强调边框，透明背景 `EDIT` 叠加其上（保留 IME/选中/复制）；
3. **复选框 → 自绘开关（toggle）**：顺带修复破图；
4. **布局**：按内容计算窗口高度，删除 `BASE_HEIGHT` 拍脑袋常量；
5. **hover 态**：owner 按钮补 `WM_MOUSEMOVE` + `TrackMouseEvent` hover。

### 3.2 组件策略（难的用原生，好看才自绘）

| 组件 | 策略 | 说明 |
|---|---|---|
| 文本输入 | 保留原生 `EDIT`，自绘底板/边框 | IME/选中/复制/无障碍由原生承担 |
| 按钮 | 已有 owner-draw，迁入 kit 泛化 | `draw_owner_button` → `widgets/button.rs` |
| 卡片/分区 | WM_PAINT 绘制，非控件 | `widgets/card.rs` |
| 开关 toggle | **新写**（约 80 行） | 原生复选框即破图元凶 |
| 热键录制器 | **新写**（约 150 行） | 纯自绘：捕获态画在卡片上，自行处理修饰键循环捕获 |

热键录制器状态机（纯自绘）：

```
Idle(显示当前组合键)
  └─ 点击 → Capturing(显示 "▶ 按下新快捷键…"，高亮脉冲)
       ├─ 捕获到合法组合键（含至少一个修饰键+主键）→ Idle(回显新组合键)
       ├─ 只按下修饰键 → 保持 Capturing（等待主键）
       ├─ Esc → Idle(恢复原值)
       └─ 失焦/点击别处 → Idle(恢复原值或提示未捕获)
```

### 3.3 UI Kit 模块结构

```
src/ui/kit/
  mod.rs        — 模块文档；边界规则：只依赖 std + windows crate，
                  禁止 use crate::{capture, annotate, app, output}
  theme.rs      — 颜色单一来源
                — 合并 toolbar.rs:22-33、settings.rs:56-61 的常量，
                  收编 overlay.rs:148-150 / pin.rs:580-588,954-959 的硬编码字面量
  text.rs       — wide()（3 份合一）、draw_text()（DrawTextW 居中样板 ×5 合一）、
                  create_font()（overlay/settings 两份合一，带回退）
  dpi.rs        — s()、scale_from_hwnd()（4 种写法合一，统一 .max(1.0) 兜底）
  gdiplus.rs    — init()（吸收 annotate/render.rs:27-50 的全局 Once 单点，
                  否则 toolbar → kit::init 会形成 kit → annotate 反向依赖）、
                  Graphics 守卫、check(Status)、pen/brush 临时对象助手
  shapes.rs     — fill_rounded / stroke_rounded / add_rounded_rect /
                  fill_ellipse / stroke_ellipse / stroke_line / stroke_lines /
                  stroke_polygon / fill_polygon / stroke_rect
                — 从 toolbar.rs:563-745 整体平移（非重写）
  window.rs     — state(hwnd)（GWLP_USERDATA 样板 ×4 合一）、register_class 助手
  widgets/
    mod.rs
    button.rs   — 圆角按钮绘制（settings 的 draw_owner_button 泛化，
                  支持 accent/普通/pressed/hover）
    card.rs     — 圆角卡片面板（底色 + 细边框）
    field.rs    — 圆角输入框底板 + hover/focus 边框（供原生 EDIT 叠加）
    toggle.rs   — 开关（off/on/hover/pressed 态）
    hotkey.rs   — 热键录制器（§3.2 状态机）
```

**明确留在业务侧、不下沉的内容**：

- `Tool` / `ToolbarAction` / `draw_icon`（图标绘制与业务枚举硬耦合，toolbar.rs:817-1001；后续如需下沉须先参数化成 IconId，第一期不碰）；
- `Toolbar::layout` / `ToolbarState`（含 can_undo/can_pin 等业务状态）；
- `overlay_wndproc` / `apply_action` / `preview_shape` / `export_image` 等遮罩业务；
- `save_settings` / `browse_folder`（配置读写与目录选择）；
- `selection.rs`（已是纯逻辑，暂不动）；
- `text_edit.rs`（带 WM_TEXT_* 业务协议的控件，暂不动）。

## 4. 实施计划（两期）

### 第一期：建 kit + 设置页重设计（先出用户可见价值）

| 步骤 | 内容 | 涉及文件 |
|---|---|---|
| 1 | 建 `src/ui/kit/`，**平移**去重：wide×3→1、create_font×2→1、DPI 4 种→1、颜色归一、GDI+ 初始化归一、shapes 平移 | 新增 `src/ui/kit/*`；改动 pin.rs / settings.rs / tray.rs / overlay.rs / toolbar.rs / annotate/render.rs（仅删本地实现换调用） |
| 2 | 新写 card / field / toggle / hotkey 四个 widget | 新增 `src/ui/kit/widgets/*` |
| 3 | 设置页重写：卡片布局 + 热键录制器 + toggle + Win11 DWM 圆角/同色标题栏 + hover 态，全部走 kit；删除 draw_owner_button | 重写 `src/ui/settings.rs` |
| 4 | 验证：cargo fmt / clippy（-D warnings）/ test 全绿；手动过一遍热键录制、目录浏览、自启开关、保存校验 | — |

第一期风险控制：步骤 1 是纯平移（对外行为不变，编译器兜底）；步骤 3 集中在 settings.rs 单文件重写，不触碰遮罩/贴图路径。

### 第二期（可选，独立提交）：存量窗口迁移

| 步骤 | 内容 |
|---|---|
| 5 | `toolbar.rs` 绘制改用 `kit::shapes` / `kit::theme`（`Toolbar::paint` 内部换实现，对外 API 不变） |
| 6 | `overlay.rs` / `pin.rs` 的 `draw_chip` / `draw_tooltip` / 文字样板逐步换 kit 调用 |

第二期动 5000 行的大头，独立做、独立测、独立提交。

## 5. 拆分形态演进路径

当前：内部模块 `src/ui/kit/`（`pub(crate)`）。
触发条件（任一）：出现第二个消费者项目，或 kit 需要独立发版测试。
迁移操作：目录平移为 `crates/win-kit`，`pub(crate)` → `pub`，加 workspace 与独立 Cargo.toml（windows features 仅需 `Foundation / Gdi / GdiPlus / WindowsAndMessaging / LibraryLoader / HiDpi`，Ime 仅 text_edit 需要故不带）。

## 6. 工期估计

| 期 | 内容 | 估计 |
|---|---|---|
| 第一期步骤 1 | kit 建立 + 存量去重平移 | 0.5 天 |
| 第一期步骤 2-3 | 四 widget + 设置页重写 | 0.5–1 天 |
| 第二期 | toolbar/overlay/pin 迁移 | 1 天（可选，另行安排） |
