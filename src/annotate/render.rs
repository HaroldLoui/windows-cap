//! GDI+ 渲染：把形状光栅化到文档 DIB 或窗口 DC。

use std::ptr::null_mut;
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::GdiPlus::{
    FillModeAlternate, FlushIntentionSync, FontStyleRegular, GdipCreateFont,
    GdipCreateFontFamilyFromName, GdipCreateFromHDC, GdipCreatePen1, GdipCreateSolidFill,
    GdipCreateStringFormat, GdipDeleteBrush, GdipDeleteFont, GdipDeleteFontFamily,
    GdipDeleteGraphics, GdipDeletePen, GdipDeleteStringFormat, GdipDrawEllipse, GdipDrawLine,
    GdipDrawLines, GdipDrawRectangle, GdipDrawString, GdipFillEllipse, GdipFillPolygon, GdipFlush,
    GdipSetPageUnit, GdipSetSmoothingMode, GdipSetStringFormatAlign, GdipSetStringFormatFlags,
    GdipSetStringFormatLineAlign, GdipSetTextRenderingHint, GdipTranslateWorldTransform,
    GdiplusStartup, GdiplusStartupInput, GdiplusStartupOutput, GpBrush, GpFont, GpFontCollection,
    GpFontFamily, GpGraphics, GpPen, GpSolidFill, GpStringFormat, MatrixOrderAppend, PointF, RectF,
    SmoothingModeAntiAlias, Status, StringAlignmentCenter, StringFormatFlagsNoWrap,
    TextRenderingHintAntiAliasGridFit, UnitPixel,
};
use windows::core::{PCWSTR, w};

use crate::annotate::{Point, Shape, ShapeKind};
use crate::platform::dib::Dib;
use crate::util::error::{AppError, Result};

static INIT: Once = Once::new();
static TOKEN: AtomicUsize = AtomicUsize::new(0);

/// 进程级 GDI+ 初始化（幂等）。
pub fn init() -> Result<()> {
    INIT.call_once(|| unsafe {
        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut output = GdiplusStartupOutput::default();
        let status = GdiplusStartup(&mut token, &input, &mut output);
        if status.0 == 0 {
            TOKEN.store(token, Ordering::SeqCst);
        }
    });

    if TOKEN.load(Ordering::SeqCst) == 0 {
        Err(AppError::msg("GDI+ 初始化失败"))
    } else {
        Ok(())
    }
}

/// 把形状绘制到文档表面；`offset` 仅用于矢量形状。
pub fn draw_shape(surface: &mut Dib, shape: &Shape, offset: Point) -> Result<()> {
    init()?;
    match &shape.kind {
        ShapeKind::Mosaic { a, b, block } => {
            apply_mosaic(surface, *a, *b, *block);
            Ok(())
        }
        _ => draw_vector(surface.dc, shape, offset),
    }
}

/// 把矢量形状绘制到任意 DC。
pub fn draw_vector(hdc: HDC, shape: &Shape, offset: Point) -> Result<()> {
    init()?;
    unsafe {
        let mut graphics: *mut GpGraphics = null_mut();
        check(GdipCreateFromHDC(hdc, &mut graphics), "GdipCreateFromHDC")?;

        let result = (|| -> Result<()> {
            let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
            // 统一使用像素单位，保证预览与导出一致。
            let _ = GdipSetPageUnit(graphics, UnitPixel);
            let _ = GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
            if offset != (0, 0) {
                let _ = GdipTranslateWorldTransform(
                    graphics,
                    offset.0 as f32,
                    offset.1 as f32,
                    MatrixOrderAppend,
                );
            }
            draw_kind(graphics, shape)?;
            let _ = GdipFlush(graphics, FlushIntentionSync);
            Ok(())
        })();

        let _ = GdipDeleteGraphics(graphics);
        result
    }
}

