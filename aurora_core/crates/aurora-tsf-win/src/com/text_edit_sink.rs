//! `ITfTextEditSink` + 光标位置提取。
//!
//! 原实现：`aurora_tsf/src/TextService.cpp` 的 `OnEndEdit` 与
//! `src/CaretEditSession.cpp`（772 行）。
//!
//! # 为什么必须走 edit session
//!
//! TSF 的几乎所有 context 操作（`GetSelection` / `GetTextExt` / `GetActiveView`）
//! 都要求当前线程持有该 context 的**编辑锁**。直接调会返回 `TF_E_NOLOCK`。
//!
//! 而 `OnSetFocus` 期间**同步** edit session 通常会被宿主拒绝（`TS_E_SYNCHRONOUS`）——
//! 因为宿主还在处理焦点切换，重入会死锁。这就是原实现把光标查询推迟到异步路径的原因。
//!
//! # 本文件的做法
//!
//! `OnEndEdit` 是**安全的**时机：它是宿主主动通知「这次编辑结束了」，此刻我们
//! 已经在一个合法的编辑上下文里，可以直接读。
//!
//! # 三级降级
//!
//! `GetTextExt` 可能对三种 range 给出不同结果，先试最好的：
//!
//! 1. 选区（插入点）—— 最精确
//! 2. 组合起点 —— selection 退化时的降级
//! 3. 组合整体矩形 —— 前两者都不可用时的兜底
//!
//! 选择逻辑在 [`crate::policy::caret_source`]（纯逻辑、已单测），本文件只负责
//! 把 TSF 的返回值填进去。

#![cfg(windows)]

use windows::Win32::Foundation::RECT;
use windows::Win32::UI::TextServices::{
    IClonableWrapper, ITfCompositionView, ITfContext, ITfContextView, ITfEditRecord, ITfRange,
    ITfTextEditSink, ITfTextEditSink_Impl, TF_ANCHOR_START, TF_SELECTION,
};
use windows::core::{Interface, Result};

use crate::policy::caret_source::{CaretProbe, CaretRect, ChosenCaret, choose_caret};

/// 光标高度为 0 时用的合成高度。
///
/// 原实现 `AURORA_DEFAULT_CARET_HEIGHT`。服务端 `caret_is_valid` 要求 `height > 0`，
/// 不合成的话「默认位置」那一条会被整条丢掉——等于修了个寂寞。
pub const DEFAULT_CARET_HEIGHT: i32 = 20;

/// 从 TSF context 读光标位置。
///
/// 返回 `None` 表示三种查询都没给出可用结果（调用方应回退到 GUI_CARET 等）。
///
/// # Safety
/// 调用方必须持有该 context 的编辑锁（在 edit session 内，或 `OnEndEdit` 回调中）。
pub unsafe fn read_caret_from_context(ctx: &ITfContext) -> Option<ChosenCaret> {
    #![allow(unsafe_op_in_unsafe_fn)]
    let view: ITfContextView = ctx.GetActiveView().ok()?;

    let mut probe = CaretProbe::default();

    // ① 选区（插入点）。
    //
    // `ITfContext::GetSelection(ec, ulindex, pselection, pcfetched)` 返回
    // `TF_SELECTION`，其中的 `range` 正是 `ITfContextView::GetTextExt` 要的东西。
    // `ec = 0` = 当前编辑上下文（OnEndEdit 内正确）。
    let mut sel: [TF_SELECTION; 1] = [TF_SELECTION {
        range: core::mem::ManuallyDrop::new(None),
        style: Default::default(),
    }];
    let mut fetched = 0u32;
    if ctx.GetSelection(0, 0, &mut sel, &mut fetched).is_ok() && fetched > 0 {
        // 从 ManuallyDrop 里取出 range
        let range: Option<ITfRange> = unsafe { core::mem::ManuallyDrop::take(&mut sel[0].range) };
        if let Some(rect) = range_rect(&view, range) {
            probe.selection = rect;
        }
    }

    // ② 组合起点 / ③ 组合整体矩形。
    //
    // ⚠ 这里**不受「选区取失败」守卫**：selection 取失败或退化时，组合起点往往仍有效。
    // 实测 shell 的临时输入小窗：selection 恒返回退化矩形 (2559,1367,2560,1367) h=0，
    // 而 composition range 给出有效的 (473,189,473,217)。那时它是手上唯一可信的坐标，
    // 却曾因为「选区失败就不取组合」的守卫压根没去取。
    if let Some((start_rect, full_rect)) = read_composition_rects(ctx, &view) {
        if let Some(r) = start_rect {
            probe.comp_start = r;
            probe.has_comp_start = true;
        }
        if let Some(r) = full_rect {
            probe.comp_rect = r;
            probe.has_comp_rect = true;
        }
    }

    let chosen = choose_caret(&probe, DEFAULT_CARET_HEIGHT);

    // 释放 view（`GetActiveView` 返回了引用）
    let _ = &view;
    if let Some(mut c) = chosen {
        // 坐标兜底：TSF 给的是屏幕坐标，直接可用
        if c.height <= 0 {
            c.height = DEFAULT_CARET_HEIGHT;
        }
        Some(c)
    } else {
        None
    }
}

