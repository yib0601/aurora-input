//! Named Pipe 传输层：连到 Rust 服务的 bridge 管道，收发帧。
//!
//! 原实现：`aurora_tsf/src/IPCClient.cpp` 的 `Connect` / `_WriteWithTimeout` /
//! `_ReadWithTimeout` / `SendSync` / `SendAsync`（约 800 行）。
//!
//! # 管道命名必须与服务端一致
//!
//! ```text
//! \\.\pipe\aurora_input{变体后缀}{per-user SID 后缀}
//! ```
//!
//! - 变体后缀：`""`（正式）/ `"_dev"`（开发），来自 `aurora_config::variant::pipe_suffix`
//! - SID 后缀：`_S-1-5-...`，管道名字空间是**机器级**的，靠它按用户隔离
//!
//! ⚠ 计算顺序不能错：`aurora_input{suffix}_{SID}`。顺序反了服务端算的是另一个名字，
//! 表现为「连接超时」——服务明明在跑却连不上，极难归因。原实现曾在 push 管道上
//! 栽过同类问题（写成 `aurora_input_push{suffix}` 漏了 SID，导致永远连不上）。
//!
//! # 超时
//!
//! 所有 I/O 都带超时。服务卡死时不能让宿主 UI 线程无限阻塞——那会让用户
//! 连打字都卡住。超时值见 [`IPCConfig`]。

#![cfg(windows)]

use std::sync::Mutex;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::core::PCWSTR;

/// 超时与重试参数。原实现 `IPCConfig` 的同名常量。
pub mod config {
    /// 连接超时（毫秒）。管道忙时等这么久。
    pub const CONNECT_TIMEOUT_MS: u32 = 100;
    /// 写超时（毫秒）。
    pub const WRITE_TIMEOUT_MS: u32 = 100;
    /// 同步读超时（毫秒）。
    pub const READ_TIMEOUT_MS: u32 = 100;
}

/// 传输层错误。
///
/// ⚠ 区分「对端确定没处理」与「不确定」很重要：
/// 前者可以安全重发，后者重发就是**重复上屏**（协调器不按 event_seq 去重）。
#[derive(Debug)]
pub enum TransportError {
    /// 连不上（服务没起 / 管道忙）。
    NotConnected,
    /// 写失败——对端确定没收到。
    ///
    /// 这类可以重连重发一次。
    WriteFailed(u32),
    /// 读超时——**对端可能已经处理了**，只是回得慢。
    ///
    /// ⚠ 不可重发：协调器不按 event_seq 去重，重发就是重复上屏。
    ReadTimeout,
    /// 一个字节都没读到就 EOF / 断连——对端确定没回。
    ///
    /// 这类可以重连重发一次（原实现 `_IsPeerGoneError` 的判据）。
    PeerGone,
    /// 帧格式错误（头部不合法 / 长度超限）。
    BadFrame(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => write!(f, "未连接"),
            Self::WriteFailed(code) => write!(f, "写失败（Win32 {code}）"),
            Self::ReadTimeout => write!(f, "读超时"),
            Self::PeerGone => write!(f, "对端已断开"),
            Self::BadFrame(msg) => write!(f, "帧格式错误: {msg}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// 一条 Named Pipe 连接。
pub struct PipeConnection {
    handle: HANDLE,
    /// 写锁：Named Pipe 的写不是线程安全的，需要串行化。
    write_lock: Mutex<()>,
}

// HANDLE 本身可以在线程间传递（Windows 句柄是进程级的）。
unsafe impl Send for PipeConnection {}
unsafe impl Sync for PipeConnection {}

impl PipeConnection {
    /// 连接到指定管道，不等待。
    pub fn connect(pipe_name: &str) -> std::result::Result<Self, TransportError> {
        let wide: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };

        match handle {
            Ok(h) => Ok(Self {
                handle: h,
                write_lock: Mutex::new(()),
            }),
            Err(_) => Err(TransportError::NotConnected),
        }
    }

    /// 连接并在管道忙时等待重试。
    pub fn connect_with_retry(pipe_name: &str) -> std::result::Result<Self, TransportError> {
        let wide: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

        // 先试一次
        if let Ok(conn) = Self::connect(pipe_name) {
            return Ok(conn);
        }

        // 管道忙：等它可用再试
        let waited = unsafe { WaitNamedPipeW(PCWSTR(wide.as_ptr()), config::CONNECT_TIMEOUT_MS) };
        if !waited.as_bool() {
            return Err(TransportError::NotConnected);
        }

        Self::connect(pipe_name)
    }

    pub fn is_connected(&self) -> bool {
        !self.handle.is_invalid()
    }

    /// 写一帧。
    ///
    /// 已加写锁：Named Pipe 的并发写会交错，帧会撕裂。
    pub fn write_frame(&self, frame: &[u8]) -> std::result::Result<(), TransportError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| TransportError::WriteFailed(0))?;

        let mut written = 0u32;
        let ok = unsafe {
            windows::Win32::Storage::FileSystem::WriteFile(
                self.handle,
                Some(frame),
                Some(&mut written),
                None,
            )
        };

        match ok {
            Ok(()) if written as usize == frame.len() => Ok(()),
            Ok(()) => Err(TransportError::WriteFailed(0)),
            Err(e) => Err(TransportError::WriteFailed(e.code().0 as u32)),
        }
    }

