//! 标注模型与成品表面。
//!
//! 坐标一律是 **document 本地像素**（相对选区左上角）；预览与导出共用同一块
//! `Document::surface`，因此不存在两套渲染结果。

pub mod render;

use crate::capture::{Frame, RectI};
use crate::platform::dib::Dib;
use crate::util::error::{AppError, Result};

pub type Point = (i32, i32);

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    /// GDI+ ARGB（`0xAARRGGBB`）。
    pub color: u32,
    pub width: f32,
}

impl Stroke {
    pub fn new(color: u32, width: f32) -> Self {
        Self { color, width }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShapeKind {
    Line { a: Point, b: Point },
    Arrow { a: Point, b: Point },
    Rect { a: Point, b: Point },
    Ellipse { a: Point, b: Point },
    Pen { points: Vec<Point> },
    Highlight { points: Vec<Point> },
    Text { pos: Point, text: String, size: f32 },
    Mosaic { a: Point, b: Point, block: u32 },
    Step { center: Point, index: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    pub stroke: Stroke,
}

impl Shape {
    pub fn new(kind: ShapeKind, color: u32, width: f32) -> Self {
        Self {
            kind,
            stroke: Stroke::new(color, width),
        }
    }

    pub fn is_step(&self) -> bool {
        matches!(self.kind, ShapeKind::Step { .. })
    }
}

/// 成品表面：底图 + 已提交的标注。
pub struct Document {
    width: u32,
    height: u32,
    origin: (i32, i32),
    base: Vec<u8>,
    surface: Dib,
    shapes: Vec<Shape>,
    undone: Vec<Shape>,
}

impl Document {
    /// 从冻结帧裁剪出选区，建立文档。
    pub fn from_frame(frame: &Frame, rect: RectI) -> Result<Self> {
        let crop = frame.crop(rect)?;
        Self::from_base_at(crop.width, crop.height, crop.bgra, (rect.x, rect.y))
    }

    #[allow(dead_code)] // 测试用构造
    pub fn from_base(width: u32, height: u32, base: Vec<u8>) -> Result<Self> {
        Self::from_base_at(width, height, base, (0, 0))
    }

    pub fn from_base_at(
        width: u32,
        height: u32,
        base: Vec<u8>,
        origin: (i32, i32),
    ) -> Result<Self> {
        if base.len() != (width as usize) * (height as usize) * 4 {
            return Err(AppError::msg("文档底图尺寸与数据不一致"));
        }
        render::init()?;
        let surface = Dib::from_bgra(width, height, &base)?;
        Ok(Self {
            width,
            height,
            origin,
            base,
            surface,
            shapes: Vec::new(),
            undone: Vec::new(),
        })
    }

    pub fn origin(&self) -> (i32, i32) {
        self.origin
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn surface(&self) -> &Dib {
        &self.surface
    }

    pub fn has_shapes(&self) -> bool {
        !self.shapes.is_empty()
    }

    pub fn can_undo(&self) -> bool {
        !self.shapes.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn next_step_index(&self) -> u32 {
        self.shapes.iter().filter(|shape| shape.is_step()).count() as u32 + 1
    }

    /// 提交一个标注：增量绘制到 surface。
    pub fn add(&mut self, shape: Shape) -> Result<()> {
        render::draw_shape(&mut self.surface, &shape, (0, 0))?;
        self.shapes.push(shape);
        self.undone.clear();
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        let Some(shape) = self.shapes.pop() else {
            return false;
        };
        self.undone.push(shape);
        let _ = self.rebuild();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(shape) = self.undone.pop() else {
            return false;
        };
        if render::draw_shape(&mut self.surface, &shape, (0, 0)).is_err() {
            self.undone.push(shape);
            return false;
        }
        self.shapes.push(shape);
        true
    }

    pub fn rebuild(&mut self) -> Result<()> {
        self.surface.pixels_mut().copy_from_slice(&self.base);
        for shape in &self.shapes {
            render::draw_shape(&mut self.surface, shape, (0, 0))?;
        }
        Ok(())
    }

    /// 导出为一张独立 `Frame`（origin 用于对回屏幕坐标）。
    pub fn export_frame(&self, origin: (i32, i32)) -> Frame {
        Frame {
            width: self.width,
            height: self.height,
            origin,
            bgra: self.surface.pixels().to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Document {
        Document::from_base(200, 120, vec![0u8; 200 * 120 * 4]).expect("创建文档失败")
    }

    fn pixel(document: &Document, x: i32, y: i32) -> (u8, u8, u8) {
        let surface = document.surface();
        let offset = ((y as usize) * surface.width as usize + x as usize) * 4;
        let pixels = surface.pixels();
        (pixels[offset], pixels[offset + 1], pixels[offset + 2])
    }

    #[test]
    fn line_is_rendered() {
        let mut doc = document();
        doc.add(Shape::new(
            ShapeKind::Line {
                a: (10, 10),
                b: (150, 10),
            },
            0xFFFF0000,
            3.0,
        ))
        .expect("绘制失败");
        // GDI+ ARGB 0x00FF0000 = 红，落盘 BGRA 为 (0,0,255)。
        let (b, g, r) = pixel(&doc, 80, 10);
        assert!(r > 200, "红色分量不足: {r}");
        assert!(g < 60 && b < 60);
    }

    #[test]
    fn rect_and_ellipse_are_rendered() {
        let mut doc = document();
        doc.add(Shape::new(
            ShapeKind::Rect {
                a: (10, 10),
                b: (100, 80),
            },
            0xFF00FF00,
            2.0,
        ))
        .expect("矩形失败");
        let (b, g, r) = pixel(&doc, 10, 40);
        assert!(g > 200 && r < 80 && b < 80);

        doc.add(Shape::new(
            ShapeKind::Ellipse {
                a: (120, 10),
                b: (190, 80),
            },
            0xFF0000FF,
            2.0,
        ))
        .expect("椭圆失败");
        let (b, g, _r) = pixel(&doc, 155, 10);
        assert!(b > 200 && g < 80);
    }

    #[test]
    fn undo_and_redo_restore_surface() {
        let mut doc = document();
        let shape = Shape::new(
            ShapeKind::Line {
                a: (10, 10),
                b: (150, 10),
            },
            0xFFFF0000,
            3.0,
        );
        doc.add(shape).expect("绘制失败");
        assert!(doc.has_shapes());

        assert!(doc.undo());
        assert!(!doc.has_shapes());
        let (b, g, r) = pixel(&doc, 80, 10);
        assert_eq!((b, g, r), (0, 0, 0));

        assert!(doc.redo());
        assert!(doc.has_shapes());
        let (_, _, r) = pixel(&doc, 80, 10);
        assert!(r > 200);
    }

    #[test]
    fn mosaic_makes_block_uniform() {
        // 造一张水平渐变，马赛克后块内颜色应一致。
        let mut base = vec![0u8; 64 * 64 * 4];
        for y in 0..64 {
            for x in 0..64 {
                let offset = (y * 64 + x) * 4;
                base[offset] = (x * 4) as u8;
                base[offset + 1] = (x * 4) as u8;
                base[offset + 2] = (x * 4) as u8;
                base[offset + 3] = 255;
            }
        }
        let mut doc = Document::from_base(64, 64, base).expect("创建失败");
        doc.add(Shape::new(
            ShapeKind::Mosaic {
                a: (0, 0),
                b: (64, 64),
                block: 8,
            },
            0xFFFFFFFF,
            1.0,
        ))
        .expect("马赛克失败");

        let a = pixel(&doc, 1, 1);
        let b = pixel(&doc, 6, 6);
        assert_eq!(a, b, "同一个马赛克块内颜色应该一致");
    }

    #[test]
    fn pen_and_step_are_rendered() {
        let mut doc = document();
        doc.add(Shape::new(
            ShapeKind::Pen {
                points: vec![(10, 100), (50, 100), (90, 100)],
            },
            0xFF00FF00,
            3.0,
        ))
        .expect("画笔失败");
        let (b, g, r) = pixel(&doc, 50, 100);
        assert!(g > 180 && r < 100 && b < 100);

        doc.add(Shape::new(
            ShapeKind::Step {
                center: (150, 80),
                index: 1,
            },
            0xFFFF0000,
            8.0,
        ))
        .expect("序号失败");
        let mut red_found = false;
        for y in 60..100 {
            for x in 130..170 {
                let (_, _, r) = pixel(&doc, x, y);
                if r > 180 {
                    red_found = true;
                }
            }
        }
        assert!(red_found, "序号圆点未渲染");
    }

    #[test]
    fn export_matches_surface() {
        let mut doc = document();
        doc.add(Shape::new(
            ShapeKind::Line {
                a: (5, 5),
                b: (100, 100),
            },
            0xFFFFFFFF,
            2.0,
        ))
        .expect("绘制失败");
        let frame = doc.export_frame((100, 50));
        assert_eq!(frame.width, doc.width());
        assert_eq!(frame.height, doc.height());
        assert_eq!(frame.origin, (100, 50));
        assert_eq!(frame.bgra, doc.surface().pixels());
    }
}
