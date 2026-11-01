//! TSF 事件 sink：焦点与文档管理器事件。
//!
//! 原实现：`aurora_tsf/src/TextService.cpp` 的 `OnSetFocus` / `OnKillThreadFocus` 等
//! （约 1200 行，含大量业务逻辑）。
//!
//! # 本文件的范围
//!
//! **只做事件接收与状态记账**，业务逻辑（发 IPC、建组合、更新语言栏）留给后续。
//! 先让事件通路打通，因为「事件收不到」和「事件收到了处理错」是两类完全不同的
//! 故障，而前者会掩盖后者。
//!
//! # 两层事件的区别（容易混）
//!
//! | 接口 | 粒度 | 何时触发 |
//! |---|---|---|
//! | `ITfThreadMgrEventSink` | **文档管理器级** | 焦点在 DocMgr 之间移动 |
//! | `ITfThreadFocusSink` | **进程级** | 整个应用获得/失去前台 |
//!
//! 两者不可互相替代。原实现 `OnKillThreadFocus` 的注释里有详细论证（为什么不能
//! 用 DocMgr 级判据挡进程级事件），这里保留结论。
//!
//! # 线程亲和
//!
//! TSF 的事件回调都在**创建 ThreadMgr 的那个线程**上调用，即 UI 线程。
//! 因此内部状态用 `Mutex` 保护是**为了跨线程读取**（IPC 线程要读当前焦点），
//! 不是为了回调之间的并发。

#![cfg(windows)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use windows::Win32::UI::TextServices::{
    ITfContext, ITfDocumentMgr, ITfThreadFocusSink, ITfThreadFocusSink_Impl, ITfThreadMgrEventSink,
    ITfThreadMgrEventSink_Impl,
};
use windows::core::Result;

/// 焦点事件记账。
///
/// 记录「当前焦点在哪」，供 IPC 层组装 `FocusGained` / `FocusLost`。
#[derive(Debug, Default)]
pub struct FocusState {
    /// 当前焦点 DocMgr 的地址（仅作实例标识，不解引用）。
    focused_doc_mgr: AtomicU64,
    /// 本线程是否持有焦点。
    has_focus: AtomicBool,
    /// 本线程是否在前台（进程级）。
    has_thread_focus: AtomicBool,
    /// 焦点会话序号：每次焦点真正变化时递增。
    ///
    /// 单独存在的理由：IPC 层用它判断「收到的响应属于哪一次焦点」。
    /// 焦点快速切换时（点击 A → 点击 B），A 的迟到响应不该被当成 B 的。
    session_id: AtomicU64,
    /// 焦点变化计数（诊断用，只增不减）。
    focus_change_count: AtomicU64,
    /// 最近一次焦点切换前是哪个 DocMgr（用于 `FocusLost` 的 reason 判定）。
    prev_doc_mgr: AtomicU64,
    /// 受保护的附加信息：焦点 DocMgr 是否可编辑（由 COM 层查询后写入）。
    context_flags: Mutex<FocusContextFlags>,
}

/// 与焦点上下文相关的标志位，由 COM 层在查询后写入。
#[derive(Debug, Default, Clone, Copy)]
pub struct FocusContextFlags {
    /// 焦点 DocMgr 有可编辑的上下文（非只读、非过渡型）。
    pub has_text_input_context: bool,
    /// 焦点在密码框（由 InputScope 判定）。
    pub is_password: bool,
}

