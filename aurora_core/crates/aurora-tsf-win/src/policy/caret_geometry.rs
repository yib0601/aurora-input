//! 光标矩形相关的纯几何判据：默认位置识别、组合矩形兜底、候选点接受。
//!
//! 原实现：`aurora_tsf/include/CaretDefaultPosPolicy.h` +
//! `include/ImmCandidateFormPolicy.h`。
//!
//! # 为什么这些判据值得单独存在
//!
//! TSF 给的 caret / composition 矩形经常是**退化**的（宽高为 0、或干脆是宿主填的默认值
//! `{0,0,0,0}`）。候选窗若照它摆位，就会出现「候选框贴在屏幕左上角」或「跟着组合框乱跳」。
//! 这些函数就是逐一识别那些退化形态，每一处都有真机案例。
//!
//! 原实现是模板（`template <class RectT>`），Rust 侧用具体的 [`Rect`]——这里只需要 i32。

/// 一个矩形（TSF 的 `RECT` 语义：left/top/right/bottom）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// 是否退化（高 <= 0，即空矩形）。
    #[inline]
    pub fn is_empty_vertical(&self) -> bool {
        self.bottom <= self.top
    }

    /// 宽是否为正。
    #[inline]
    pub fn has_positive_width(&self) -> bool {
        self.right > self.left
    }

    /// 高是否为正。
    #[inline]
    pub fn has_positive_height(&self) -> bool {
        self.bottom > self.top
    }
}

/// 两个矩形是否完全相同。
#[inline]
pub fn same_rect(a: &Rect, b: &Rect) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

/// 这个 caret 是不是宿主填的**默认位置**而非真实位置。
///
/// 判据：caret 高度退化为 0，且它同时等于组合起点矩形与组合矩形——三者都一样，
/// 说明宿主根本没算过，只是把同一个默认值抄了三份。
#[inline]
pub fn is_host_default_position(
    caret: &Rect,
    has_comp_start: bool,
    comp_start: &Rect,
    has_comp_rect: bool,
    comp_rect: &Rect,
) -> bool {
    caret.is_empty_vertical()
        && has_comp_start
        && has_comp_rect
        && same_rect(caret, comp_start)
        && same_rect(caret, comp_rect)
}

/// 是否展示「已排版」的组合（而非退化组合）。
///
/// 用于判断能否拿组合矩形当光标位置的替代：caret 有高度、组合矩形非空、两者**不同**
/// （相同说明都是默认值）、且 caret 横向落在组合矩形内。
#[inline]
pub fn shows_laid_out_composition(
    caret_ok: bool,
    caret: &Rect,
    has_comp_rect: bool,
    comp_rect: &Rect,
) -> bool {
    caret_ok
        && caret.has_positive_height()
        && has_comp_rect
        && comp_rect.has_positive_width()
        && comp_rect.has_positive_height()
        && !same_rect(caret, comp_rect)
        && caret.left >= comp_rect.left
        && caret.left <= comp_rect.right
        && caret.top < comp_rect.bottom
        && caret.bottom > comp_rect.top
}

/// 能否退回用组合矩形定位候选窗。
///
/// - `host_default_pos`：光标是宿主默认值（见 [`is_host_default_position`]）⇒ 不回退，
///   拿默认值摆位只会贴到屏幕角落。
/// - `comp_rect_width`：组合矩形宽度必须为正。
/// - `comp_rect_inside_screen_ext`：组合矩形必须落在屏幕范围内，否则候选窗会被摆到屏幕外。
#[inline]
pub fn allow_composition_rect_fallback(
    host_default_pos: bool,
    has_screen_ext: bool,
    has_comp_rect: bool,
    comp_rect_width: i32,
    comp_rect_inside_screen_ext: bool,
) -> bool {
    !host_default_pos
        && has_screen_ext
        && has_comp_rect
        && comp_rect_width > 0
        && comp_rect_inside_screen_ext
}

/// 已锁存的位置是否仍然适用（只在有组合时才算数）。
#[inline]
pub fn latch_applies(latched: bool, has_composition: bool) -> bool {
    latched && has_composition
}

// ── IMM 候选窗位置（ImmCandidateFormPolicy）──────────────────────────────

/// `COMPOSITIONFORM.dwStyle`：默认（未指定）。
pub const CFS_DEFAULT: u32 = 0x0000;
/// 按点定位。
pub const CFS_POINT: u32 = 0x0002;
/// 按候选位置定位。
pub const CFS_CANDIDATEPOS: u32 = 0x0040;
/// 排除该区域。
pub const CFS_EXCLUDE: u32 = 0x0080;

/// 这个 style 是否属于「按位置摆候选窗」。
#[inline]
pub fn is_candidate_form_positioned(style: u32) -> bool {
    matches!(style, CFS_POINT | CFS_CANDIDATEPOS | CFS_EXCLUDE)
}