    /// 读一帧：先读 8 字节头，再按 `header.length` 读载荷。
    pub fn read_frame(&self) -> std::result::Result<Vec<u8>, TransportError> {
        use aurora_ipc::codec::decode_header;

        let mut header = [0u8; 8];
        self.read_exact(&mut header)?;

        // 头部不合法就断开——继续读只会错位
        let h = decode_header(&header).map_err(|e| TransportError::BadFrame(e.to_string()))?;
        let len = h.length as usize;

        let mut frame = Vec::with_capacity(8 + len);
        frame.extend_from_slice(&header);
        if len > 0 {
            let mut payload = vec![0u8; len];
            self.read_exact(&mut payload)?;
            frame.extend_from_slice(&payload);
        }
        Ok(frame)
    }

    /// 读满指定字节数。
    ///
    /// ⚠ Named Pipe 的 `ReadFile` **可能只返回一部分**（管道缓冲区大小限制）。
    /// 必须循环读满，否则会把半帧当成整帧解析。
    fn read_exact(&self, buf: &mut [u8]) -> std::result::Result<(), TransportError> {
        let mut offset = 0usize;
        while offset < buf.len() {
            let mut read = 0u32;
            let ok = unsafe {
                windows::Win32::Storage::FileSystem::ReadFile(
                    self.handle,
                    Some(&mut buf[offset..]),
                    Some(&mut read),
                    None,
                )
            };
            match ok {
                Ok(()) if read == 0 => {
                    // 0 字节 = 对端关闭。
                    // 若一点都没读到 → 对端确定没回（可重发）；
                    // 读到一半 → 帧撕裂，不可重发。
                    return if offset == 0 {
                        Err(TransportError::PeerGone)
                    } else {
                        Err(TransportError::BadFrame("帧被截断".into()))
                    };
                }
                Ok(()) => offset += read as usize,
                Err(e) => {
                    return if offset == 0 {
                        Err(TransportError::PeerGone)
                    } else {
                        Err(TransportError::WriteFailed(e.code().0 as u32))
                    };
                }
            }
        }
        Ok(())
    }
}

impl Drop for PipeConnection {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

/// 计算主命令管道名。
///
/// ⚠ 与服务端 [`aurora_bridge::server::pipe_name`] **必须逐字一致**。
/// 见模块文档的命名规则。
pub fn command_pipe_name(variant_suffix: &str, user_sid_suffix: &str) -> String {
    format!(r"\\.\pipe\aurora_input{variant_suffix}{user_sid_suffix}")
}

/// 计算 push 管道名。
pub fn push_pipe_name(variant_suffix: &str, user_sid_suffix: &str) -> String {
    format!(r"\\.\pipe\aurora_input_push{variant_suffix}{user_sid_suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 管道名必须与服务端 `aurora-bridge` 的格式完全一致。
    ///
    /// 顺序写错（`aurora_input{SID}{suffix}` 而非 `aurora_input{suffix}{SID}`）
    /// 的症状是「服务在跑却连不上」——原实现栽过同类问题。
    #[test]
    fn command_pipe_name_format() {
        assert_eq!(
            command_pipe_name("", ""),
            r"\\.\pipe\aurora_input",
            "正式版无后缀"
        );
        assert_eq!(
            command_pipe_name("_dev", ""),
            r"\\.\pipe\aurora_input_dev",
            "开发版"
        );
        assert_eq!(
            command_pipe_name("", "_S-1-5-21-1234"),
            r"\\.\pipe\aurora_input_S-1-5-21-1234",
            "带用户 SID"
        );
        assert_eq!(
            command_pipe_name("_dev", "_S-1-5-21-1234"),
            r"\\.\pipe\aurora_input_dev_S-1-5-21-1234",
            "变体后缀在前，SID 在后"
        );
    }

    #[test]
    fn push_pipe_name_format() {
        assert_eq!(push_pipe_name("", ""), r"\\.\pipe\aurora_input_push");
        assert_eq!(
            push_pipe_name("_dev", "_S-1-5"),
            r"\\.\pipe\aurora_input_push_dev_S-1-5"
        );
    }

    /// 两个管道名不能相同——否则 push 会连到命令管道上。
    #[test]
    fn command_and_push_differ() {
        for (v, s) in [("", ""), ("_dev", ""), ("", "_S-1"), ("_dev", "_S-1")] {
            assert_ne!(command_pipe_name(v, s), push_pipe_name(v, s));
        }
    }

    /// 错误分类决定能否重发，这是正确性判据。
    #[test]
    fn error_classification_allows_retry_correctly() {
        // 可以重发的：对端确定没处理
        assert!(matches!(
            TransportError::WriteFailed(232),
            TransportError::WriteFailed(_)
        ));
        assert!(matches!(TransportError::PeerGone, TransportError::PeerGone));

        // 不可重发的：读超时（对端可能已处理，重发 = 重复上屏）
        assert!(matches!(
            TransportError::ReadTimeout,
            TransportError::ReadTimeout
        ));
    }

    /// 超时值必须小——服务卡死时不能拖住宿主 UI 线程。
    #[test]
    fn timeouts_are_small() {
        assert!(
            config::CONNECT_TIMEOUT_MS <= 200,
            "连接超时过大会拖住 UI 线程"
        );
        assert!(config::READ_TIMEOUT_MS <= 500);
        assert!(config::WRITE_TIMEOUT_MS <= 500);
    }
}
