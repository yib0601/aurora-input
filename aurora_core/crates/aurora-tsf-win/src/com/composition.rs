//! 组合管理：`ITfContextComposition` / `ITfComposition` / `ITfCompositionSink` 的封装。
//!
//! 原实现：`aurora_tsf/src/TextService.cpp` 的 `CEndCompositionEditSession`
//! 与 `_StartComposition` / `_EndComposition`（约 400 行）。
//!
//! # 为什么必须走 edit session
//!
//! `StartComposition` / `EndComposition` / `SetText` 都要求**写锁**（`ec` 非零）。
//! 我们不在 TSF 回调里时没有锁，必须：
//!
//! ```text
//! ITfContext::RequestEditSession(TF_ES_ASYNC, session)
//!   → 宿主回调 ITfEditSession::DoEditSession(ec)
//!     → 此刻拿到 ec，可以做写操作
//! ```
//!
//! # 同步 vs 异步
//!
//! | 场景 | 标志 |
//! |---|---|
//! | 按键回调内（本来就持有锁） | `TF_ES_SYNC` |
//! | 其他（IPC 线程、定时器） | `TF_ES_ASYNC` |
//!
//! ⚠ 在 `OnSetFocus` 期间用 `TF_ES_SYNC` 会被宿主拒（`TS_E_SYNCHRONOUS`），
//! 这是原实现踩过的坑。
//!
//! # 结束组合前必须清空文字
//!
//! 判据与理由见 [`crate::policy::composition`] 的模块文档。

#![cfg(windows)]

use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::E_POINTER;
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextComposition,
    ITfEditSession, ITfEditSession_Impl, ITfRange, TF_ES_ASYNC, TF_ES_SYNC,
};
use windows::core::{Interface, Result, implement};

use crate::policy::composition::{Action, CompositionState, CompositionTracker, EndReason};

/// 一次 edit session 要做的事。
#[derive(Debug, Clone, Copy)]
pub enum EditTask {
    /// 开始组合，并用给定文字填进去。
    Start,
    /// 用新文字替换组合内容。
    UpdateText,
    /// 结束组合（`clear_text` 决定是否先清空）。
    End { clear_text: bool },
}

/// 组合会话的运行时状态。
///
/// 与 [`CompositionTracker`]（纯逻辑）的区别：这里持有 COM 指针。
#[derive(Default)]
pub struct CompositionRuntime {
    tracker: Mutex<CompositionTracker>,
    /// 活跃的 `ITfComposition`。
    ///
    /// ⚠ 用 `Option<ITfComposition>` 而不是裸指针：COM 指针带引用计数，
    /// 需要准确释放。宿主强杀后要立刻清空。
    composition: Mutex<Option<ITfComposition>>,
    /// 组合 range（建组合时构造的那个，结束时不需要单独 Release）。
    range: Mutex<Option<ITfRange>>,
}

impl CompositionRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> CompositionState {
        self.tracker
            .lock()
            .map(|t| t.state().clone())
            .unwrap_or(CompositionState::Idle)
    }

    /// 当前是否持有活跃的 COM 组合指针。
    pub fn has_composition(&self) -> bool {
        self.composition
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false)
    }

    /// 计划：服务端给了新文本。
    pub fn plan_text(&self, text_len: u32) -> Action {
        self.tracker
            .lock()
            .map(|mut t| t.on_text(text_len))
            .unwrap_or(Action::None)
    }

    /// 计划：结束组合。
    pub fn plan_end(&self, clear_text: bool) -> Action {
        self.tracker
            .lock()
            .map(|mut t| t.on_end_requested(clear_text))
            .unwrap_or(Action::None)
    }

    /// 宿主强杀了组合。
    ///
    /// ⚠ 必须**立刻**丢掉 COM 指针：此后它已失效，任何调用都是 use-after-free。
    pub fn on_host_terminated(&self) -> EndReason {
        if let Ok(mut g) = self.composition.lock() {
            *g = None;
        }
        if let Ok(mut g) = self.range.lock() {
            *g = None;
        }
        self.tracker
            .lock()
            .map(|mut t| t.on_host_terminated())
            .unwrap_or(EndReason::TerminatedByHost)
    }

    /// 清理完成。
    pub fn on_cleanup_done(&self) {
        if let Ok(mut g) = self.composition.lock() {
            *g = None;
        }
        if let Ok(mut g) = self.range.lock() {
            *g = None;
        }
        if let Ok(mut t) = self.tracker.lock() {
            t.on_cleanup_done();
        }
    }

    /// 焦点丢失：丢状态。
    pub fn on_focus_lost(&self) {
        if let Ok(mut g) = self.composition.lock() {
            *g = None;
        }
        if let Ok(mut g) = self.range.lock() {
            *g = None;
        }
        if let Ok(mut t) = self.tracker.lock() {
            t.on_focus_lost();
        }
    }

    fn store(&self, comp: ITfComposition, range: ITfRange) {
        if let Ok(mut g) = self.composition.lock() {
            *g = Some(comp);
        }
        if let Ok(mut g) = self.range.lock() {
            *g = Some(range);
        }
    }

    fn composition_ref(&self) -> Option<ITfComposition> {
        self.composition.lock().ok().and_then(|g| g.clone())
    }

    fn take_composition(&self) -> Option<ITfComposition> {
        self.composition.lock().ok().and_then(|mut g| g.take())
    }
}

