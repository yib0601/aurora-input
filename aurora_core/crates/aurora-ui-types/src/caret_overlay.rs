//! 候选窗定位调试浮窗（渲染在 aurora-ui 的 `caret_overlay`）的帧数据与判据分支。

/// 候选窗定位调试浮窗的一帧数据。
///
/// ★ 为什么要画出来：定位缺陷本质是**空间**问题——「宿主说组合在这块区域」与「候选窗
/// 画在了那个点」是否对得上。日志只能给出离散数值，人得在脑子里把它们还原成屏幕位置
/// 关系；宿主一多、操作序列一长（打字/换行/逐字删除交替），这个还原就不可靠了。
/// 直接叠在屏幕上画，错位一眼可见。
///
/// 所有坐标都是**屏幕物理坐标**，与 IPC 上报口径一致。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaretOverlayView {
    /// 宿主上报的插入点 `(x, y_bottom, height)`。
    pub caret: (i32, i32, i32),
    /// 宿主上报的组合起点（折叠到起点的 rect 左下角）；`None` = 本帧没有。
    pub comp_start: Option<(i32, i32)>,
    /// 宿主上报的整个组合范围矩形 `(left, top, right, bottom)`；`None` = 本帧没取到。
    pub comp_rect: Option<(i32, i32, i32, i32)>,
    /// 候选窗**实际**使用的锚点，以及它取自哪里（`composition_start` / `caret` /
    /// `shown_anchor`）。这是把「判据算出了什么」与「宿主说了什么」对上的关键一行。
    pub anchor: (i32, i32),
    pub anchor_source: &'static str,
    /// 本帧 caret 坐标来源（`aurora_ipc::protocol::caret_source::*` 的名字）。
    pub caret_source: &'static str,
    /// 组合矩形判据走了哪条分支，直接显示在浮窗上——不必回头翻日志。
    pub rect_verdict: RectVerdict,
    /// 焦点进程名，用于确认 per-app 规则作用在谁身上。
    pub process: String,
}

/// 组合矩形判据的分支，与 `message_handler` 里的判断一一对应。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RectVerdict {
    /// 本帧宿主没给矩形（旧 DLL / macOS / TS_E_NOLAYOUT）。
    #[default]
    Absent,
    /// 采信：锚点取自矩形左下角。
    Trusted,
    /// 退化（宽或高为 0）——整帧不参与锚点决策。
    Degenerate,
    /// 是插入点不是范围（`left >= caret.x`）——回退既有锚点逻辑。
    InsertionPoint,
    /// 不覆盖当前插入行（`caret.y > bottom`）——疑该宿主只返回首行。
    NotCoveringCaretLine,
}

impl RectVerdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Absent => "无矩形",
            Self::Trusted => "采信矩形",
            Self::Degenerate => "退化(w或h=0)",
            Self::InsertionPoint => "插入点非范围",
            Self::NotCoveringCaretLine => "不覆盖插入行",
        }
    }
}
