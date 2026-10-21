# P0 Spike 结论

> 日期：2026-10-21  
> 版本：window-cap 0.1.0（P0）  
> 环境：Windows 11 Pro 10.0.26300，单显示器 2560×1440 @ 96 DPI，本地交互会话  
> 代码位置：`src/spike/mod.rs`

## 总览

| Spike | 问题 | 结论 | 关键数据 |
|---|---|---|---|
| S1 | GDI 能否正确截取虚拟桌面 | **通过** | 2560×1440，24–28 ms，非黑像素 93.9%–99.8% |
| S2 | 全屏置顶窗口能否抢焦点并收键盘 | **通过** | `SetForegroundWindow` 直接成功，注入 Esc 被窗口收到 |
| S3 | EDIT 子窗口是否有 IME 上下文 | **有条件通过** | IME 存在、上下文非空；真实中文输入需人工复测 |
| S4 | 4K 上 GDI+ 绘制 8 种形状的性能 | **通过** | 平均 1.84 ms/帧（4K），CJK 文字渲染正常 |

**D0 决策：通过，进入 P1。**

---

## S1：GDI 采集

### 命令

```powershell
cargo run -- --selftest
cargo run -- --spike capture
```

### 结果

```
虚拟桌面: 2560x1440 @ (0, 0)
显示器数量: 1
  #0 [主屏] 2560x1440 @ (0, 0) dpi=96x96 device=\\.\DISPLAY1
截屏: 2560x1440 @ (0, 0) (14745600 字节) 耗时 24.4 ms
非黑像素占比: 98.1%
已保存: target/selftest/capture.png
```

复测（release）：26.9 ms，非黑像素 99.8%。`--spike capture` 另存 `target/spike/s1-capture.png`，内容人工确认为真实桌面画面。

### 结论

- 尺寸、原点、DPI 全部正确；
- 单次 4K 级抓取（实际 1440p）约 25 ms，主要成本是内存拷贝，符合预期；
- PNG 落盘、Unicode 路径、日志均正常。

### 未覆盖（转入后续阶段）

| 项 | 计划 |
|---|---|
| 多显示器 / 负坐标 | 当前机器只有单屏，P2 在目标机器上补测 |
| 混合 DPI（125%/150%/200%） | P1 的"区域截图 == 全屏裁剪"自动化测试覆盖 |
| RDP / 锁屏 | P5 测试矩阵覆盖 |
| HDR / 独占全屏 / DRM | P5 明确提示；WGC 兜底在 P6 |

---

## S2：遮罩抢焦点与键盘

### 命令

```powershell
cargo run -- --spike focus
```

### 结果

```
[S2] 全屏遮罩已显示: 2560x1440 @ (0, 0)
[S2] SetForegroundWindow 直接结果: true; AttachThreadInput: false -> false; 最终前台: true
[S2] 注入 Esc 后窗口收到 WM_KEYDOWN: true
[S2] 结论: 通过：可抢焦点并接收键盘
```

### 结论

- CLI 启动的进程就能直接抢到前台（不依赖 `AttachThreadInput`），说明热键场景（系统已授予前台权限）只会更容易；
- 全屏 popup + `WS_EX_TOPMOST` 能正常接收 `WM_KEYDOWN`；
- `AttachThreadInput` 兜底路径本次未触发，保留代码，P2 在真实遮罩里继续验证。

### 未覆盖

- 真实热键（`RegisterHotKey`）触发的抢焦点路径 → P2；
- 全屏游戏/浏览器全屏下的焦点 → P2/P5；
- 多显示器下的全屏窗口 → P2。

---

## S3：EDIT 子窗口与 IME

### 命令

```powershell
cargo run -- --spike ime
```

### 结果

```
[S3] 键盘布局: 0x8040804，是否 IME: true
[S3] EDIT 的 IME 上下文: true
[S3] 默认 IME 窗口: true
```

### 结论

- 当前系统安装了中文 IME（`ImmIsIME` 为真）；
- 为 `EDIT` 子窗口创建后能拿到 `HIMC`，说明"用 EDIT 承载输入、系统负责 IME"的方案成立；
- GDI+ 的 CJK 渲染在 S4 中同时验证（17059 个非黑像素）。

### 未覆盖（需人工）

- 实际用微软拼音输入中文、选候选、回车提交的完整交互。Spike 窗口只存在 2 秒，无法自动化，P3 实现文字工具时人工确认一次；
- 若 P3 发现 IME 候选窗位置遮挡选区，再调整 EDIT 的定位策略。

---

## S4：GDI+ 性能

### 命令

```powershell
cargo run -- --spike gdiplus
```

### 结果

```
[S4] 4K DIB（3840x2160）8 形状 × 10 次: 总 18.4 ms，平均 1.84 ms/帧
[S4] 文字区域非黑像素: 17059（CJK 渲染正常）
```

### 结论

- 在 4K DIB 上绘制 8 种形状（线、矩形、椭圆、实心椭圆、折线、箭头两条线、中文文字）平均 **1.84 ms/帧**，远低于 16 ms 预算；
- 中文用 `Microsoft YaHei UI` + `UnitPixel` 渲染正常，说明"文字只栅格化一次"的方案可行；
- GDI+ 作为 P3 的默认渲染后端成立，无需提前引入 Direct2D。

### 说明

- 该数据包含 10 次循环的平均（首次调用开销被摊薄），真实交互中单次提交形状的耗时只会更低；
- 未测超大画布（8K）与大量形状（>100）的情况，P3 再补边界数据。

---

## D0 决策

**通过。** 四条技术路线均成立，进入 P1（采集与输出）。

带入后续阶段的验证清单：

| 项 | 阶段 |
|---|---|
| 多屏 / 负坐标 / 混合 DPI 的坐标一致性 | P1（自动化）、P2（交互） |
| 热键触发路径的抢焦点 | P2 |
| 中文输入完整交互 | P3（人工） |
| RDP / HDR / 游戏 / DRM | P5 |
| 4K 以上画布、形状数量上限 | P3 |

---

## 质量门禁记录

| 项 | 结果 |
|---|---|
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --all-targets -- -D warnings` | 通过 |
| `cargo test` | 通过（0 个测试，P1 开始补） |
| `cargo build --release` | 通过 |
| release 产物 | `target/release/window-cap.exe`，333,824 字节 |
| manifest | 已嵌入（`PerMonitorV2`、`asInvoker` 可检出） |
| 图标 / 版本信息 | 已嵌入（FileDescription / ProductName / Copyright 可读） |

## 复现命令

```powershell
cargo build --release
target\release\window-cap.exe --selftest
target\release\window-cap.exe --spike capture
target\release\window-cap.exe --spike focus
target\release\window-cap.exe --spike ime
target\release\window-cap.exe --spike gdiplus
```

日志：`%LOCALAPPDATA%\window-cap\logs\window-cap.log`
