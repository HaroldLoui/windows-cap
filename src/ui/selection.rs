//! 选区状态机：与窗口/绘制解耦，便于单元测试。
//!
//! 坐标一律是**屏幕坐标**（虚拟桌面物理像素）。

use crate::capture::{RectI, VirtualScreen};

/// 8 个缩放手柄。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle::NorthWest,
        Handle::North,
        Handle::NorthEast,
        Handle::East,
        Handle::SouthEast,
        Handle::South,
        Handle::SouthWest,
        Handle::West,
    ];

    /// 手柄在选区矩形上的位置。
    pub fn position(self, rect: RectI) -> (i32, i32) {
        let left = rect.x;
        let center_x = rect.x + rect.width / 2;
        let right = rect.x + rect.width;
        let top = rect.y;
        let center_y = rect.y + rect.height / 2;
        let bottom = rect.y + rect.height;
        match self {
            Handle::NorthWest => (left, top),
            Handle::North => (center_x, top),
            Handle::NorthEast => (right, top),
            Handle::East => (right, center_y),
            Handle::SouthEast => (right, bottom),
            Handle::South => (center_x, bottom),
            Handle::SouthWest => (left, bottom),
            Handle::West => (left, center_y),
        }
    }
}

/// 一次鼠标抬起后的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndResult {
    /// 新建了选区。
    Selected,
    /// 移动/缩放了已有选区。
    Adjusted,
    /// 点在选区外但没有拖动：恢复原选区。
    Restored,
    /// 没有选区且只是单击：应当取消整个遮罩。
    Cancelled,
    /// 没有发生交互。
    Idle,
}

#[derive(Clone, Copy, Debug, Default)]
enum Mode {
    #[default]
    Idle,
    Creating {
        anchor: (i32, i32),
        cursor: (i32, i32),
    },
    Moving {
        start: (i32, i32),
        origin: RectI,
    },
    Resizing {
        handle: Handle,
        start: (i32, i32),
        origin: RectI,
    },
}

const HANDLE_TOLERANCE: i32 = 6;
const MIN_SIZE: i32 = 3;

#[derive(Default)]
pub struct SelectionState {
    rect: Option<RectI>,
    previous: Option<RectI>,
    mode: Mode,
}

