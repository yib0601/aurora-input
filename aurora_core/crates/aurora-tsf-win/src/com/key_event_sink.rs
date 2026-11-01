//! `ITfKeyEventSink`：按键入口。
//!
//! 原实现：`aurora_tsf/src/KeyEventSink.cpp`（3683 行）+ `include/KeyEventSink.h`（731 行）。
//!
//! # 本文件的范围
//!
//! **只做按键统计与吃键决策的骨架**，不含业务逻辑：
//!
//! - 修饰键状态跟踪（[`ModifierState`]）
//! - 事件序号分配
//! - 「吃 / 不吃」的返回
//!
//! 尚未移植（原实现里的这些是行为主体）：
//!
//! | 功能 | 原文位置 | 状态 |
//! |---|---|---|
//! | 热键白名单派发 | `DispatchHotkey` | 未做 |
//! | 屏障机制（CommitRequest / 等待响应） | `_pendingCommit` + 定时器 | 未做 |
//! | skip 表消费 | `_TryConsumeSkipKey` | 表已移植，接线未做 |
//! | 智能配对 / 英文半角吃键 | `_IsEnglishHalfEatenPunctKey` | 判据已移植，接线未做 |
//! | key.hold 重放 | `ShouldReplayAfterHold` | 判据已移植，接线未做 |
//! | 数字后智能标点记账 | `PassthroughState` | 已移植，接线未做 |
//!
//! # ⚠ 当前行为：一律不吃
//!
//! 因为 IPC 层还没接，此刻「吃下」键就意味着**键被吃掉但没有任何字出来**。
//! 原实现反复吃过这个亏（仓库 `AGENTS.md` 的「C++ 吃键集必须 ⊆ Rust 出字集」）。
//! 所以在能真正把键转发给服务端之前，本 sink 一律返回 `false`（不吃）——
//! 这样即使被系统激活，也只是「这个输入法没反应」，不会丢键。
//!
//! # OnTestKeyDown 与 OnKeyDown 必须同源
//!
//! TSF 先调 `OnTestKeyDown` 问「这个键你要不要」，宿主据此决定是否让键进入
//! 我们的 `OnKeyDown`。但 **Chrome / QQ 等宿主无视 `pfEaten=FALSE` 仍会调
//! `OnKeyDown`**（见原实现里点名的那两处注释）。因此两处必须用**同一个判据**，
//! 否则会出现「Test 说不要、KeyDown 却处理了」→ 宿主出一次、我们插一次，
//! PIN 框里一键两个字。

#![cfg(windows)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use windows::Win32::Foundation::BOOL;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::TextServices::{
    ITfContext, ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeyTraceEventSink,
    ITfKeyTraceEventSink_Impl,
};
use windows::core::{GUID, Result};

use crate::ipc::client::{IpcClient, SyncOutcome};
use crate::passthrough_note::PassthroughState;
use crate::policy::key_state::ModifierState;
use crate::skip_key_table::SkipKeyTable;
use aurora_ipc::protocol::{KeyPayload, TOGGLE_PASSTHROUGH_KEY};

/// 当前时间（毫秒），用 `GetTickCount64`。
///
/// 参数化时钟是纯逻辑层的做法；COM 层直接用系统时钟。
fn now_ms() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

/// 按键状态：修饰键、事件序号、诊断计数。
#[derive(Debug)]
pub struct KeySinkState {
    pub modifiers: Mutex<ModifierState>,
    /// 注入键登记表（我们自己 SendInput 造的键要放行）。
    pub skip_table: Mutex<SkipKeyTable>,
    /// 「有键进了宿主」的记账（服务端智能符号 press2 需要）。
    pub passthrough: Mutex<PassthroughState>,
    /// IPC 客户端。`None` 表示未装配（测试或降级）。
    pub ipc: Option<std::sync::Arc<IpcClient>>,
    /// 本 sink 是否收到过焦点（`OnSetFocus`）。
    has_focus: AtomicBool,
    /// 累计收到的 keydown 数（诊断）。
    keydown_count: AtomicU64,
    /// 累计吃下的键数（诊断）。当前恒为 0（见模块文档）。
    eaten_count: AtomicU64,
}