/// 按形状类型绘制。
fn draw_kind(graphics: *mut GpGraphics, shape: &Shape) -> Result<()> {
    // 步骤 1：绘制基础矢量形状，临时 GDI+ 对象用后立即释放。
    let color = shape.stroke.color;
    let width = shape.stroke.width.max(1.0);

    unsafe {
        match &shape.kind {
            ShapeKind::Line { a, b } => {
                let mut pen: *mut GpPen = null_mut();
                check(
                    GdipCreatePen1(color, width, UnitPixel, &mut pen),
                    "GdipCreatePen1",
                )?;
                let status = GdipDrawLine(
                    graphics, pen, a.0 as f32, a.1 as f32, b.0 as f32, b.1 as f32,
                );
                let _ = GdipDeletePen(pen);
                check(status, "GdipDrawLine")?;
            }
            ShapeKind::Arrow { a, b } => {
                // 实心三角箭头：主线只画到箭头底边中心，避免尖端出现两段线拼接感。
                let (p1, p2) = arrow_head(*a, *b, width);
                let base = ((p1.0 + p2.0) / 2.0, (p1.1 + p2.1) / 2.0);
                let mut pen: *mut GpPen = null_mut();
                check(
                    GdipCreatePen1(color, width, UnitPixel, &mut pen),
                    "GdipCreatePen1",
                )?;
                let status = GdipDrawLine(graphics, pen, a.0 as f32, a.1 as f32, base.0, base.1);
                let _ = GdipDeletePen(pen);
                check(status, "GdipDrawLine")?;

                let mut brush: *mut GpSolidFill = null_mut();
                check(
                    GdipCreateSolidFill(color, &mut brush),
                    "GdipCreateSolidFill",
                )?;
                let points = [
                    PointF {
                        X: b.0 as f32,
                        Y: b.1 as f32,
                    },
                    PointF { X: p1.0, Y: p1.1 },
                    PointF { X: p2.0, Y: p2.1 },
                ];
                let status = GdipFillPolygon(
                    graphics,
                    brush as *mut GpBrush,
                    points.as_ptr(),
                    3,
                    FillModeAlternate,
                );
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
                check(status, "GdipFillPolygon")?;
            }
            ShapeKind::Rect { a, b } => {
                let mut pen: *mut GpPen = null_mut();
                check(
                    GdipCreatePen1(color, width, UnitPixel, &mut pen),
                    "GdipCreatePen1",
                )?;
                let (x, y, w, h) = normalized(*a, *b);
                let status = GdipDrawRectangle(graphics, pen, x, y, w, h);
                let _ = GdipDeletePen(pen);
                check(status, "GdipDrawRectangle")?;
            }
            ShapeKind::Ellipse { a, b } => {
                let mut pen: *mut GpPen = null_mut();
                check(
                    GdipCreatePen1(color, width, UnitPixel, &mut pen),
                    "GdipCreatePen1",
                )?;
                let (x, y, w, h) = normalized(*a, *b);
                let status = GdipDrawEllipse(graphics, pen, x, y, w, h);
                let _ = GdipDeletePen(pen);
                check(status, "GdipDrawEllipse")?;
            }
            ShapeKind::Pen { points } | ShapeKind::Highlight { points } => {
                if points.len() < 2 {
                    return Ok(());
                }
                let mut pen: *mut GpPen = null_mut();
                check(
                    GdipCreatePen1(color, width, UnitPixel, &mut pen),
                    "GdipCreatePen1",
                )?;
                let converted: Vec<PointF> = points.iter().map(|point| pf(*point)).collect();
                let status =
                    GdipDrawLines(graphics, pen, converted.as_ptr(), converted.len() as i32);
                let _ = GdipDeletePen(pen);
                check(status, "GdipDrawLines")?;
            }
            // 步骤 2：绘制文字标注。
            ShapeKind::Text {
                pos,
                text,
                size,
                max_width,
            } => {
                draw_text_at(
                    graphics,
                    text,
                    pos.0 as f32,
                    pos.1 as f32,
                    *size,
                    color,
                    *max_width,
                )?;
            }
            // 步骤 3：绘制序号；马赛克由 draw_shape 的像素路径处理。
            ShapeKind::Step { center, index } => {
                let label = index.to_string();
                let digits = label.chars().count();
                let base_radius = (width * 2.0).max(14.0);
                // 两位数/三位数时放大圆圈、缩小字号，保证数字完整居中。
                let radius = match digits {
                    1 => base_radius,
                    2 => base_radius * 1.3,
                    _ => base_radius * 1.6,
                };
                let font_size = match digits {
                    1 => radius * 1.25,
                    2 => radius * 0.95,
                    _ => radius * 0.75,
                };
                let mut brush: *mut GpSolidFill = null_mut();
                check(
                    GdipCreateSolidFill(color, &mut brush),
                    "GdipCreateSolidFill",
                )?;
                let status = GdipFillEllipse(
                    graphics,
                    brush as *mut GpBrush,
                    center.0 as f32 - radius,
                    center.1 as f32 - radius,
                    radius * 2.0,
                    radius * 2.0,
                );
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
                check(status, "GdipFillEllipse")?;

                let rect = RectF {
                    X: center.0 as f32 - radius,
                    Y: center.1 as f32 - radius,
                    Width: radius * 2.0,
                    Height: radius * 2.0,
                };
                draw_text_centered(graphics, &label, rect, font_size, 0xFFFF_FFFF)?;
            }
            ShapeKind::Mosaic { .. } => {}
        }
    }

    Ok(())
}

