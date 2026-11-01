//! IPC 客户端：把传输层、熔断器、帧编解码组合成一个可用的门面。
//!
//! 原实现：`aurora_tsf/src/IPCClient.cpp`（3021 行）的核心路径。
//!
//! # 三层职责
//!
//! ```text
//! client.rs        ← 本文件：门面。连接管理 + 熔断 + 发送/接收
//! transport.rs     ← Named Pipe 读写
//! circuit_breaker  ← 熔断状态机
//! ```
//!
//! 帧的字节布局**不在这里**：上行用 `aurora_ipc::upstream::encode_*`，
//! 下行用 `aurora_ipc::codec::decode_*`。本 crate 不许出现第二份布局。
//!
//! # 同步 vs 异步
//!
//! | 方式 | 用什么 | 何时用 |
//! |---|---|---|
//! | 同步 | [`IpcClient::send_sync`] | 需要响应才能决策（按键、焦点） |
//! | 异步 | [`IpcClient::send_async`] | 不需要响应（IME_ACTIVATED、CaretUpdate） |
//!
//! ⚠ 异步帧必须带 `ASYNC_FLAG`：服务端据此不回应答。忘了带会让客户端
//! 等一个永远不来的响应，卡到超时。

#![cfg(windows)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use aurora_ipc::protocol::*;

use super::circuit_breaker::CircuitBreaker;
use super::transport::{PipeConnection, TransportError, command_pipe_name};

/// IPC 客户端配置。
#[derive(Debug, Clone)]
pub struct IpcConfig {
    /// 管道变体后缀（`""` / `"_dev"`）。
    pub variant_suffix: String,
    /// per-user SID 后缀（`"_S-1-5-..."`），用于按用户隔离管道。
    pub sid_suffix: String,
}

impl IpcConfig {
    pub fn new(variant_suffix: impl Into<String>, sid_suffix: impl Into<String>) -> Self {
        Self {
            variant_suffix: variant_suffix.into(),
            sid_suffix: sid_suffix.into(),
        }
    }

    /// 完整管道名。
    pub fn pipe_name(&self) -> String {
        command_pipe_name(&self.variant_suffix, &self.sid_suffix)
    }
}

/// 一次同步请求的结果。
#[derive(Debug)]
pub enum SyncOutcome {
    /// 收到响应帧（已解码头部，载荷原样）。
    Response(Vec<u8>),
    /// 服务不可用（熔断中 / 连不上）。
    ServiceUnavailable,
    /// 传输错误。
    Transport(TransportError),
}

/// 客户端统计（诊断）。
#[derive(Debug, Default)]
pub struct Stats {
    pub frames_sent: AtomicU64,
    pub frames_received: AtomicU64,
    pub connect_attempts: AtomicU64,
}

/// IPC 客户端。
pub struct IpcClient {
    config: IpcConfig,
    conn: Mutex<Option<PipeConnection>>,
    breaker: Mutex<CircuitBreaker>,
    /// 单调递增的按键事件序号。服务端用它排序与判断丢帧。
    event_seq: AtomicU32,
    /// 本进程是否已与服务端握手过（收过 SERVICE_READY）。
    ready: AtomicBool,
    stats: Stats,
}

