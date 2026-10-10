//! Tauri 命令集合。按领域拆分，`lib.rs` 只负责注册。

pub mod albums;
pub mod images;
pub mod library;
pub mod system;
pub mod tags;

pub use albums::*;
pub use images::*;
pub use library::*;
pub use system::*;
pub use tags::*;