impl FocusState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 处理 `OnSetFocus`。
    ///
    /// 返回 `true` 表示焦点**真的变了**（`pdimfocus` 与之前不同），调用方据此决定
    /// 要不要发 IPC。同一 DocMgr 重复收到 `OnSetFocus` 是正常的（切换输入法会触发），
    /// 不该重复上报——原实现对此有专门的 `willSkipFocusGained` 判据。
    pub fn on_set_focus(&self, focus: Option<u64>, prev_focus: Option<u64>) -> bool {
        let new = focus.unwrap_or(0);
        let old = self.focused_doc_mgr.swap(new, Ordering::SeqCst);
        self.prev_doc_mgr
            .store(prev_focus.unwrap_or(0), Ordering::SeqCst);

        if new == old {
            // 焦点没变（同一 DocMgr 重复触发）——不上报，但焦点状态仍要刷新
            self.has_focus.store(new != 0, Ordering::SeqCst);
            return false;
        }

        self.has_focus.store(new != 0, Ordering::SeqCst);
        self.session_id.fetch_add(1, Ordering::SeqCst);
        self.focus_change_count.fetch_add(1, Ordering::SeqCst);
        true
    }

    /// 处理 `OnKillThreadFocus`（整个应用失去前台）。
    ///
    /// 与原实现的差异：这里只清状态，**不清 `prev_doc_mgr`**——它要留给
    /// `CleanupInputStateForDocChange` 当 hint 用（组合建在那个 DocMgr 上，
    /// 而此刻 `GetFocus()` 未必可用）。
    pub fn on_kill_thread_focus(&self) {
        self.has_thread_focus.store(false, Ordering::SeqCst);
        self.has_focus.store(false, Ordering::SeqCst);
    }

    /// 处理 `OnSetThreadFocus`。
    pub fn on_set_thread_focus(&self) {
        self.has_thread_focus.store(true, Ordering::SeqCst);
    }

    pub fn focused_doc_mgr(&self) -> u64 {
        self.focused_doc_mgr.load(Ordering::SeqCst)
    }

    pub fn prev_doc_mgr(&self) -> u64 {
        self.prev_doc_mgr.load(Ordering::SeqCst)
    }

    pub fn has_focus(&self) -> bool {
        self.has_focus.load(Ordering::SeqCst)
    }

    pub fn has_thread_focus(&self) -> bool {
        self.has_thread_focus.load(Ordering::SeqCst)
    }

    pub fn session_id(&self) -> u64 {
        self.session_id.load(Ordering::SeqCst)
    }

    pub fn focus_change_count(&self) -> u64 {
        self.focus_change_count.load(Ordering::SeqCst)
    }

    pub fn set_context_flags(&self, flags: FocusContextFlags) {
        if let Ok(mut g) = self.context_flags.lock() {
            *g = flags;
        }
    }

    pub fn context_flags(&self) -> FocusContextFlags {
        self.context_flags.lock().map(|g| *g).unwrap_or_default()
    }
}

/// `ITfThreadMgrEventSink` 的实现。
///
/// 持有一个 `Arc<FocusState>`，与 `TextService` 共享同一份记账。
#[windows::core::implement(ITfThreadMgrEventSink)]
pub struct ThreadMgrEventSink {
    pub state: std::sync::Arc<FocusState>,
}

impl ThreadMgrEventSink {
    pub fn new(state: std::sync::Arc<FocusState>) -> Self {
        Self { state }
    }
}

impl ITfThreadMgrEventSink_Impl for ThreadMgrEventSink_Impl {
    /// DocMgr 初始化（新建文档）。
    ///
    /// 原实现在这里不做业务，只记日志。保持同样行为。
    fn OnInitDocumentMgr(&self, _pdim: Option<&ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    /// DocMgr 销毁。
    fn OnUninitDocumentMgr(&self, _pdim: Option<&ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    /// **焦点切换的主入口**。
    ///
    /// ⚠ 正常路径下不该在这里做重活：同步 edit session 在 `OnSetFocus` 期间通常
    /// 会被宿主拒绝（`TS_E_SYNCHRONOUS`）。原实现把取光标位置等操作推迟到
    /// 异步路径。本文件只记状态。
    fn OnSetFocus(
        &self,
        pdimfocus: Option<&ITfDocumentMgr>,
        pdimprevfocus: Option<&ITfDocumentMgr>,
    ) -> Result<()> {
        use windows::core::Interface;
        let focus = pdimfocus.map(|p| p.as_raw() as usize as u64);
        let prev = pdimprevfocus.map(|p| p.as_raw() as usize as u64);
        self.state.on_set_focus(focus, prev);
        Ok(())
    }

    fn OnPushContext(&self, _pic: Option<&ITfContext>) -> Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, _pic: Option<&ITfContext>) -> Result<()> {
        Ok(())
    }
}

/// `ITfThreadFocusSink` 的实现：进程级前台变化。
#[windows::core::implement(ITfThreadFocusSink)]
pub struct ThreadFocusSink {
    pub state: std::sync::Arc<FocusState>,
}

impl ThreadFocusSink {
    pub fn new(state: std::sync::Arc<FocusState>) -> Self {
        Self { state }
    }
}

impl ITfThreadFocusSink_Impl for ThreadFocusSink_Impl {
    fn OnSetThreadFocus(&self) -> Result<()> {
        self.state.on_set_thread_focus();
        Ok(())
    }