/// 取一个 range 的屏幕矩形。
///
/// `ec = 0` 表示「当前编辑上下文」——在 `OnEndEdit` 回调内这是正确的。
unsafe fn range_rect(view: &ITfContextView, range: Option<ITfRange>) -> Option<CaretRect> {
    #![allow(unsafe_op_in_unsafe_fn)]
    let range = range?;
    let mut rc = RECT::default();
    let mut clipped = windows::Win32::Foundation::BOOL(0);
    view.GetTextExt(0, &range, &mut rc, &mut clipped).ok()?;
    Some(CaretRect::new(rc.left, rc.top, rc.right, rc.bottom))
}

/// 编辑事件状态。
#[derive(Debug, Default)]
pub struct EditState {
    /// 最近一次成功读到的光标（供 IPC 层重复上报用）。
    last_caret: std::sync::Mutex<Option<ChosenCaret>>,
    /// 累计 `OnEndEdit` 次数（诊断）。
    end_edit_count: std::sync::atomic::AtomicU64,
}

impl EditState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_caret(&self) -> Option<ChosenCaret> {
        self.last_caret.lock().ok().and_then(|g| *g)
    }

    pub fn note_end_edit(&self) {
        self.end_edit_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn end_edit_count(&self) -> u64 {
        self.end_edit_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// `ITfTextEditSink` 实现。
#[windows::core::implement(ITfTextEditSink)]
pub struct TextEditSink {
    pub state: std::sync::Arc<EditState>,
}

impl TextEditSink {
    pub fn new(state: std::sync::Arc<EditState>) -> Self {
        Self { state }
    }
}

impl ITfTextEditSink_Impl for TextEditSink_Impl {
    /// 一次编辑操作结束。
    ///
    /// # 为什么在这里读光标
    ///
    /// 宿主主动通知「这次编辑结束了」⇒ 我们已经在合法的编辑上下文里，可以直接调
    /// `GetSelection` / `GetTextExt`，不需要再包一层 edit session。
    ///
    /// # ⚠ 不要在这里做重活
    ///
    /// 这个回调在宿主的 UI 线程上。任何阻塞（IPC 往返、文件 IO）都会直接卡住用户打字。
    /// 原实现的教训：`OnSetFocus` 里的同步 edit session 被 `TS_E_SYNCHRONOUS` 拒，
    /// 后来改成异步；同理这里只做本地读取，上报交给 IPC 层异步做。
    fn OnEndEdit(
        &self,
        pic: Option<&ITfContext>,
        _ecreadonly: u32,
        _peditrecord: Option<&ITfEditRecord>,
    ) -> Result<()> {
        self.state.note_end_edit();

        let Some(ctx) = pic else {
            return Ok(());
        };

        // SAFETY: OnEndEdit 回调内持有编辑锁，可以直接读。
        if let Some(caret) = unsafe { read_caret_from_context(ctx) } {
            if let Ok(mut g) = self.state.last_caret.lock() {
                *g = Some(caret);
            }
            tracing::debug!(
                "OnEndEdit: caret=({}, {}) h={} src={}",
                caret.x,
                caret.y,
                caret.height,
                caret.source
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_state_starts_clean() {
        let s = EditState::new();
        assert!(s.last_caret().is_none());
        assert_eq!(s.end_edit_count(), 0);
    }

    #[test]
    fn end_edit_count_accumulates() {
        let s = EditState::new();
        s.note_end_edit();
        s.note_end_edit();
        assert_eq!(s.end_edit_count(), 2);
    }

    /// 默认高度必须 >0——服务端 `caret_is_valid` 要求它。
    #[test]
    fn default_caret_height_is_positive() {
        const _: () = assert!(DEFAULT_CARET_HEIGHT > 0);
    }
}

/// 从 context 读组合的两个矩形。
///
/// 返回 `(组合起点矩形, 组合整体矩形)`，任一可为 `None`。
///
/// # 两个矩形的区别只有「折不折叠」，但答的是完全不同的问题
///
/// - **折叠到起点** ⇒ 答「组合从哪开始」
/// - **不折叠** ⇒ 答「组合占了多大一块」
///
/// 组合换行后这两者分处不同行，只有后者能看出「跨行了」。
///
/// # ⚠ 组合整体矩形当前只上报、不参与决策
///
/// 各宿主对跨行 range 的 `GetTextExt` 行为尚未逐一验证（规范说返回包围矩形，
/// 实现可能只返回首行、也可能失败）。原实现同样只上报 + 打日志。
///
/// # Safety
/// 必须在持有编辑锁的上下文中调用（`OnEndEdit` 内即满足）。
unsafe fn read_composition_rects(
    ctx: &ITfContext,
    view: &ITfContextView,
) -> Option<(Option<CaretRect>, Option<CaretRect>)> {
    #![allow(unsafe_op_in_unsafe_fn)]

    // 拿 context composition 接口
    let Ok(comp_iface) = ctx.cast::<windows::Win32::UI::TextServices::ITfContextComposition>()
    else {
        return None;
    };

    // 枚举组合，取第一个（同一 context 同时只有一个活动组合）。
    //
    // ⚠ 枚举给的是 `ITfCompositionView`（只读视图），不是 `ITfComposition`
    // （可写的那个）。视图的 `GetRange()` 正是我们要的——读位置不需要写权限。
    let Ok(enum_comp) = comp_iface.EnumCompositions() else {
        return None;
    };
    let mut buf: [Option<ITfCompositionView>; 1] = [None];
    let mut fetched = 0u32;
    if enum_comp.Next(&mut buf, &mut fetched).is_err() || fetched == 0 {
        return None;
    }
    let Some(comp_view) = buf[0].take() else {
        return None;
    };

    // 组合 range
    let Ok(comp_range) = comp_view.GetRange() else {
        return None;
    };

    // ③ 整体矩形：不折叠，直接取
    let full = rect_of_range(view, &comp_range);

    // ② 起点矩形：克隆一份再折叠到起点
    //
    // `ITfRange::Clone` 返回 `IAnchor`（不是 ITfRange）。要从它拿到可折叠的 range，
    // 走 `CloneNewWrapper::<ITfRange>()`。
    // `ITfRange::Clone()` 返回 `IAnchor`（不是 ITfRange），而 Collapse 只在
    // ITfRange 上。故走 `IClonableWrapper::CloneNewWrapper::<ITfRange>()` 拿一个
    // 可独立修改的副本。
    let start = (|| -> Option<CaretRect> {
        let clonable: IClonableWrapper = comp_range.cast().ok()?;
        let cloned: ITfRange = clonable.CloneNewWrapper::<ITfRange>().ok()?;
        cloned.Collapse(0, TF_ANCHOR_START).ok()?;
        rect_of_range(view, &cloned)
    })();

    Some((start, full))
}

/// 取一个 range 的屏幕矩形（不折叠）。
///
/// # Safety
/// 同 [`read_composition_rects`]。
unsafe fn rect_of_range(view: &ITfContextView, range: &ITfRange) -> Option<CaretRect> {
    #![allow(unsafe_op_in_unsafe_fn)]
    let mut rc = RECT::default();
    let mut clipped = windows::Win32::Foundation::BOOL(0);
    view.GetTextExt(0, range, &mut rc, &mut clipped).ok()?;
    Some(CaretRect::new(rc.left, rc.top, rc.right, rc.bottom))
}
