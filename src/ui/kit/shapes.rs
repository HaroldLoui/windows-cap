//! GDI+ 基础绘制原语：圆角矩形、矩形、椭圆、线段与多边形。

use std::ptr::null_mut;

use windows::Win32::Graphics::GdiPlus::{
    FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreatePath, GdipCreatePen1,
    GdipCreateSolidFill, GdipDeleteBrush, GdipDeletePath, GdipDeletePen, GdipDrawEllipse,
    GdipDrawLine, GdipDrawLines, GdipDrawPath, GdipDrawPolygon, GdipDrawRectangle, GdipFillEllipse,
    GdipFillPath, GdipFillPolygon, GdipFillRectangle, GpGraphics, GpPath, GpPen, GpSolidFill,
    PointF, UnitPixel,
};

/// 填充圆角矩形。
pub unsafe fn fill_rounded(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: u32,
) {
    unsafe {
        let mut path: *mut GpPath = null_mut();
        if GdipCreatePath(FillModeAlternate, &mut path).0 != 0 || path.is_null() {
            return;
        }
        add_rounded_rect(path, x, y, width, height, radius);
        let mut brush: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(color, &mut brush).0 == 0 && !brush.is_null() {
            let _ = GdipFillPath(graphics, brush as *mut _, path);
            let _ = GdipDeleteBrush(brush as *mut _);
        }
        let _ = GdipDeletePath(path);
    }
}

/// 描边圆角矩形。
#[allow(clippy::too_many_arguments)]
pub unsafe fn stroke_rounded(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut path: *mut GpPath = null_mut();
        if GdipCreatePath(FillModeAlternate, &mut path).0 != 0 || path.is_null() {
            return;
        }
        add_rounded_rect(path, x, y, width, height, radius);
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawPath(graphics, pen, path);
            let _ = GdipDeletePen(pen);
        }
        let _ = GdipDeletePath(path);
    }
}

/// 把圆角矩形路径加入 GDI+ 路径。
pub unsafe fn add_rounded_rect(path: *mut GpPath, x: f32, y: f32, w: f32, h: f32, r: f32) {
    unsafe {
        let d = 2.0 * r;
        let _ = GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        let _ = GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        let _ = GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        let _ = GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        let _ = GdipClosePathFigure(path);
    }
}

/// 填充矩形。
pub unsafe fn solid_fill(graphics: *mut GpGraphics, x: f32, y: f32, w: f32, h: f32, color: u32) {
    unsafe {
        let mut brush: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(color, &mut brush).0 == 0 && !brush.is_null() {
            let _ = GdipFillRectangle(graphics, brush as *mut _, x, y, w, h);
            let _ = GdipDeleteBrush(brush as *mut _);
        }
    }
}

/// 填充椭圆。
pub unsafe fn fill_ellipse(graphics: *mut GpGraphics, x: f32, y: f32, w: f32, h: f32, color: u32) {
    unsafe {
        let mut brush: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(color, &mut brush).0 == 0 && !brush.is_null() {
            let _ = GdipFillEllipse(graphics, brush as *mut _, x, y, w, h);
            let _ = GdipDeleteBrush(brush as *mut _);
        }
    }
}

/// 描边椭圆。
pub unsafe fn stroke_ellipse(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawEllipse(graphics, pen, x, y, w, h);
            let _ = GdipDeletePen(pen);
        }
    }
}

/// 画线段。
pub unsafe fn stroke_line(
    graphics: *mut GpGraphics,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawLine(graphics, pen, x1, y1, x2, y2);
            let _ = GdipDeletePen(pen);
        }
    }
}

/// 画折线。
pub unsafe fn stroke_lines(
    graphics: *mut GpGraphics,
    points: &[PointF],
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawLines(graphics, pen, points.as_ptr(), points.len() as i32);
            let _ = GdipDeletePen(pen);
        }
    }
}

/// 画多边形轮廓。
pub unsafe fn stroke_polygon(
    graphics: *mut GpGraphics,
    points: &[PointF],
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawPolygon(graphics, pen, points.as_ptr(), points.len() as i32);
            let _ = GdipDeletePen(pen);
        }
    }
}

/// 填充多边形。
pub unsafe fn fill_polygon(graphics: *mut GpGraphics, points: &[PointF], color: u32) {
    unsafe {
        let mut brush: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(color, &mut brush).0 == 0 && !brush.is_null() {
            let _ = GdipFillPolygon(
                graphics,
                brush as *mut _,
                points.as_ptr(),
                points.len() as i32,
                FillModeAlternate,
            );
            let _ = GdipDeleteBrush(brush as *mut _);
        }
    }
}

/// 描边矩形。
pub unsafe fn stroke_rect(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawRectangle(graphics, pen, x, y, w, h);
            let _ = GdipDeletePen(pen);
        }
    }
}