impl Default for KeySinkState {
    fn default() -> Self {
        Self::new()
    }
}

impl KeySinkState {
    pub fn new() -> Self {
        Self {
            modifiers: Mutex::new(ModifierState::new()),
            skip_table: Mutex::new(SkipKeyTable::new()),
            passthrough: Mutex::new(PassthroughState::new()),
            ipc: None,
            has_focus: AtomicBool::new(false),
            keydown_count: AtomicU64::new(0),
            eaten_count: AtomicU64::new(0),
        }
    }

    pub fn has_focus(&self) -> bool {
        self.has_focus.load(Ordering::SeqCst)
    }

    pub fn keydown_count(&self) -> u64 {
        self.keydown_count.load(Ordering::SeqCst)
    }

    pub fn eaten_count(&self) -> u64 {
        self.eaten_count.load(Ordering::SeqCst)
    }

    /// 记录一次 keydown。
    pub fn note_keydown(&self) {
        self.keydown_count.fetch_add(1, Ordering::SeqCst);
    }
}

/// `ITfKeyEventSink` 实现。
#[windows::core::implement(ITfKeyEventSink, ITfKeyTraceEventSink)]
pub struct KeyEventSink {
    pub state: std::sync::Arc<KeySinkState>,
}

impl KeyEventSink {
    pub fn new(state: std::sync::Arc<KeySinkState>) -> Self {
        Self { state }
    }

    /// 吃键判据的**唯一入口**。
    ///
    /// `OnTestKeyDown` 与 `OnKeyDown` 都调它——这是原实现吃过亏的地方，
    /// 见模块文档「必须同源」。
    ///
    /// # 返回值的语义
    ///
    /// - `true`：键交给我们（转发服务端并等响应）
    /// - `false`：键交给宿主
    ///
    /// # ⚠ 这里**不能**做 IPC 往返
    ///
    /// `OnTestKeyDown` 是宿主的**前置闸门**——它必须立刻回答，否则宿主 UI 线程卡住。
    /// 原实现（以及本实现）的策略是：Test 阶段只做**本地**判定，
    /// 真正转发在 `OnKeyDown` 里。
    ///
    /// 本地判定能吃什么？只有「无论如何都要送到服务端」的键：
    /// 修饰键组合、输入法自己的热键。普通可打印字符**不能**在 Test 阶段吃
    /// ——那会把「服务端不需要的键」也吃掉，宿主再也拿不到（PIN 框一键两个字）。
    ///
    /// ⚠ 当前实现：只吃「非修饰键且已连接服务」的键。
    /// 这与原实现的 `_IsInputMethodOwnedKey` 相比保守得多——保守的方向是
    /// 「宁可漏吃，不可错吃」，因为漏吃只是宿主自己处理（用户看得见结果），
    /// 错吃则是丢键（用户看不到任何反馈）。
    fn should_eat(&self, vk: u32, _modifiers: u32) -> bool {
        // 修饰键永远不吃：宿主需要看见它们（组合键、系统热键）
        if crate::policy::key_state::modifier_bit(vk).is_some() {
            return false;
        }
        // 服务不可用时不吃：吃了也没人能出字
        let Some(ipc) = self.state.ipc.as_ref() else {
            return false;
        };
        ipc.ensure_connected()
    }