/// 接不接受 IMM 给的候选窗位置。
///
/// 四个条件同时成立才接受：宿主明说「没有真实光标」、确实拿到 form、
/// style 是按位置摆的、且点落在屏幕内。
#[inline]
pub fn accept_candidate_form_point(
    host_says_no_caret: bool,
    got_form: bool,
    style: u32,
    x: i32,
    y: i32,
    has_screen_ext: bool,
    screen_ext: &Rect,
) -> bool {
    host_says_no_caret
        && got_form
        && is_candidate_form_positioned(style)
        && has_screen_ext
        && x >= screen_ext.left
        && x <= screen_ext.right
        && y >= screen_ext.top
        && y <= screen_ext.bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    const R_EMPTY: Rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    const R_CARET: Rect = Rect {
        left: 100,
        top: 50,
        right: 102,
        bottom: 70,
    };
    const R_COMP: Rect = Rect {
        left: 100,
        top: 50,
        right: 300,
        bottom: 70,
    };

    #[test]
    fn same_rect_works() {
        assert!(same_rect(&R_CARET, &R_CARET));
        assert!(!same_rect(&R_CARET, &R_COMP));
    }

    /// 三者相同且高度为 0 ⇒ 宿主默认值（真机：候选框贴屏幕左上角）。
    #[test]
    fn host_default_position_detected() {
        assert!(is_host_default_position(
            &R_EMPTY, true, &R_EMPTY, true, &R_EMPTY
        ));
    }

    #[test]
    fn real_position_is_not_default() {
        // 有高度就不是默认
        assert!(!is_host_default_position(
            &R_CARET, true, &R_CARET, true, &R_CARET
        ));
        // 三个矩形不一致说明宿主算过
        assert!(!is_host_default_position(
            &R_EMPTY, true, &R_CARET, true, &R_EMPTY
        ));
    }

    #[test]
    fn missing_comp_rects_means_not_default() {
        assert!(!is_host_default_position(
            &R_EMPTY, false, &R_EMPTY, true, &R_EMPTY
        ));
        assert!(!is_host_default_position(
            &R_EMPTY, true, &R_EMPTY, false, &R_EMPTY
        ));
    }

    #[test]
    fn laid_out_composition() {
        assert!(shows_laid_out_composition(true, &R_CARET, true, &R_COMP));
    }

    #[test]
    fn degenerate_composition_is_not_laid_out() {
        assert!(
            !shows_laid_out_composition(true, &R_CARET, false, &R_COMP),
            "没拿到组合矩形"
        );
        assert!(
            !shows_laid_out_composition(false, &R_CARET, true, &R_COMP),
            "caret 不可用"
        );
        assert!(
            !shows_laid_out_composition(true, &R_EMPTY, true, &R_COMP),
            "caret 无高度"
        );
        assert!(
            !shows_laid_out_composition(true, &R_CARET, true, &R_EMPTY),
            "组合矩形退化"
        );
        // caret 与组合矩形相同 ⇒ 都是默认值
        assert!(!shows_laid_out_composition(true, &R_CARET, true, &R_CARET));
    }

    #[test]
    fn caret_outside_composition_is_not_laid_out() {
        let outside = Rect::new(400, 50, 402, 70);
        assert!(!shows_laid_out_composition(true, &outside, true, &R_COMP));
    }

    #[test]
    fn composition_fallback_rules() {
        assert!(allow_composition_rect_fallback(
            false, true, true, 200, true
        ));
        assert!(
            !allow_composition_rect_fallback(true, true, true, 200, true),
            "默认位置不回退"
        );
        assert!(
            !allow_composition_rect_fallback(false, false, true, 200, true),
            "没屏幕范围"
        );
        assert!(
            !allow_composition_rect_fallback(false, true, false, 200, true),
            "没组合矩形"
        );
        assert!(
            !allow_composition_rect_fallback(false, true, true, 0, true),
            "宽度为 0"
        );
        assert!(
            !allow_composition_rect_fallback(false, true, true, 200, false),
            "超出屏幕"
        );
    }

    #[test]
    fn latch_only_with_composition() {
        assert!(latch_applies(true, true));
        assert!(!latch_applies(true, false));
        assert!(!latch_applies(false, true));
    }

    // ── IMM candidate form ──

    #[test]
    fn positioned_styles() {
        assert!(is_candidate_form_positioned(CFS_POINT));
        assert!(is_candidate_form_positioned(CFS_CANDIDATEPOS));
        assert!(is_candidate_form_positioned(CFS_EXCLUDE));
        assert!(!is_candidate_form_positioned(CFS_DEFAULT));
    }

    const SCREEN: Rect = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    #[test]
    fn accepts_valid_form_point() {
        assert!(accept_candidate_form_point(
            true, true, CFS_POINT, 100, 200, true, &SCREEN
        ));
    }

    #[test]
    fn rejects_when_any_condition_fails() {
        // 宿主没说「无光标」——那我们有更好的位置来源，不用 IMM 的
        assert!(!accept_candidate_form_point(
            false, true, CFS_POINT, 100, 200, true, &SCREEN
        ));
        assert!(!accept_candidate_form_point(
            true, false, CFS_POINT, 100, 200, true, &SCREEN
        ));
        // style 不是按位置摆的
        assert!(!accept_candidate_form_point(
            true,
            true,
            CFS_DEFAULT,
            100,
            200,
            true,
            &SCREEN
        ));
        // 点超出屏幕
        assert!(!accept_candidate_form_point(
            true, true, CFS_POINT, -10, 200, true, &SCREEN
        ));
        assert!(!accept_candidate_form_point(
            true, true, CFS_POINT, 100, 2000, true, &SCREEN
        ));
    }
}
