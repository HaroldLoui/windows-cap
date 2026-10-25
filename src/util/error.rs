//! 应用统一错误类型与结果别名。

use thiserror::Error;

/// 应用统一错误。
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Win32 调用失败: {0}")]
    Windows(#[from] windows::core::Error),

    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("图像编码失败: {0}")]
    Image(#[from] image::ImageError),

    #[error("{0}")]
    Message(String),
}

impl AppError {
    /// 构造消息错误。
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

/// 应用结果类型。
pub type Result<T> = std::result::Result<T, AppError>;