    /// 整个应用失去前台。
    ///
    /// ⚠ 这里**刻意不做** `willSkipFocusGained` 那样的对称预判。原实现有三条理由，
    /// 任一条单独成立：
    ///
    /// 1. 判据跨层：`locked`/`transient` 守卫是 **DocMgr 级**判据，而线程失焦是
    ///    **进程级**事件；服务端的 client token 也是进程级。拿前者挡后者，与
    ///    「三层判据不可跨层复用」是同一个错误。
    /// 2. 会造成净回归：真 transient 场景（explorer 地址栏）里，该进程此前必定有过
    ///    正常 DocMgr 的 `focus_gained`，此刻进程确实失焦了，这条 lost 是**对的**。
    ///    挡掉它，切到非 TSF 宿主时工具栏就永远不隐藏。
    /// 3. 服务端已有结构性防线：只放行 token == active 的失焦，而 active 只能由
    ///    `focus_gained` / `ime_activated` 设置 ⇒ 从未获焦的 token 发来的 lost 必被丢弃。
    fn OnKillThreadFocus(&self) -> Result<()> {
        self.state.on_kill_thread_focus();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_without_focus() {
        let s = FocusState::new();
        assert!(!s.has_focus());
        assert!(!s.has_thread_focus());
        assert_eq!(s.focused_doc_mgr(), 0);
        assert_eq!(s.session_id(), 0);
    }

    /// 焦点真的变了才返回 true。
    #[test]
    fn focus_change_is_detected() {
        let s = FocusState::new();
        assert!(s.on_set_focus(Some(0x1000), None), "从无到有");
        assert_eq!(s.session_id(), 1);
        assert_eq!(s.focused_doc_mgr(), 0x1000);

        assert!(s.on_set_focus(Some(0x2000), Some(0x1000)), "换到另一个");
        assert_eq!(s.session_id(), 2);
        assert_eq!(s.prev_doc_mgr(), 0x1000);
    }

    /// 同一 DocMgr 重复触发不算变化——切换输入法会这样，重复上报会让服务端
    /// 反复重置状态。
    #[test]
    fn same_doc_mgr_is_not_a_change() {
        let s = FocusState::new();
        s.on_set_focus(Some(0x1000), None);
        let before = s.session_id();
        assert!(!s.on_set_focus(Some(0x1000), Some(0x1000)), "同一 DocMgr");
        assert_eq!(s.session_id(), before, "会话号不该增");
        assert!(s.has_focus(), "但仍算有焦点");
    }

    /// 失焦：focus = None。
    #[test]
    fn losing_focus_clears_state() {
        let s = FocusState::new();
        s.on_set_focus(Some(0x1000), None);
        assert!(s.has_focus());
        assert!(s.on_set_focus(None, Some(0x1000)), "失去焦点是变化");
        assert!(!s.has_focus());
        assert_eq!(s.focused_doc_mgr(), 0);
    }

    /// 进程级失焦要同时清 has_focus。
    #[test]
    fn kill_thread_focus_clears_both() {
        let s = FocusState::new();
        s.on_set_thread_focus();
        s.on_set_focus(Some(0x1000), None);
        assert!(s.has_thread_focus());

        s.on_kill_thread_focus();
        assert!(!s.has_thread_focus());
        assert!(!s.has_focus(), "进程失焦也意味着文档失焦");
    }

    /// 关键：OnKillThreadFocus **不清 prev_doc_mgr**。
    ///
    /// 它要留给 CleanupInputStateForDocChange 当 hint——组合建在那个 DocMgr 上，
    /// 而此刻 GetFocus() 未必可用。不传则可能落到 forced cleanup，
    /// 把残留文本提交进文档（Excel/WPS 表格的 'd' 漏字）。
    #[test]
    fn kill_thread_focus_keeps_prev_doc_mgr_as_hint() {
        let s = FocusState::new();
        s.on_set_focus(Some(0x2000), Some(0x1000));
        assert_eq!(s.prev_doc_mgr(), 0x1000);

        s.on_kill_thread_focus();
        assert_eq!(
            s.prev_doc_mgr(),
            0x1000,
            "prev 必须保留：清理时要用它当 hint 定位组合所在的 DocMgr"
        );
    }

    #[test]
    fn set_thread_focus_marks_foreground() {
        let s = FocusState::new();
        assert!(!s.has_thread_focus());
        s.on_set_thread_focus();
        assert!(s.has_thread_focus());
    }

    /// 焦点变化计数只增不减（诊断用）。
    #[test]
    fn focus_change_count_only_grows() {
        let s = FocusState::new();
        assert_eq!(s.focus_change_count(), 0);
        s.on_set_focus(Some(1), None);
        s.on_set_focus(Some(2), Some(1));
        s.on_set_focus(None, Some(2));
        assert_eq!(s.focus_change_count(), 3);
    }

    #[test]
    fn context_flags_roundtrip() {
        let s = FocusState::new();
        assert!(
            !s.context_flags().has_text_input_context,
            "默认无输入上下文"
        );
        s.set_context_flags(FocusContextFlags {
            has_text_input_context: true,
            is_password: true,
        });
        let f = s.context_flags();
        assert!(f.has_text_input_context);
        assert!(f.is_password);
    }
}
