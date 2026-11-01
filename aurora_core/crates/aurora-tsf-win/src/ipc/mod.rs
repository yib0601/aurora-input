//! IPC 客户端：与 Rust 服务（`aurora_input`）的二进制通信。
//!
//! 原实现：`aurora_tsf/src/IPCClient.cpp`（3021 行）+ `include/IPCClient.h`。
//!
//! # 分层
//!
//! ```text
//! circuit_breaker.rs   纯状态机，无 Win32     ← 本机可测
//! （后续）transport     Named Pipe 读写        ← cfg(windows)
//! （后续）reader        async push 读线程      ← cfg(windows)
//! ```
//!
//! # 帧的编解码不在这里
//!
//! 一律走 `aurora-ipc`：下行用 `decode_*`，上行用 `upstream::encode_*`。
//! 本 crate **不许**出现第二份字节布局——那正是线格式真值表要防的事。

pub mod circuit_breaker;
#[cfg(windows)]
pub mod client;
#[cfg(windows)]
pub mod transport;

pub use circuit_breaker::{CircuitBreaker, CircuitState};