impl SelectionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 用于脚本化测试：直接设置一个选区。
    pub fn set_selection(&mut self, rect: RectI) {
        self.rect = Some(rect);
        self.previous = None;
        self.mode = Mode::Idle;
    }

    pub fn rect(&self) -> Option<RectI> {
        self.current_rect()
    }

    /// 是否有正在进行的拖拽（创建/移动/缩放）。
    pub fn is_active(&self) -> bool {
        !matches!(self.mode, Mode::Idle)
    }

    /// 正在绘制/生效的选区（创建过程中返回实时矩形）。
    pub fn current_rect(&self) -> Option<RectI> {
        match self.mode {
            Mode::Creating { anchor, cursor } => Some(normalized(anchor, cursor)),
            _ => self.rect,
        }
    }

    pub fn begin(&mut self, x: i32, y: i32, bounds: VirtualScreen) {
        let point = clamp_point(x, y, bounds);

        if let Some(rect) = self.rect {
            if let Some(handle) = Self::handle_at(rect, point.0, point.1, HANDLE_TOLERANCE) {
                self.mode = Mode::Resizing {
                    handle,
                    start: point,
                    origin: rect,
                };
                return;
            }
            if contains(rect, point.0, point.1) {
                self.mode = Mode::Moving {
                    start: point,
                    origin: rect,
                };
                return;
            }
        }

        // 在空白处按下：开始新选区（先保留旧选区，单击未拖动时恢复）。
        self.previous = self.rect.take();
        self.mode = Mode::Creating {
            anchor: point,
            cursor: point,
        };
    }

    pub fn update(&mut self, x: i32, y: i32, bounds: VirtualScreen) {
        let point = clamp_point(x, y, bounds);
        match self.mode {
            Mode::Creating { anchor, .. } => {
                self.mode = Mode::Creating {
                    anchor,
                    cursor: point,
                };
            }
            Mode::Moving { start, origin } => {
                self.rect = Some(move_rect(
                    origin,
                    point.0 - start.0,
                    point.1 - start.1,
                    bounds,
                ));
            }
            Mode::Resizing {
                handle,
                start,
                origin,
            } => {
                self.rect = Some(resize_rect(
                    origin,
                    handle,
                    point.0 - start.0,
                    point.1 - start.1,
                    bounds,
                ));
            }
            Mode::Idle => {}
        }
    }

    pub fn end(&mut self, _bounds: VirtualScreen) -> EndResult {
        let mode = std::mem::replace(&mut self.mode, Mode::Idle);
        match mode {
            Mode::Creating { anchor, cursor } => {
                let rect = normalized(anchor, cursor);
                if rect.width >= MIN_SIZE && rect.height >= MIN_SIZE {
                    self.rect = Some(rect);
                    self.previous = None;
                    EndResult::Selected
                } else {
                    self.rect = self.previous.take();
                    if self.rect.is_some() {
                        EndResult::Restored
                    } else {
                        EndResult::Cancelled
                    }
                }
            }
            Mode::Moving { .. } | Mode::Resizing { .. } => {
                self.previous = None;
                EndResult::Adjusted
            }
            Mode::Idle => EndResult::Idle,
        }
    }

    pub fn move_by(&mut self, dx: i32, dy: i32, bounds: VirtualScreen) {
        if let Some(rect) = self.rect {
            self.rect = Some(move_rect(rect, dx, dy, bounds));
        }
    }

    pub fn resize_by(&mut self, dx: i32, dy: i32, bounds: VirtualScreen) {
        if let Some(rect) = self.rect {
            let max_width = bounds.right() - rect.x;
            let max_height = bounds.bottom() - rect.y;
            let width = (rect.width + dx).clamp(1, max_width);
            let height = (rect.height + dy).clamp(1, max_height);
            self.rect = Some(RectI::new(rect.x, rect.y, width, height));
        }
    }

    pub fn handle_at(rect: RectI, x: i32, y: i32, tolerance: i32) -> Option<Handle> {
        Handle::ALL.into_iter().find(|handle| {
            let (handle_x, handle_y) = handle.position(rect);
            (x - handle_x).abs() <= tolerance && (y - handle_y).abs() <= tolerance
        })
    }
}

fn clamp_point(x: i32, y: i32, bounds: VirtualScreen) -> (i32, i32) {
    (
        x.clamp(bounds.x, bounds.right() - 1),
        y.clamp(bounds.y, bounds.bottom() - 1),
    )
}