/// `ITfCompositionSink`：宿主通知组合被终止。
#[implement(ITfCompositionSink)]
pub struct CompositionSink {
    pub runtime: Arc<CompositionRuntime>,
}

impl CompositionSink {
    pub fn new(runtime: Arc<CompositionRuntime>) -> Self {
        Self { runtime }
    }
}

impl ITfCompositionSink_Impl for CompositionSink_Impl {
    /// 组合被宿主终止（切窗口、应用主动结束、崩溃恢复）。
    ///
    /// ⚠ **绝对不能**在这里调 `EndComposition`：`pcomposition` 已经是失效指针。
    fn OnCompositionTerminated(
        &self,
        _ecwrite: u32,
        _pcomposition: Option<&ITfComposition>,
    ) -> Result<()> {
        let reason = self.runtime.on_host_terminated();
        tracing::debug!("组合被宿主终止: {reason:?}");
        Ok(())
    }
}

/// 执行一次组合操作的 edit session。
#[implement(ITfEditSession)]
pub struct CompositionEditSession {
    pub runtime: Arc<CompositionRuntime>,
    pub task: EditTask,
    /// 要写入组合的文字（`End` 时忽略）。
    pub text: Vec<u16>,
    /// 组合 range 的起点（acp 偏移）。
    pub acp_start: i32,
    /// 创建时的 context。`DoEditSession` 里要用它拿 `ITfContextComposition`。
    pub context: Mutex<Option<ITfContext>>,
}

impl CompositionEditSession {
    pub fn new(
        runtime: Arc<CompositionRuntime>,
        task: EditTask,
        text: String,
        acp_start: i32,
        context: ITfContext,
    ) -> Self {
        Self {
            runtime,
            task,
            text: text.encode_utf16().collect(),
            acp_start,
            context: Mutex::new(Some(context)),
        }
    }
}

impl ITfEditSession_Impl for CompositionEditSession_Impl {
    /// 宿主回调：此刻持有写锁 `ec`。
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        let Some(ctx) = self.context.lock().ok().and_then(|g| g.clone()) else {
            return Err(E_POINTER.into());
        };

        match self.task {
            EditTask::Start => {
                let Ok(comp_iface) = ctx.cast::<ITfContextComposition>() else {
                    tracing::warn!("context 不支持 ITfContextComposition");
                    return Ok(());
                };
                // SAFETY: DoEditSession 内持有写锁。
                unsafe { self.do_start(&ctx, &comp_iface, ec) };
            }
            EditTask::UpdateText => {
                // SAFETY: 同上。
                unsafe { self.do_update(ec) };
            }
            EditTask::End { clear_text } => {
                // SAFETY: 同上。
                unsafe { self.do_end(ec, clear_text) };
            }
        }
        Ok(())
    }
}

impl CompositionEditSession_Impl {
    /// 开始组合。
    unsafe fn do_start(&self, ctx: &ITfContext, comp_iface: &ITfContextComposition, ec: u32) {
        #![allow(unsafe_op_in_unsafe_fn)]
        let Ok(range) = self.make_range(ctx) else {
            tracing::warn!("组合 range 构造失败");
            return;
        };

        let sink: ITfCompositionSink = CompositionSink::new(self.runtime.clone()).into();
        match comp_iface.StartComposition(ec, &range, &sink) {
            Ok(comp) => {
                if !self.text.is_empty() {
                    if let Ok(comp_range) = comp.GetRange() {
                        let _ = comp_range.SetText(ec, 0, &self.text);
                    }
                }
                self.runtime.store(comp, range);
                tracing::debug!("组合已开始，{} 个字", self.text.len());
            }
            Err(e) => tracing::warn!("StartComposition 失败: {e}"),
        }
    }

    /// 更新组合文字。
    unsafe fn do_update(&self, ec: u32) {
        #![allow(unsafe_op_in_unsafe_fn)]
        let Some(comp) = self.runtime.composition_ref() else {
            return;
        };
        let Ok(range) = comp.GetRange() else {
            return;
        };
        if let Err(e) = range.SetText(ec, 0, &self.text) {
            tracing::warn!("组合 SetText 失败: {e}");
        }
    }

    /// 结束组合。
    ///
    /// ⚠ 清空文字必须在 `EndComposition` **之前**：直接结束会把组合文本
    /// 提交进文档——按 Esc 后拼音字母留在文档里就是这么来的。
    unsafe fn do_end(&self, ec: u32, clear_text: bool) {
        #![allow(unsafe_op_in_unsafe_fn)]
        let Some(comp) = self.runtime.take_composition() else {
            return;
        };

        if clear_text {
            if let Ok(range) = comp.GetRange() {
                let _ = range.SetText(ec, 0, &[]);
            }
        }

        let _ = comp.EndComposition(ec);
        // drop 时释放我们持有的那份引用
        drop(comp);
        self.runtime.on_cleanup_done();
        tracing::debug!("组合已结束（clear_text={clear_text}）");
    }

