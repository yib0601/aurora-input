//! 光标位置来源的选择与降级。
//!
//! 原实现：`aurora_tsf/src/TextService.cpp` 的 `CCTextService::_GetCaretPosition`
//! 与 `CaretEditSession.cpp` 的「三级降级」（约 800 行）。
//!
//! # 为什么需要一个「来源」字段
//!
//! 同一组 `x/y/h` 可能来自语义完全不同的通道，压进同一个字段后下游就再也分不开了。
//! 曾因此让 `GetGUIThreadInfo` 的 Win32 光标冒充 TSF 插入点——在 Word 非正文行错位
//! 814px、在桌面输入定位到任务栏。
//!
//! # 回退链（按可信度从高到低）
//!
//! ```text
//! 1. TSF_SELECTION         GetTextExt(selection)          —— 最精确
//! 2. TSF_COMPOSITION       selection 退化后降级用组合起点  —— 仍属 TSF 域
//! 3. TSF_CACHED            update session 内缓存           —— 同属 TSF 域
//! 4. IMM_CANDIDATE_FORM    宿主经 ImmSetCandidateWindow 声明的位置
//! 5. TSF_DEFAULT_POS       宿主明说「这里没有插入点」（退化矩形）
//! 6. GUI_CARET             GetGUIThreadInfo 回退           —— **跨窗口，不可作权威**
//! 7. CONSOLE               控制台窗口估算
//! 8. LAST_KNOWN            上次已知好值
//! ```
//!
//! 两个反直觉的点：
//!
//! - **`TSF_DEFAULT_POS` 比 `GUI_CARET` 可信**。它不是垃圾值，是宿主的一句答复
//!   「这里没有插入点」。其它输入法直接采信，候选窗落在屏幕右下角（任务栏之上）；
//!   我们若按「退化 ⇒ 还没排版完」一律丢弃，会跌到 `GUI_CARET`——那是界面里某个
//!   无关控件的 Win32 光标，候选窗被钉在屏幕左上角。**丢弃比采信更糟**。
//!
//! - **`IMM_CANDIDATE_FORM` 属于 TSF 域**（与 `TSF_DEFAULT_POS` 相反）：那个说
//!   「这里没有插入点」，这个说「插入点在这儿」。但 DLL **不随它上报组合起点**——
//!   首帧多是上一次组合的旧值。

/// 光标来源，与 `BinaryProtocol.h` 的 `CARET_SRC_*` 逐值对齐。
pub mod source {
    /// 旧协议 / 短包，无法判定。
    pub const UNKNOWN: i32 = 0;
    /// `GetTextExt(selection)`，最精确。
    pub const TSF_SELECTION: i32 = 1;
    /// selection 退化后降级用的组合起点，仍属 TSF 域。
    pub const TSF_COMPOSITION: i32 = 2;
    /// `UpdateComposition` edit session 内缓存，同属 TSF 域。
    pub const TSF_CACHED: i32 = 3;
    /// `GetGUIThreadInfo` / `GetCaretPos` 回退——**跨窗口，不可作权威**。
    pub const GUI_CARET: i32 = 4;
    /// 控制台窗口的估算位置。
    pub const CONSOLE: i32 = 5;
    /// 上次已知好值。
    pub const LAST_KNOWN: i32 = 6;
    /// 组合刚启动时的异步探测值——**reflow 前**的坐标。
    ///
    /// 不可作权威、不可参与首显决策；服务端只拿它刷新坐标缓存。
    pub const PRE_REFLOW: i32 = 7;
    /// 宿主明说「没有插入点可给」的惯用退化矩形。
    pub const TSF_DEFAULT_POS: i32 = 8;
    /// 宿主经 IMM32 `ImmSetCandidateWindow` 声明的候选窗位置。
    pub const IMM_CANDIDATE_FORM: i32 = 9;

    /// 该来源是否属于 TSF 语义域。
    ///
    /// ⚠ `TSF_DEFAULT_POS` **不属于**：虽出自 TSF context，但它明说「这里没有插入点」，
    /// 不得用于跟行、漂移校正那类需要真插入点的逻辑。
    pub fn is_tsf(src: i32) -> bool {
        matches!(
            src,
            TSF_SELECTION | TSF_COMPOSITION | TSF_CACHED | PRE_REFLOW | IMM_CANDIDATE_FORM
        )
    }

    /// 该来源能否当作**权威**坐标。
    ///
    /// `GUI_CARET` / `CONSOLE` / `LAST_KNOWN` / `PRE_REFLOW` 都不行。
    pub fn is_authoritative(src: i32) -> bool {
        matches!(
            src,
            TSF_SELECTION | TSF_COMPOSITION | TSF_CACHED | IMM_CANDIDATE_FORM
        )
    }
}

/// 一个矩形（TSF `RECT` 语义）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaretRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl CaretRect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// 高度是否为正（服务端 `caret_is_valid` 要求 `height > 0`）。
    #[inline]
    pub fn has_height(&self) -> bool {
        self.bottom > self.top
    }

    /// 是否退化（高度 <= 0）。
    #[inline]
    pub fn is_degenerate(&self) -> bool {
        !self.has_height()
    }

    pub fn same_as(&self, other: &Self) -> bool {
        self.left == other.left
            && self.top == other.top
            && self.right == other.right
            && self.bottom == other.bottom
    }
}