    /// 把 keydown 转发给服务端，并据响应决定是否吃掉。
    ///
    /// 这是真正的输入路径。返回值即 `pfEaten`。
    fn forward_key(&self, vk: u32, event_seq: u16, prev_char: u16, toggles: u8) -> bool {
        let Some(ipc) = self.state.ipc.as_ref() else {
            return false;
        };

        let modifiers = self
            .state
            .modifiers
            .lock()
            .map(|m| m.current())
            .unwrap_or(0);

        // 透传记账：把这个事实告诉服务端（智能符号 press2 需要）
        let passthrough_bit = self
            .state
            .passthrough
            .lock()
            .map(|mut p| p.take_on_key_down_send())
            .unwrap_or(false);
        let toggles = if passthrough_bit {
            toggles | TOGGLE_PASSTHROUGH_KEY
        } else {
            toggles
        };

        let payload = KeyPayload {
            key_code: vk,
            scan_code: 0,
            modifiers,
            event_type: 0, // keydown
            toggles,
            event_seq,
            prev_char,
        };

        match ipc.send_key_event(&payload) {
            SyncOutcome::Response(frame) => {
                // 用 aurora-ipc 解码响应类型
                let cmd = aurora_ipc::codec::decode_header(&frame)
                    .map(|h| h.command)
                    .unwrap_or(0);
                use crate::policy::response_type::ResponseType;
                match ResponseType::from_command(cmd) {
                    // 服务端消费了键（不管出不出字）⇒ 我们吃掉
                    ResponseType::Consumed
                    | ResponseType::CommitText
                    | ResponseType::InsertTextWithCursor
                    | ResponseType::CommitThenDefer
                    | ResponseType::ReplaceBackward
                    | ResponseType::UpdateComposition
                    | ResponseType::ClearComposition => true,
                    // PassThrough：服务端不要这个键，还给宿主
                    ResponseType::PassThrough | ResponseType::Other => false,
                }
            }
            SyncOutcome::ServiceUnavailable | SyncOutcome::Transport(_) => {
                // 转发失败 ⇒ 不吃。吃了就丢键——这是原实现反复吃的亏
                false
            }
        }
    }
}

/// 从 `WPARAM` 取虚拟键码。
#[inline]
fn vk_from_wparam(wparam: WPARAM) -> u32 {
    (wparam.0 & 0xFFFF) as u32
}

impl ITfKeyEventSink_Impl for KeyEventSink_Impl {
    /// 焦点进入/离开本 sink。
    ///
    /// 失焦时必须复位修饰键状态——否则「失焦时按着 Shift，回来以为还按着」。
    fn OnSetFocus(&self, fforeground: BOOL) -> Result<()> {
        self.state
            .has_focus
            .store(fforeground.as_bool(), Ordering::SeqCst);
        if let Ok(mut m) = self.state.modifiers.lock() {
            m.reset();
        }
        Ok(())
    }

    /// 宿主问「这个键你要不要」。
    ///
    /// ⚠ 这里**只判不吃**，不做任何副作用：宿主的 Test 可能被调多次
    /// （不同阶段各问一遍），在里面改状态会重复计数。
    fn OnTestKeyDown(
        &self,
        _pic: Option<&ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let vk = vk_from_wparam(wparam);
        let mods = self
            .state
            .modifiers
            .lock()
            .map(|m| m.current())
            .unwrap_or(0);
        Ok(BOOL::from(self.should_eat(vk, mods)))
    }

    /// 同 `OnTestKeyDown`（必须同源）。
    fn OnTestKeyUp(
        &self,
        _pic: Option<&ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        // 当前没有「只吃 keyup」的判据（原实现里的修饰键轻敲走的是另一条路）。
        Ok(BOOL::from(false))
    }

