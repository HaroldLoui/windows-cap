# 实施进度

> 依据：`docs/实施计划.md`  
> 更新：2026-10-21

## 阶段状态

| 阶段 | 状态 | 完成日期 | 验收记录 | 备注 |
|---|---|---|---|---|
| P0 基建 + Spike | ✅ 完成 | 2026-10-21 | [spikes.md](spikes.md) | S3 真实中文输入待 P3 人工复测 |
| P1 采集 + 输出 | ✅ 完成 | 2026-10-21 | 见下方 P1 验收 | 混合 DPI 与手动粘贴待目标机补测 |
| P2 遮罩 + 框选 | ⏸ 等待开始 | | | 用户确认 P1 效果后开始 |
| P3 标注引擎 | ⬜ 未开始 | | | |
| P4 系统集成 | ⬜ 未开始 | | | |
| P5 兼容性 + Beta | ⬜ 未开始 | | | |
| P6 V2（可选） | ⬜ 未开始 | | | |

## P1 交付清单

- `capture`：`RectI`、`Frame::crop` / `pixel` / `pixel_diff_ratio`、`CaptureBackend` trait、`GdiBackend`
- `output/clipboard`：`CF_DIB` 写入、`probe_dib` 回读、`OpenClipboard` 重试
- `output/files`：PNG 编码、临时文件 + rename 原子写、默认路径与时间戳
- `platform/test_pattern`：静态四色测试窗，用于坐标与颜色校验
- `app`：
  - `--capture` / `--region x,y,w,h` / `--output` / `--clipboard` / `--json`
  - `--soak N`：预热后统计 GDI/USER 对象与内存
  - `--selftest`：环境 → 整屏 → 测试图案颜色 → 区域 vs 裁剪 → 剪贴板 → Unicode 落盘
- 测试：7 个（坐标/裁剪/DIB 编码/PNG 往返/原子写/区域一致性）

## P1 验收记录

| 验收项 | 结果 | 证据 |
|---|---|---|
| 整屏尺寸 == 虚拟桌面 | ✅ | 2560×1440；`--capture --output target\p1\full.png` |
| 区域截图 == 整屏裁剪（逐像素） | ✅ | 测试图案窗口 400×300，diff = 0.0000%；`region_capture_equals_full_crop` 测试通过 |
| 125% / 150% / 200% 缩放一致性 | ⏳ 未测 | 本机仅 96 DPI，需在目标机补测 |
| 剪贴板图像可被标准程序读取 | ✅ | .NET `Clipboard.GetImage()` 读回 800×600；画图/Word/微信人工粘贴未测 |
| 剪贴板字节数正确 | ✅ | 400×300：480040 字节 = 40 + 400×300×4 |
| 100 次采集无句柄泄漏 | ✅ | GDI +0，USER +0 |
| 100 次采集内存回到基线 | ✅ | 预热后内存 +0.0 MB（分配器稳态 20.4 MB） |
| Unicode 路径落盘 | ✅ | `target\selftest\中文路径 测试.png`、`target\p1\截图 中文.png` |
| JSON 输出可解析 | ✅ | `ConvertFrom-Json` 通过，字段 width/height/x/y/backend/elapsed_ms/output/clipboard |
| 非法区域报错 | ✅ | `--region -10,0,100,100` 退出码 1，提示超出虚拟桌面 |
| `--selftest` 失败退出码 | ✅（设计） | 任一校验失败返回 `AppError` → 进程退出 1；本次全部通过 |

## 偏差与说明

| 项 | 记录 |
|---|---|
| 内存 soak 口径 | 每帧约 14.7 MB，分配器会保留一块容量，因此 soak 先预热 10 次再取基线；判定阈值 32 MB |
| DPI 矩阵 | 当前开发机单屏 96 DPI；混合 DPI/负坐标的坐标逻辑已有单测与图案校验，真实缩放环境待目标机验证 |
| 手动粘贴 | 用 .NET 剪贴板读取代替了"画图/Word/微信"人工验证，P2 演示时再补一次真机操作 |
| `default_path` | 已实现但暂未接入 CLI，P2/P4 使用（已标记 `allow(dead_code)`） |
| CLI 解析 | 目前为手写解析（约 120 行、零依赖）。用户允许引入第三方 crate；计划在 CLI 参数扩展（P4）或收尾阶段切换为 `clap`，避免中途返工 |
| 第三方 crate | 项目允许引入；当前依赖为 `windows`、`image`、`log`、`thiserror`、`winresource`，新增依赖需在文档记录用途 |
| git | P0 未单独提交；P1 完成时合并为首个提交 |

## 提交记录

| 日期 | 阶段 | tag | commit | 说明 |
|---|---|---|---|---|
| 2026-10-21 | P0 + P1 | `v0.2.0-p1` | `c912582` | 工程骨架、Spike、采集与输出闭环 |



## 下一步（P2 待用户确认后开始）

1. 热键隐藏窗口 + `RegisterHotKey`（冲突提示）
2. 全屏遮罩窗口（置顶、无任务栏、禁 DWM 动画）
3. 冻结帧 + 压暗 + 选区状态机（8 手柄、移动、键盘微调）
4. 尺寸标签、光标形状、取消/恢复前台
5. 松手 → 复用 P1 输出
6. 4K 端到端延迟与 60fps 目标