/// 在指定位置绘制文本。
fn draw_text_at(
    graphics: *mut GpGraphics,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    color: u32,
    max_width: f32,
) -> Result<()> {
    let rect = RectF {
        X: x,
        Y: y,
        Width: if max_width > 0.0 { max_width } else { 4000.0 },
        Height: 4000.0,
    };
    draw_text(graphics, text, rect, size, color, false, max_width > 0.0)
}

/// 在矩形内居中绘制文本。
fn draw_text_centered(
    graphics: *mut GpGraphics,
    text: &str,
    rect: RectF,
    size: f32,
    color: u32,
) -> Result<()> {
    draw_text(graphics, text, rect, size, color, true, false)
}

/// 绘制文本并释放临时 GDI+ 对象。
fn draw_text(
    graphics: *mut GpGraphics,
    text: &str,
    rect: RectF,
    size: f32,
    color: u32,
    centered: bool,
    wrap: bool,
) -> Result<()> {
    unsafe {
        let mut family: *mut GpFontFamily = null_mut();
        check(
            GdipCreateFontFamilyFromName(
                w!("Microsoft YaHei UI"),
                null_mut::<GpFontCollection>(),
                &mut family,
            ),
            "GdipCreateFontFamilyFromName",
        )?;

        let mut font: *mut GpFont = null_mut();
        let font_status = GdipCreateFont(family, size, FontStyleRegular.0, UnitPixel, &mut font);

        let mut format: *mut GpStringFormat = null_mut();
        let format_status = GdipCreateStringFormat(0, 0, &mut format);
        if !wrap {
            let _ = GdipSetStringFormatFlags(format, StringFormatFlagsNoWrap.0);
        }
        if centered {
            let _ = GdipSetStringFormatAlign(format, StringAlignmentCenter);
            let _ = GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
        }

        let mut brush: *mut GpSolidFill = null_mut();
        let brush_status = GdipCreateSolidFill(color, &mut brush);

        let mut status = font_status;
        if status.0 == 0 {
            status = format_status;
        }
        if status.0 == 0 {
            status = brush_status;
        }
        if status.0 == 0 {
            let wide: Vec<u16> = text.encode_utf16().collect();
            status = GdipDrawString(
                graphics,
                PCWSTR::from_raw(wide.as_ptr()),
                wide.len() as i32,
                font,
                &rect,
                format,
                brush as *const GpBrush,
            );
        }

        if !format.is_null() {
            let _ = GdipDeleteStringFormat(format);
        }
        if !brush.is_null() {
            let _ = GdipDeleteBrush(brush as *mut GpBrush);
        }
        if !font.is_null() {
            let _ = GdipDeleteFont(font);
        }
        if !family.is_null() {
            let _ = GdipDeleteFontFamily(family);
        }

        check(status, "GdipDrawString")
    }
}

