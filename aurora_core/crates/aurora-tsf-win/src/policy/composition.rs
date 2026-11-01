//! 组合（composition）生命周期状态机。
//!
//! 原实现：`aurora_tsf/src/TextService.cpp` 的 `CEndCompositionEditSession`（约 120 行）
//! 与散落各处的 `_pComposition` / `_isComposing` 守卫。
//!
//! # 组合是什么
//!
//! 用户打拼音时，还没上屏的编码显示在光标处、带下划线、可编辑——那一段就是组合。
//! TSF 要求我们显式 `StartComposition` / `EndComposition`，否则那段文字会被当成
//! 已提交内容进入文档撤销栈。
//!
//! # 本文件管什么
//!
//! **只有状态转移判据**，不碰 COM。它回答四个问题：
//!
//! 1. 现在该不该 `StartComposition`？
//! 2. 现在该不该 `EndComposition`？
//! 3. 结束前要不要先清空文字？
//! 4. 收到 `OnCompositionTerminated`（宿主强杀）后该怎么记账？
//!
//! # 三个容易错的点
//!
//! ## 一、结束前必须清空 range 文字
//!
//! 直接 `EndComposition` 会把组合文本**提交进文档**。原实现的 `DoEditSession`
//! 明确先 `SetText(ec, 0, L"", 0)` 再 `EndComposition`，注释写的是
//! 「This prevents the composition text from being committed」。
//!
//! 这就是「按 Esc 取消输入」能干净取消的原因——如果只是 `EndComposition`，
//! Esc 之后那串拼音字母会留在文档里。
//!
//! ## 二、`OnCompositionTerminated` 之后不能再 `EndComposition`
//!
//! 宿主强杀组合时（切窗口、应用主动结束、崩溃恢复）会回调它，**此后那个
//! `ITfComposition` 指针已失效**。再调 `EndComposition` 是 use-after-free。
//!
//! ## 三、状态必须能区分「我结束了」与「宿主结束了」
//!
//! 两者后续动作不同：前者要发 IPC 通知服务端清状态，后者只能被动同步。
//! 用一个 `Option<EndReason>` 记下来。

/// 组合结束的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// 我们自己结束（上屏 / 取消 / 切模式）。
    ///
    /// 这类要发 IPC 告诉服务端「组合没了」，让它清掉对应状态。
    Ourselves,
    /// 宿主强杀（`OnCompositionTerminated`）。
    ///
    /// ⚠ 此后**不可**再对该组合调 `EndComposition`——指针已失效。
    TerminatedByHost,
}

/// 组合的生命周期状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CompositionState {
    /// 没有组合。
    #[default]
    Idle,
    /// 有活跃组合。
    Active {
        /// 组合会话序号。用来丢弃「上一轮组合的迟到响应」。
        session: u64,
        /// 组合里当前有多少字符（用于判断是否需要清空）。
        text_len: u32,
    },
    /// 组合刚结束，等待清理完成。
    Ending {
        session: u64,
        reason: EndReason,
        /// 结束前还有没有文字（决定要先清空）。
        had_text: bool,
    },
}

impl CompositionState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    pub fn is_ending(&self) -> bool {
        matches!(self, Self::Ending { .. })
    }

    pub fn session(&self) -> Option<u64> {
        match self {
            Self::Active { session, .. } | Self::Ending { session, .. } => Some(*session),
            Self::Idle => None,
        }
    }

    /// 当前是否应该拒绝新的 `StartComposition`。
    ///
    /// ⚠ `Ending` 也算拒绝：组合的异步清理还没跑完时再起一个，
    /// 会出现两个 `ITfComposition` 指向同一 range 的情况。
    pub fn should_reject_start(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

/// 一次状态转移的决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 什么都不做。
    None,
    /// 新建组合。
    Start { session: u64 },
    /// 更新组合文字（不结束）。
    UpdateText { session: u64 },
    /// 结束组合。`clear_text` 为真时**先清空 range 文字**再结束。
    End { session: u64, clear_text: bool },
}

/// 组合状态机。
#[derive(Debug)]
pub struct CompositionTracker {
    state: CompositionState,
    next_session: u64,
}

impl Default for CompositionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CompositionTracker {
    pub fn new() -> Self {
        Self {
            state: CompositionState::Idle,
            next_session: 0,
        }
    }

    pub fn state(&self) -> &CompositionState {
        &self.state
    }

