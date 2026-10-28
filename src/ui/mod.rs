//! 遮罩、选区与标注 UI。

pub mod kit;
pub mod overlay;
pub mod pin;
pub mod selection;
pub mod settings;
pub mod text_edit;
pub mod toolbar;

pub use overlay::{OutputAction, OverlayOptions, OverlayResult, select};