fn normalized(a: (i32, i32), b: (i32, i32)) -> RectI {
    let left = a.0.min(b.0);
    let top = a.1.min(b.1);
    RectI::new(left, top, (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

fn contains(rect: RectI, x: i32, y: i32) -> bool {
    x >= rect.x && y >= rect.y && x < rect.x + rect.width && y < rect.y + rect.height
}

fn move_rect(origin: RectI, dx: i32, dy: i32, bounds: VirtualScreen) -> RectI {
    let max_x = bounds.right() - origin.width;
    let max_y = bounds.bottom() - origin.height;
    let x = (origin.x + dx).clamp(bounds.x, max_x);
    let y = (origin.y + dy).clamp(bounds.y, max_y);
    RectI::new(x, y, origin.width, origin.height)
}

fn resize_rect(origin: RectI, handle: Handle, dx: i32, dy: i32, bounds: VirtualScreen) -> RectI {
    let mut left = origin.x;
    let mut top = origin.y;
    let mut right = origin.x + origin.width;
    let mut bottom = origin.y + origin.height;

    match handle {
        Handle::NorthWest => {
            left += dx;
            top += dy;
        }
        Handle::North => top += dy,
        Handle::NorthEast => {
            right += dx;
            top += dy;
        }
        Handle::East => right += dx,
        Handle::SouthEast => {
            right += dx;
            bottom += dy;
        }
        Handle::South => bottom += dy,
        Handle::SouthWest => {
            left += dx;
            bottom += dy;
        }
        Handle::West => left += dx,
    }

    left = left.clamp(bounds.x, bounds.right() - 1);
    top = top.clamp(bounds.y, bounds.bottom() - 1);
    right = right.clamp(bounds.x + 1, bounds.right());
    bottom = bottom.clamp(bounds.y + 1, bounds.bottom());

    if right <= left {
        if matches!(handle, Handle::NorthWest | Handle::West | Handle::SouthWest) {
            left = right - 1;
        } else {
            right = left + 1;
        }
    }
    if bottom <= top {
        if matches!(
            handle,
            Handle::NorthWest | Handle::North | Handle::NorthEast
        ) {
            top = bottom - 1;
        } else {
            bottom = top + 1;
        }
    }

    RectI::new(left, top, right - left, bottom - top)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> VirtualScreen {
        VirtualScreen {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        }
    }

    #[test]
    fn drag_creates_selection() {
        let mut state = SelectionState::new();
        state.begin(100, 100, bounds());
        state.update(300, 250, bounds());
        assert_eq!(state.end(bounds()), EndResult::Selected);
        assert_eq!(state.rect(), Some(RectI::new(100, 100, 200, 150)));
    }

    #[test]
    fn reversed_drag_is_normalized() {
        let mut state = SelectionState::new();
        state.begin(300, 250, bounds());
        state.update(100, 100, bounds());
        assert_eq!(state.end(bounds()), EndResult::Selected);
        assert_eq!(state.rect(), Some(RectI::new(100, 100, 200, 150)));
    }

    #[test]
    fn click_without_drag_cancels() {
        let mut state = SelectionState::new();
        state.begin(100, 100, bounds());
        state.update(101, 100, bounds());
        assert_eq!(state.end(bounds()), EndResult::Cancelled);
        assert_eq!(state.rect(), None);
    }

    #[test]
    fn drag_is_clamped_to_bounds() {
        let mut state = SelectionState::new();
        state.begin(900, 700, bounds());
        state.update(2000, 2000, bounds());
        assert_eq!(state.end(bounds()), EndResult::Selected);
        assert_eq!(state.rect(), Some(RectI::new(900, 700, 99, 99)));
    }

    #[test]
    fn move_selection_keeps_size_and_clamps() {
        let mut state = SelectionState::new();
        state.set_selection(RectI::new(100, 100, 200, 150));
        state.begin(150, 150, bounds());
        state.update(200, 180, bounds());
        assert_eq!(state.end(bounds()), EndResult::Adjusted);
        assert_eq!(state.rect(), Some(RectI::new(150, 130, 200, 150)));

        state.begin(250, 205, bounds());
        state.update(-500, -500, bounds());
        assert_eq!(state.end(bounds()), EndResult::Adjusted);
        assert_eq!(state.rect(), Some(RectI::new(0, 0, 200, 150)));
    }

    #[test]
    fn resize_from_south_east() {
        let mut state = SelectionState::new();
        state.set_selection(RectI::new(100, 100, 200, 150));
        let handle = Handle::SouthEast.position(RectI::new(100, 100, 200, 150));
        state.begin(handle.0, handle.1, bounds());
        state.update(handle.0 + 50, handle.1 + 30, bounds());
        assert_eq!(state.end(bounds()), EndResult::Adjusted);
        assert_eq!(state.rect(), Some(RectI::new(100, 100, 250, 180)));
    }

    #[test]
    fn click_outside_restores_previous_selection() {
        let mut state = SelectionState::new();
        state.set_selection(RectI::new(100, 100, 200, 150));
        state.begin(600, 600, bounds());
        assert_eq!(state.end(bounds()), EndResult::Restored);
        assert_eq!(state.rect(), Some(RectI::new(100, 100, 200, 150)));
    }

    #[test]
    fn arrows_move_and_resize() {
        let mut state = SelectionState::new();
        state.set_selection(RectI::new(100, 100, 200, 150));
        state.move_by(5, -10, bounds());
        assert_eq!(state.rect(), Some(RectI::new(105, 90, 200, 150)));
        state.resize_by(20, 10, bounds());
        assert_eq!(state.rect(), Some(RectI::new(105, 90, 220, 160)));
        state.resize_by(-1000, -1000, bounds());
        assert_eq!(state.rect(), Some(RectI::new(105, 90, 1, 1)));
    }
}