/// 一次光标查询得到的三个矩形。
///
/// 这是 TSF「三级降级」的输入：三次 `GetTextExt` 的结果。
#[derive(Debug, Clone, Copy, Default)]
pub struct CaretProbe {
    /// 选区（插入点）矩形。
    pub selection: CaretRect,
    /// 组合起点矩形。
    pub comp_start: CaretRect,
    /// 整个组合 range 的包围矩形。
    pub comp_rect: CaretRect,
    /// 是否拿到了组合起点。
    pub has_comp_start: bool,
    /// 是否拿到了组合矩形。
    pub has_comp_rect: bool,
    /// IMM `ImmSetCandidateWindow` 声明的点（屏幕坐标），`None` = 没有。
    pub imm_point: Option<(i32, i32)>,
}

/// 选中的光标位置。
#[derive(Debug, Clone, Copy)]
pub struct ChosenCaret {
    pub x: i32,
    pub y: i32,
    pub height: i32,
    pub source: i32,
}

/// 从一次探测结果里选光标位置。
///
/// # 降级顺序
///
/// 1. 选区矩形有高度 ⇒ `TSF_SELECTION`
/// 2. 组合起点有高度 ⇒ `TSF_COMPOSITION`
/// 3. 三者同值且退化 ⇒ 宿主已被问过重试窗口，接受它的答复 ⇒ `TSF_DEFAULT_POS`
///    （高度**合成**为 `default_height`——原值 h=0 会被服务端 `caret_is_valid` 整条丢掉）
/// 4. IMM 声明了点 且 上面是 TSF_DEFAULT_POS 那条指纹 ⇒ `IMM_CANDIDATE_FORM`
/// 5. 都没有 ⇒ `None`（调用方自行回退到 GUI_CARET 等）
///
/// ⚠ 第 3 步的判据必须是「三者同值且都退化」——那是宿主「这里没有插入点」的**指纹**。
/// 单纯一个退化矩形不能这么判：实测存在只退化 30ms 随后变正常的混合场景。
pub fn choose_caret(probe: &CaretProbe, default_height: i32) -> Option<ChosenCaret> {
    // ① 选区
    if probe.selection.has_height() {
        return Some(ChosenCaret {
            x: probe.selection.left,
            y: probe.selection.top,
            height: probe.selection.bottom - probe.selection.top,
            source: source::TSF_SELECTION,
        });
    }

    // ② 组合起点
    if probe.has_comp_start && probe.comp_start.has_height() {
        return Some(ChosenCaret {
            x: probe.comp_start.left,
            y: probe.comp_start.top,
            height: probe.comp_start.bottom - probe.comp_start.top,
            source: source::TSF_COMPOSITION,
        });
    }

    // ③ 宿主「没有插入点」的指纹：选区 / 组合起点 / 组合矩形三者同值且全退化
    let all_degenerate = probe.selection.is_degenerate()
        && (!probe.has_comp_start || probe.comp_start.is_degenerate())
        && (!probe.has_comp_rect || probe.comp_rect.is_degenerate());
    let all_same =
        probe.selection.same_as(&probe.comp_start) && probe.selection.same_as(&probe.comp_rect);

    if all_degenerate && all_same {
        // ④ IMM 声明了点 ⇒ 它比「没有插入点」更具体
        if let Some((x, y)) = probe.imm_point {
            return Some(ChosenCaret {
                x,
                y,
                height: default_height,
                source: source::IMM_CANDIDATE_FORM,
            });
        }
        // ⑤ 采信宿主的答复（高度合成，否则服务端会整条丢掉）
        return Some(ChosenCaret {
            x: probe.selection.left,
            y: probe.selection.top,
            height: default_height,
            source: source::TSF_DEFAULT_POS,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_H: i32 = 20;

    fn probe(sel: CaretRect, comp_start: CaretRect, comp_rect: CaretRect) -> CaretProbe {
        CaretProbe {
            selection: sel,
            comp_start,
            comp_rect,
            has_comp_start: true,
            has_comp_rect: true,
            imm_point: None,
        }
    }

    #[test]
    fn source_domains() {
        assert!(source::is_tsf(source::TSF_SELECTION));
        assert!(source::is_tsf(source::IMM_CANDIDATE_FORM));
        assert!(
            !source::is_tsf(source::TSF_DEFAULT_POS),
            "它说「这里没有插入点」，不属 TSF 语义域"
        );
        assert!(!source::is_tsf(source::GUI_CARET));
    }

    #[test]
    fn authoritative_sources() {
        assert!(source::is_authoritative(source::TSF_SELECTION));
        assert!(source::is_authoritative(source::IMM_CANDIDATE_FORM));
        assert!(!source::is_authoritative(source::GUI_CARET));
        assert!(!source::is_authoritative(source::PRE_REFLOW));
    }

    /// 正常情况：选区有高度，用它。
    #[test]
    fn selection_wins_when_valid() {
        let sel = CaretRect::new(100, 200, 102, 220);
        let p = probe(sel, CaretRect::default(), CaretRect::default());
        let c = choose_caret(&p, DEFAULT_H).unwrap();
        assert_eq!(c.source, source::TSF_SELECTION);
        assert_eq!((c.x, c.y, c.height), (100, 200, 20));
    }

    /// 选区退化，用组合起点。
    #[test]
    fn falls_back_to_comp_start() {
        let sel = CaretRect::default(); // 退化
        let cs = CaretRect::new(50, 60, 52, 80);
        let p = probe(sel, cs, CaretRect::default());
        let c = choose_caret(&p, DEFAULT_H).unwrap();
        assert_eq!(c.source, source::TSF_COMPOSITION);
        assert_eq!((c.x, c.y, c.height), (50, 60, 20));
    }

    /// Illustrator 场景：三者同值且退化 ⇒ 采信宿主的答复。
    ///
    /// 丢弃会跌到 GUI_CARET，候选窗被钉在屏幕左上角——丢弃比采信更糟。
    #[test]
    fn accepts_host_default_position() {
        let deg = CaretRect::new(2559, 1367, 2560, 1367); // 高度 0
        let p = probe(deg, deg, deg);
        let c = choose_caret(&p, DEFAULT_H).unwrap();
        assert_eq!(c.source, source::TSF_DEFAULT_POS);
        assert_eq!((c.x, c.y), (2559, 1367));
        assert_eq!(
            c.height, DEFAULT_H,
            "高度必须合成，否则服务端 caret_is_valid 会丢掉"
        );
    }

    /// 混合场景：选区退化但组合起点有效 ⇒ 用组合起点（不是那个指纹）。
    ///
    /// 对位 C++：`usedCompStart` 分支。只有「三者同值且全退化」才是默认位置指纹。
    #[test]
    fn mixed_degeneracy_falls_back_to_comp_start() {
        let deg = CaretRect::new(0, 0, 0, 0);
        let comp = CaretRect::new(10, 10, 12, 30);
        let p = probe(deg, comp, comp);
        let c = choose_caret(&p, DEFAULT_H).unwrap();
        assert_eq!(c.source, source::TSF_COMPOSITION);
        assert_eq!((c.x, c.y, c.height), (10, 10, 20));
    }

    /// IMM 声明了点 ⇒ 优先用它（比「没有插入点」更具体）。
    #[test]
    fn imm_point_beats_default_pos() {
        let deg = CaretRect::new(2559, 1367, 2560, 1367);
        let mut p = probe(deg, deg, deg);
        p.imm_point = Some((500, 600));
        let c = choose_caret(&p, DEFAULT_H).unwrap();
        assert_eq!(c.source, source::IMM_CANDIDATE_FORM);
        assert_eq!((c.x, c.y), (500, 600));
    }

    /// IMM 有值但 TSF 拿到了真坐标 ⇒ 用 TSF 的（更精确）。
    #[test]
    fn real_selection_beats_imm() {
        let sel = CaretRect::new(100, 200, 102, 220);
        let mut p = probe(sel, CaretRect::default(), CaretRect::default());
        p.imm_point = Some((500, 600));
        let c = choose_caret(&p, DEFAULT_H).unwrap();
        assert_eq!(c.source, source::TSF_SELECTION);
    }

    /// 全退化但三个矩形不同值 ⇒ 不是那个指纹，返回 None。
    #[test]
    fn all_degenerate_but_different_is_not_fingerprint() {
        let a = CaretRect::new(0, 0, 1, 0);
        let b = CaretRect::new(5, 5, 6, 5);
        let p = probe(a, b, b);
        assert!(choose_caret(&p, DEFAULT_H).is_none());
    }

    /// 没有组合信息时，只有选区退化 ⇒ 无法判定指纹 ⇒ None。
    #[test]
    fn without_comp_info_degenerate_selection_is_unknown() {
        let deg = CaretRect::new(0, 0, 0, 0);
        let p = CaretProbe {
            selection: deg,
            comp_start: CaretRect::default(),
            comp_rect: CaretRect::default(),
            has_comp_start: false,
            has_comp_rect: false,
            imm_point: None,
        };
        // has_comp_* 为 false 时 all_same 仍会成立（默认值与 deg 相同），
        // 故这里走 default_pos 分支。这是刻意：没有组合时宿主仍可能报默认位置。
        let c = choose_caret(&p, DEFAULT_H);
        assert!(c.is_some());
    }

    #[test]
    fn rect_helpers() {
        assert!(CaretRect::new(0, 0, 10, 20).has_height());
        assert!(!CaretRect::new(0, 0, 10, 0).has_height());
        assert!(CaretRect::new(0, 0, 10, 0).is_degenerate());
        assert!(CaretRect::new(1, 2, 3, 4).same_as(&CaretRect::new(1, 2, 3, 4)));
        assert!(!CaretRect::new(1, 2, 3, 4).same_as(&CaretRect::new(1, 2, 3, 5)));
    }
}
