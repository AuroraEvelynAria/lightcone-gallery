//! Tauri 命令集合。按领域拆分，`lib.rs` 只负责注册。

pub mod albums;
pub mod artworks;
pub mod images;
pub mod library;
pub mod media;
pub mod pixiv;
pub mod system;
pub mod tag_galleries;
pub mod tags;

pub use albums::*;
pub use artworks::*;
pub use images::*;
pub use library::*;
pub use media::*;
pub use pixiv::*;
pub use system::*;
pub use tag_galleries::*;
pub use tags::*;