    /// 服务端给出了新的编码文本。
    ///
    /// - 空闲 ⇒ `Start`
    /// - 已有组合 ⇒ `UpdateText`
    /// - 正在结束 ⇒ `None`（等清理完成，别抢）
    pub fn on_text(&mut self, text_len: u32) -> Action {
        match &self.state {
            CompositionState::Idle => {
                if text_len == 0 {
                    // 空文本不建组合——那会在文档里留一个零宽标记
                    return Action::None;
                }
                let session = self.next_session;
                self.next_session += 1;
                self.state = CompositionState::Active { session, text_len };
                Action::Start { session }
            }
            CompositionState::Active { session, .. } => {
                let session = *session;
                if text_len == 0 {
                    // 编码被删空 ⇒ 结束组合（并清空，虽然本来就是空的）
                    self.state = CompositionState::Ending {
                        session,
                        reason: EndReason::Ourselves,
                        had_text: true,
                    };
                    Action::End {
                        session,
                        clear_text: true,
                    }
                } else {
                    self.state = CompositionState::Active { session, text_len };
                    Action::UpdateText { session }
                }
            }
            CompositionState::Ending { .. } => Action::None,
        }
    }

    /// 需要结束组合（上屏 / 取消 / 切模式）。
    ///
    /// `clear_text` 的判据：**取消**时要清空（不然拼音留在文档里），
    /// **上屏**时不用（服务端会紧接着插入正式文本，清不清都一样，
    /// 但清一下更保险，见下面的注释）。
    pub fn on_end_requested(&mut self, clear_text: bool) -> Action {
        match &self.state {
            CompositionState::Active { session, text_len } => {
                let session = *session;
                let had_text = *text_len > 0;
                self.state = CompositionState::Ending {
                    session,
                    reason: EndReason::Ourselves,
                    had_text,
                };
                Action::End {
                    session,
                    clear_text: clear_text && had_text,
                }
            }
            _ => Action::None,
        }
    }

    /// 宿主强杀了组合（`OnCompositionTerminated`）。
    ///
    /// ⚠ 返回 `Action::None`：**已经失效的指针不能再动**。
    /// 这里只把状态清干净，让下一次 `on_text` 能重新开始。
    pub fn on_host_terminated(&mut self) -> EndReason {
        let reason = EndReason::TerminatedByHost;
        self.state = CompositionState::Idle;
        reason
    }

    /// 异步清理完成，回到空闲。
    ///
    /// 只对 `Ending` 有效：`Active` 调它是逻辑错误（组合还活着）。
    pub fn on_cleanup_done(&mut self) -> bool {
        if self.state.is_ending() {
            self.state = CompositionState::Idle;
            true
        } else {
            false
        }
    }