    /// 构造一个零宽 range 作为组合起点。
    ///
    /// 两条 TSF 事实决定了实现方式：
    ///
    /// 1. `ITfContext::GetStart` 返回 **`IAnchor`** 而不是 `ITfRange`——两者是
    ///    不同的 TSF 接口，`IAnchor` 没有 `ShiftStart`/`SetText`。
    /// 2. 拿 `ITfRange` 最稳的途径是 `GetSelection` 返回的 `TF_SELECTION.range`
    ///    （它一定是有效 range，且已绑定到本 context）。
    ///
    /// 拿到 range 后折叠到起点，再用 `ShiftStart`/`ShiftEnd` 双移把零宽点
    /// 挪到目标 `acp_start`。**必须两个都移**：只移 start 会得到一个「起点在
    /// 目标、终点在选区」的非零宽 range，后面 `SetText` 会覆盖掉选区原有内容。
    unsafe fn make_range(&self, ctx: &ITfContext) -> Result<ITfRange> {
        #![allow(unsafe_op_in_unsafe_fn)]
        use windows::Win32::UI::TextServices::{TF_ANCHOR_START, TF_SELECTION};

        let mut sel: [TF_SELECTION; 1] = [TF_SELECTION {
            range: core::mem::ManuallyDrop::new(None),
            style: Default::default(),
        }];
        let mut fetched = 0u32;
        ctx.GetSelection(0, 0, &mut sel, &mut fetched)?;

        let base: Option<ITfRange> = core::mem::ManuallyDrop::take(&mut sel[0].range);
        let Some(range) = base else {
            return Err(E_POINTER.into());
        };

        // 折叠成零宽点
        range.Collapse(0, TF_ANCHOR_START)?;

        // 移位到目标 acp（相对当前位置的偏移）
        if self.acp_start != 0 {
            let mut moved = 0i32;
            range.ShiftStart(0, self.acp_start, &mut moved, core::ptr::null())?;
            let mut moved2 = 0i32;
            range.ShiftEnd(0, self.acp_start, &mut moved2, core::ptr::null())?;
        }
        Ok(range)
    }
}

/// 请求一次 edit session。
///
/// `sync` 决定用 `TF_ES_SYNC` 还是 `TF_ES_ASYNC`。
/// 返回 `Ok(())` 只表示请求**已提交**（异步时不代表已完成）。
pub fn run_edit_session(
    ctx: &ITfContext,
    runtime: Arc<CompositionRuntime>,
    task: EditTask,
    text: String,
    acp_start: i32,
    sync: bool,
) -> Result<()> {
    let session = CompositionEditSession::new(runtime, task, text, acp_start, ctx.clone());
    let iface: ITfEditSession = session.into();
    let flags = if sync { TF_ES_SYNC } else { TF_ES_ASYNC };
    // ⚠ 返回值不是普通 HRESULT：成功时可能是 `TS_S_ASYNC`（异步已排队），
    // 而不是 `S_OK`。用 `?` 会把它当错误吞掉，所以要显式判断。
    let hr = unsafe { ctx.RequestEditSession(0, &iface, flags)? };
    // TS_S_ASYNC = 0x00040001。其它成功值（S_OK / S_FALSE）也接受。
    const TS_S_ASYNC: i32 = 0x0004_0001;
    if hr.0 < 0 && hr.0 != TS_S_ASYNC {
        tracing::warn!("RequestEditSession 返回失败: 0x{:08X}", hr.0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_starts_idle() {
        let r = CompositionRuntime::new();
        assert_eq!(r.state(), CompositionState::Idle);
        assert!(!r.has_composition());
    }

    /// 空文本不建组合。
    #[test]
    fn empty_text_plans_nothing() {
        let r = CompositionRuntime::new();
        assert_eq!(r.plan_text(0), Action::None);
        assert_eq!(r.state(), CompositionState::Idle);
    }

    #[test]
    fn first_text_plans_start() {
        let r = CompositionRuntime::new();
        assert_eq!(r.plan_text(2), Action::Start { session: 0 });
    }

    /// 宿主强杀后状态清干净，且不再产生任何动作。
    #[test]
    fn host_termination_clears_everything() {
        let r = CompositionRuntime::new();
        r.plan_text(3);
        assert_eq!(r.on_host_terminated(), EndReason::TerminatedByHost);
        assert_eq!(r.state(), CompositionState::Idle);
        assert_eq!(r.plan_end(true), Action::None, "已失效的组合不能再结束");
    }

    #[test]
    fn focus_lost_resets() {
        let r = CompositionRuntime::new();
        r.plan_text(3);
        r.on_focus_lost();
        assert_eq!(r.state(), CompositionState::Idle);
    }

    /// 结束流程：plan_end → 清理 → 空闲。
    #[test]
    fn end_then_cleanup() {
        let r = CompositionRuntime::new();
        r.plan_text(3);
        assert!(matches!(r.plan_end(true), Action::End { .. }));
        assert!(r.state().is_ending());
        r.on_cleanup_done();
        assert_eq!(r.state(), CompositionState::Idle);
    }
}
