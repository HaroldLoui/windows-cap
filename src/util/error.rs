use thiserror::Error;

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
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