/// 计算箭头两翼端点。
fn arrow_head(a: Point, b: Point, width: f32) -> ((f32, f32), (f32, f32)) {
    let dx = (a.0 - b.0) as f32;
    let dy = (a.1 - b.1) as f32;
    let length = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / length;
    let uy = dy / length;
    let head = (width * 3.0).max(12.0);
    let (sin, cos) = 0.6f32.sin_cos();
    let p1 = (
        b.0 as f32 + head * (ux * cos - uy * sin),
        b.1 as f32 + head * (ux * sin + uy * cos),
    );
    let p2 = (
        b.0 as f32 + head * (ux * cos + uy * sin),
        b.1 as f32 + head * (-ux * sin + uy * cos),
    );
    (p1, p2)
}

/// 由两个对角点返回左上角与宽高。
fn normalized(a: Point, b: Point) -> (f32, f32, f32, f32) {
    let left = a.0.min(b.0) as f32;
    let top = a.1.min(b.1) as f32;
    let width = (a.0 - b.0).unsigned_abs() as f32;
    let height = (a.1 - b.1).unsigned_abs() as f32;
    (left, top, width, height)
}

/// 文档坐标点转为 GDI+ 浮点坐标。
fn pf(point: Point) -> PointF {
    PointF {
        X: point.0 as f32,
        Y: point.1 as f32,
    }
}

/// 将 GDI+ 状态转为 Result。
fn check(status: Status, what: &str) -> Result<()> {
    if status.0 == 0 {
        Ok(())
    } else {
        Err(AppError::msg(format!(
            "GDI+ {what} 失败: status={}",
            status.0
        )))
    }
}

/// 把矩形区域马赛克化。
fn apply_mosaic(surface: &mut Dib, a: Point, b: Point, block: u32) {
    let width = surface.width as i32;
    let height = surface.height as i32;
    let left = a.0.min(b.0).clamp(0, width);
    let top = a.1.min(b.1).clamp(0, height);
    let right = a.0.max(b.0).clamp(0, width);
    let bottom = a.1.max(b.1).clamp(0, height);
    if right <= left || bottom <= top {
        return;
    }

    let block = block.max(2) as i32;
    let pixels = surface.pixels_mut();

    let mut y = top;
    while y < bottom {
        let y_end = (y + block).min(bottom);
        let mut x = left;
        while x < right {
            let x_end = (x + block).min(right);
            let mut sums = [0u32; 3];
            let mut count = 0u32;
            for row in y..y_end {
                for column in x..x_end {
                    let offset = ((row as usize) * (width as usize) + column as usize) * 4;
                    sums[0] += pixels[offset] as u32;
                    sums[1] += pixels[offset + 1] as u32;
                    sums[2] += pixels[offset + 2] as u32;
                    count += 1;
                }
            }
            let average = [
                (sums[0] / count) as u8,
                (sums[1] / count) as u8,
                (sums[2] / count) as u8,
            ];
            for row in y..y_end {
                for column in x..x_end {
                    let offset = ((row as usize) * (width as usize) + column as usize) * 4;
                    pixels[offset] = average[0];
                    pixels[offset + 1] = average[1];
                    pixels[offset + 2] = average[2];
                    pixels[offset + 3] = 255;
                }
            }
            x += block;
        }
        y += block;
    }
}
