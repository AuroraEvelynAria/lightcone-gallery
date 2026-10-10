//! 统一的错误类型：所有 Tauri 命令都返回 `AppResult<T>`。

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("数据库错误: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("文件系统错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("图像处理错误: {0}")]
    Image(#[from] image::ImageError),

    #[error("Tauri 运行时错误: {0}")]
    Tauri(#[from] tauri::Error),

    #[error("无效路径: {0}")]
    InvalidPath(String),

    #[error("未找到: {0}")]
    NotFound(String),

    #[error("参数非法: {0}")]
    InvalidInput(String),

    #[error("{0}")]
    Other(String),
}

/// 序列化为字符串交给前端；前端 `invoke` 的 reject 值即为该文案。
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
