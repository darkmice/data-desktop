//! 商品数据研究助手 — 客户端库。
//!
//! `core` 是纯逻辑(凭证/提交流程/规则),可独立单测;`ws_client` 连接 API
//! 服务并实现提交流程的 `Signer`。Tauri 层(`app`)把它们接到前端。

pub mod app;
pub mod core;
pub mod ws_client;
pub mod ws_frame;