impl std::fmt::Debug for IpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcClient")
            .field("pipe", &self.config.pipe_name())
            .field("ready", &self.ready.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl IpcClient {
    pub fn new(config: IpcConfig) -> Self {
        Self {
            config,
            conn: Mutex::new(None),
            breaker: Mutex::new(CircuitBreaker::new()),
            event_seq: AtomicU32::new(0),
            ready: AtomicBool::new(false),
            stats: Stats::default(),
        }
    }

    pub fn config(&self) -> &IpcConfig {
        &self.config
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// 取下一个事件序号。
    pub fn next_event_seq(&self) -> u16 {
        self.event_seq.fetch_add(1, Ordering::SeqCst) as u16
    }

    /// 确保已连接。返回是否可用。
    ///
    /// 熔断打开时直接返回 `false`，不尝试连接——这正是熔断的意义：
    /// 服务死了时别让每个按键都付一次连接超时。
    pub fn ensure_connected(&self) -> bool {
        let now = now_ms();

        // 熔断检查
        {
            let Ok(mut b) = self.breaker.lock() else {
                return false;
            };
            if !b.should_attempt(now) {
                return false;
            }
        }

        // 已连接？
        {
            let Ok(guard) = self.conn.lock() else {
                return false;
            };
            if let Some(c) = guard.as_ref() {
                if c.is_connected() {
                    return true;
                }
            }
        }

        // 尝试连接
        self.stats.connect_attempts.fetch_add(1, Ordering::SeqCst);
        match PipeConnection::connect_with_retry(&self.config.pipe_name()) {
            Ok(conn) => {
                if let Ok(mut guard) = self.conn.lock() {
                    *guard = Some(conn);
                }
                if let Ok(mut b) = self.breaker.lock() {
                    b.record_success();
                }
                true
            }
            Err(_) => {
                if let Ok(mut b) = self.breaker.lock() {
                    b.record_failure(now);
                }
                false
            }
        }
    }

    /// 发一帧，不等响应。
    ///
    /// ⚠ 调用方必须自己保证帧上带 `ASYNC_FLAG`（用 `IpcHeader::new_async`）。
    /// 本函数不代劳：有的「异步」帧其实要等 ack（如 batch），加不加是帧的语义。
    pub fn send_async(&self, frame: &[u8]) -> Result<(), TransportError> {
        if !self.ensure_connected() {
            return Err(TransportError::NotConnected);
        }
        let result = {
            let Ok(guard) = self.conn.lock() else {
                return Err(TransportError::NotConnected);
            };
            match guard.as_ref() {
                Some(c) => c.write_frame(frame),
                None => Err(TransportError::NotConnected),
            }
        };
        self.note_result(result.is_ok());
        result
    }

    /// 发一帧并等响应。
    ///
    /// ⚠ **失败时不可盲目重发**：`ReadTimeout` 意味着对端可能已经处理、
    /// 只是回得慢。协调器不按 `event_seq` 去重，重发就是重复上屏。
    /// 只有 `WriteFailed` / `PeerGone`（对端确定没处理）才安全。
    pub fn send_sync(&self, frame: &[u8]) -> SyncOutcome {
        if !self.ensure_connected() {
            return SyncOutcome::ServiceUnavailable;
        }

        let write_result = {
            let Ok(guard) = self.conn.lock() else {
                return SyncOutcome::ServiceUnavailable;
            };
            match guard.as_ref() {
                Some(c) => c.write_frame(frame),
                None => return SyncOutcome::ServiceUnavailable,
            }
        };
        if let Err(e) = write_result {
            self.note_result(false);
            return SyncOutcome::Transport(e);
        }
        self.stats.frames_sent.fetch_add(1, Ordering::SeqCst);

        let read_result = {
            let Ok(guard) = self.conn.lock() else {
                return SyncOutcome::ServiceUnavailable;
            };
            match guard.as_ref() {
                Some(c) => c.read_frame(),
                None => return SyncOutcome::ServiceUnavailable,
            }
        };

        match read_result {
            Ok(frame) => {
                self.stats.frames_received.fetch_add(1, Ordering::SeqCst);
                if let Ok(mut b) = self.breaker.lock() {
                    b.record_success();
                }
                SyncOutcome::Response(frame)
            }
            Err(e) => {
                self.note_result(false);
                SyncOutcome::Transport(e)
            }
        }
    }

    /// 记一次结果到熔断器。只看成败，不需要错误值本身。
    fn note_result(&self, ok: bool) {
        let now = now_ms();
        if let Ok(mut b) = self.breaker.lock() {
            if ok {
                b.record_success();
            } else {
                b.record_failure(now);
            }
        }
    }

    /// 发按键事件（同步，等响应）。
    ///
    /// 这是最高频的路径：每个键一次往返。
    pub fn send_key_event(&self, payload: &KeyPayload) -> SyncOutcome {
        let frame = aurora_ipc::upstream::encode_key_event(payload);
        self.stats.frames_sent.fetch_add(1, Ordering::SeqCst);
        self.send_sync(&frame)
    }

    /// 发 IME_ACTIVATED（异步，不等响应）。
    ///
    /// ⚠ 必须异步：服务端**从不回**这个帧。同步发会让主线程白等一个超时。
    pub fn send_ime_activated(&self) -> Result<(), TransportError> {
        let payload = aurora_ipc::upstream::encode_ime_activated();
        // 重新组装成异步帧
        let frame = with_async_flag(&payload);
        self.send_async(&frame)
    }

    /// 发 IME_DEACTIVATED（异步）。
    pub fn send_ime_deactivated(&self) -> Result<(), TransportError> {
        let payload = aurora_ipc::upstream::encode_ime_deactivated();
        let frame = with_async_flag(&payload);
        self.send_async(&frame)
    }

    /// 断开连接（服务重启 / 主动重连时调）。
    pub fn disconnect(&self) {
        if let Ok(mut guard) = self.conn.lock() {
            *guard = None;
        }
        self.ready.store(false, Ordering::SeqCst);
    }

    /// 标记已收到 SERVICE_READY。会重置熔断器。
    pub fn on_service_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
        if let Ok(mut b) = self.breaker.lock() {
            b.reset();
        }
    }

    /// 熔断状态（诊断）。
    pub fn circuit_state(&self) -> super::circuit_breaker::CircuitState {
        self.breaker
            .lock()
            .map(|b| b.state())
            .unwrap_or(super::circuit_breaker::CircuitState::Closed)
    }
}

/// 把一帧的头部改成异步标志。
///
/// 复用同一份载荷，只动 `version` 字段的高位——这样上行帧的布局仍只有
/// `aurora-ipc` 一处定义。
fn with_async_flag(frame: &[u8]) -> Vec<u8> {
    let mut out = frame.to_vec();
    if out.len() >= 2 {
        let v = u16::from_le_bytes([out[0], out[1]]) | ASYNC_FLAG;
        out[0..2].copy_from_slice(&v.to_le_bytes());
    }
    out
}

fn now_ms() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_builds_correct_pipe_name() {
        let c = IpcConfig::new("_dev", "_S-1-5-21-1");
        assert_eq!(c.pipe_name(), r"\\.\pipe\aurora_input_dev_S-1-5-21-1");
    }

    /// 异步标志必须真的写进头部——忘了会让客户端等一个永不到来的响应。
    #[test]
    fn async_flag_is_set_in_header() {
        let payload = aurora_ipc::upstream::encode_ime_activated();
        // 原始帧：version = PROTOCOL_VERSION
        assert_eq!(
            u16::from_le_bytes([payload[0], payload[1]]),
            PROTOCOL_VERSION
        );
        let async_frame = with_async_flag(&payload);
        let v = u16::from_le_bytes([async_frame[0], async_frame[1]]);
        assert_ne!(v & ASYNC_FLAG, 0, "ASYNC_FLAG 必须置位");
        // 其余部分不变
        assert_eq!(&payload[2..], &async_frame[2..], "只动 version 字段");
    }

    /// 帧太短时不该 panic（防御性）。
    #[test]
    fn async_flag_on_short_frame_is_safe() {
        let short = [0u8; 1];
        let out = with_async_flag(&short);
        assert_eq!(out.len(), 1);
    }

    /// 未连接时同步请求返回 ServiceUnavailable 而不是挂住。
    #[test]
    fn sync_without_connection_does_not_block() {
        // 用一个不存在的管道名，确保连不上
        let c = IpcClient::new(IpcConfig::new("_nonexistent_test_", "_S-9-9-9"));
        let p = KeyPayload {
            key_code: 0x41,
            scan_code: 0,
            modifiers: 0,
            event_type: 0,
            toggles: 0,
            event_seq: 0,
            prev_char: 0,
        };
        let frame = aurora_ipc::upstream::encode_key_event(&p);
        match c.send_sync(&frame) {
            SyncOutcome::ServiceUnavailable | SyncOutcome::Transport(_) => {}
            other => panic!("期望不可用，实得 {other:?}"),
        }
    }

    /// 事件序号单调递增。
    #[test]
    fn event_seq_is_monotonic() {
        let c = IpcClient::new(IpcConfig::new("", ""));
        assert_eq!(c.next_event_seq(), 0);
        assert_eq!(c.next_event_seq(), 1);
        assert_eq!(c.next_event_seq(), 2);
    }

    /// 连续失败会让熔断打开。
    #[test]
    fn repeated_failures_open_the_circuit() {
        use super::super::circuit_breaker::{CircuitState, MAX_CONSECUTIVE_FAILURES};
        let c = IpcClient::new(IpcConfig::new("_no_such_pipe_x_", ""));
        let p = KeyPayload {
            key_code: 0x41,
            scan_code: 0,
            modifiers: 0,
            event_type: 0,
            toggles: 0,
            event_seq: 0,
            prev_char: 0,
        };
        let frame = aurora_ipc::upstream::encode_key_event(&p);
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            let _ = c.send_sync(&frame);
        }
        assert_eq!(
            c.circuit_state(),
            CircuitState::Open,
            "连续失败后应熔断，避免每个键都付一次连接超时"
        );
    }

    /// 收到 SERVICE_READY 会重置熔断。
    #[test]
    fn service_ready_resets_breaker() {
        use super::super::circuit_breaker::CircuitState;
        let c = IpcClient::new(IpcConfig::new("_no_such_pipe_y_", ""));
        let p = KeyPayload {
            key_code: 0x41,
            scan_code: 0,
            modifiers: 0,
            event_type: 0,
            toggles: 0,
            event_seq: 0,
            prev_char: 0,
        };
        let frame = aurora_ipc::upstream::encode_key_event(&p);
        for _ in 0..3 {
            let _ = c.send_sync(&frame);
        }
        c.on_service_ready();
        assert_eq!(c.circuit_state(), CircuitState::Closed);
        assert!(c.is_ready());
    }
}