    /// 按键真正到达。
    ///
    /// 与 `OnTestKeyDown` 用**同一个** `should_eat`。
    fn OnKeyDown(
        &self,
        _pic: Option<&ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let vk = vk_from_wparam(wparam);
        self.state.note_keydown();

        // 更新修饰键状态（即使不吃也要更新：修饰键状态是全局的）
        let mods = {
            let Ok(mut m) = self.state.modifiers.lock() else {
                return Ok(BOOL::from(false));
            };
            m.next_event_seq();
            if m.on_modifier_down(vk, now_ms()).is_none() {
                // 非修饰键：作废任何待判定的轻敲
                m.on_non_modifier_key();
            }
            m.current()
        };

        // 先看 skip 表：命中说明这是我们自己注入的键，直接放行
        let suppressed = self
            .state
            .skip_table
            .lock()
            .map(|mut t| t.try_consume(vk as u16, false, now_ms()))
            .unwrap_or(false);
        if suppressed {
            return Ok(BOOL::from(false));
        }

        let eat = self.should_eat(vk, mods);
        if !eat {
            // 不吃：记「这个键进了宿主」，服务端智能符号要用
            if let Ok(mut p) = self.state.passthrough.lock() {
                p.note_key_down(vk, false, false);
            }
            return Ok(BOOL::from(false));
        }

        let seq = self
            .state
            .modifiers
            .lock()
            .map(|mut m| m.next_event_seq())
            .unwrap_or(0);
        let eaten = self.forward_key(vk, seq, 0, 0);
        if eaten {
            self.state.eaten_count.fetch_add(1, Ordering::SeqCst);
        }
        Ok(BOOL::from(eaten))
    }

    /// 按键松开。
    ///
    /// 修饰键松开可能构成「轻敲」——那是中英切换的触发条件之一。
    /// 当前只更新状态，不触发切换（切换逻辑未移植）。
    fn OnKeyUp(&self, _pic: Option<&ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        let vk = vk_from_wparam(wparam);
        if let Ok(mut m) = self.state.modifiers.lock() {
            let _is_tap = m.on_modifier_up(vk, now_ms());
            // 轻敲 → 中英切换：待移植（原实现走 ToggleModeFromKey）
        }
        Ok(BOOL::from(false))
    }

    /// 保留键（`RegisterHotKey` 注册过的组合键）。
    fn OnPreservedKey(&self, _pic: Option<&ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        // 当前不注册任何保留键。
        Ok(BOOL::from(false))
    }
}

impl ITfKeyTraceEventSink_Impl for KeyEventSink_Impl {
    /// 键在**到达任何输入法之前**被观察到。
    ///
    /// 用途：即使键最终被宿主吃掉或透传，我们也知道用户按了什么。
    /// 原实现用它做「输入序列记账」（数字后智能标点的 `prevChar` 来源）。
    /// 当前只更新修饰键状态。
    fn OnKeyTraceDown(&self, wparam: WPARAM, _lparam: LPARAM) -> Result<()> {
        let vk = vk_from_wparam(wparam);
        if let Ok(mut m) = self.state.modifiers.lock() {
            let _ = m.on_modifier_down(vk, now_ms());
        }
        Ok(())
    }

    fn OnKeyTraceUp(&self, wparam: WPARAM, _lparam: LPARAM) -> Result<()> {
        let vk = vk_from_wparam(wparam);
        if let Ok(mut m) = self.state.modifiers.lock() {
            let _ = m.on_modifier_up(vk, now_ms());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vk_extraction_from_wparam() {
        // 低 16 位是 VK，高 16 位是重复计数（这里应为 0）
        assert_eq!(vk_from_wparam(WPARAM(0x0041)), 0x41);
        assert_eq!(vk_from_wparam(WPARAM(0x0000_0041)), 0x41);
    }

    #[test]
    fn state_starts_clean() {
        let s = KeySinkState::new();
        assert!(!s.has_focus());
        assert_eq!(s.keydown_count(), 0);
        assert_eq!(s.eaten_count(), 0);
    }

    /// 当前策略：一律不吃。IPC 未接时吃键 = 丢键。
    #[test]
    fn nothing_is_eaten_yet() {
        let s = std::sync::Arc::new(KeySinkState::new());
        let sink = KeyEventSink::new(s);
        for vk in [0x41u32, 0x20, 0x0D, 0x1B, 0x25] {
            assert!(!sink.should_eat(vk, 0), "vk=0x{vk:02X} 当前不该被吃");
        }
    }
}
