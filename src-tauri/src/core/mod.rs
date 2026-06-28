//! Client core: credential handling and the order (提交) flow. Pure-ish Rust,
//! testable independently of Tauri.

pub mod access_token;
pub mod ck;
pub mod history;
pub mod notify;
pub mod order;
pub mod rules;
pub mod sysinfo;