    /// 焦点丢失 / 切文档：强制丢弃组合状态。
    ///
    /// ⚠ **不产生 Action**：此时 context 可能已经不可用，任何 COM 调用都会失败。
    /// 状态照清，让下次能干净开始。
    pub fn on_focus_lost(&mut self) {
        self.state = CompositionState::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_idle() {
        let t = CompositionTracker::new();
        assert_eq!(*t.state(), CompositionState::Idle);
        assert!(!t.state().is_active());
        assert!(!t.state().should_reject_start());
    }

    /// 空文本不建组合——会在文档里留零宽标记。
    #[test]
    fn empty_text_does_not_start() {
        let mut t = CompositionTracker::new();
        assert_eq!(t.on_text(0), Action::None);
        assert_eq!(*t.state(), CompositionState::Idle);
    }

    #[test]
    fn first_text_starts_composition() {
        let mut t = CompositionTracker::new();
        let a = t.on_text(3);
        assert_eq!(a, Action::Start { session: 0 });
        assert!(t.state().is_active());
        assert_eq!(t.state().session(), Some(0));
    }

    #[test]
    fn subsequent_text_updates() {
        let mut t = CompositionTracker::new();
        t.on_text(2);
        assert_eq!(t.on_text(5), Action::UpdateText { session: 0 });
        assert_eq!(t.state().session(), Some(0), "会话号不该变");
    }

    /// 编码被删空 ⇒ 结束组合。
    #[test]
    fn deleting_all_text_ends_composition() {
        let mut t = CompositionTracker::new();
        t.on_text(2);
        assert_eq!(
            t.on_text(0),
            Action::End {
                session: 0,
                clear_text: true
            }
        );
        assert!(t.state().is_ending());
    }

    /// 每个新组合拿到新会话号——用来丢弃上一轮的迟到响应。
    #[test]
    fn sessions_increment() {
        let mut t = CompositionTracker::new();
        t.on_text(1);
        t.on_end_requested(true);
        t.on_cleanup_done();
        match t.on_text(1) {
            Action::Start { session } => assert_eq!(session, 1),
            other => panic!("期望 Start，实得 {other:?}"),
        }
    }

    /// 结束请求：有文字时要清空。
    #[test]
    fn end_with_text_clears() {
        let mut t = CompositionTracker::new();
        t.on_text(3);
        assert_eq!(
            t.on_end_requested(true),
            Action::End {
                session: 0,
                clear_text: true
            }
        );
    }

    /// 没有文字时不用清空。
    #[test]
    fn end_without_text_does_not_clear() {
        let mut t = CompositionTracker::new();
        t.on_text(3);
        // 模拟编码已空但还在 Active（理论上前一步会转 Ending，
        // 这里直接构造验证 clear_text 的 had_text 判据）
        t.state = CompositionState::Active {
            session: 0,
            text_len: 0,
        };
        assert_eq!(
            t.on_end_requested(true),
            Action::End {
                session: 0,
                clear_text: false
            }
        );
    }

    /// 空闲时请求结束 ⇒ 什么都不做。
    #[test]
    fn end_when_idle_is_noop() {
        let mut t = CompositionTracker::new();
        assert_eq!(t.on_end_requested(true), Action::None);
    }

    /// **核心安全判据**：宿主强杀后不能再产生任何 Action。
    #[test]
    fn host_termination_blocks_further_actions() {
        let mut t = CompositionTracker::new();
        t.on_text(3);
        assert_eq!(t.on_host_terminated(), EndReason::TerminatedByHost);
        assert_eq!(*t.state(), CompositionState::Idle);
        // 此刻那个 ITfComposition 已失效，任何 End 都是 use-after-free
        assert_eq!(t.on_end_requested(true), Action::None);
    }

    /// Ending 期间拒绝新组合——否则会有两个组合指向同一 range。
    #[test]
    fn ending_rejects_start() {
        let mut t = CompositionTracker::new();
        t.on_text(3);
        t.on_end_requested(true);
        assert!(t.state().should_reject_start());
        assert_eq!(t.on_text(5), Action::None, "清理未完成时不该起新组合");
    }

    #[test]
    fn cleanup_returns_to_idle() {
        let mut t = CompositionTracker::new();
        t.on_text(3);
        t.on_end_requested(true);
        assert!(t.on_cleanup_done());
        assert_eq!(*t.state(), CompositionState::Idle);
        // 再次清理返回 false（幂等）
        assert!(!t.on_cleanup_done());
    }

    /// 焦点丢失：状态清干净，不产生 Action（context 可能已不可用）。
    #[test]
    fn focus_lost_resets_state() {
        let mut t = CompositionTracker::new();
        t.on_text(3);
        t.on_focus_lost();
        assert_eq!(*t.state(), CompositionState::Idle);
        // 之后还能正常重新开始
        assert_eq!(t.on_text(1), Action::Start { session: 1 });
    }

    /// 完整的输入周期：起 → 更新 → 上屏 → 清理。
    #[test]
    fn full_cycle() {
        let mut t = CompositionTracker::new();
        assert_eq!(t.on_text(2), Action::Start { session: 0 });
        assert_eq!(t.on_text(4), Action::UpdateText { session: 0 });
        assert_eq!(
            t.on_end_requested(true),
            Action::End {
                session: 0,
                clear_text: true
            }
        );
        assert!(t.on_cleanup_done());
        assert_eq!(*t.state(), CompositionState::Idle);
        // 下一轮
        assert_eq!(t.on_text(1), Action::Start { session: 1 });
    }

    /// 结束时记下 had_text，供 COM 层决定要不要先 SetText。
    #[test]
    fn ending_remembers_whether_text_existed() {
        let mut t = CompositionTracker::new();
        t.on_text(5);
        t.on_end_requested(true);
        match t.state() {
            CompositionState::Ending {
                had_text, reason, ..
            } => {
                assert!(*had_text);
                assert_eq!(*reason, EndReason::Ourselves);
            }
            other => panic!("期望 Ending，实得 {other:?}"),
        }
    }
}
