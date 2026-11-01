//! host-render 候选窗的「按下-松开同一点才算点选」判据。
//!
//! 原实现：`aurora_tsf/include/HostClickPolicy.h`。
//!
//! 宿主自绘候选窗时，鼠标事件经 `CMD_PANEL_POINTER` 报来。点选要求**按下与松开命中
//! 同一个候选**——否则用户按住拖动到别的候选再松手也算选中，与 Windows 原生行为不符。

/// 「没有候选被按下」的哨兵值。用 `i32::MIN` 而不是 -1：
/// -1 在命中表里是「上翻页」的合法下标。
pub const NONE: i32 = i32::MIN;

/// 这次松开是否构成点选。
///
/// - `pressed`：按下时命中的候选下标（或 [`NONE`]）。
/// - `hit`：松开时命中的候选下标。
#[inline]
pub fn selects_on_release(pressed: i32, hit: i32) -> bool {
    pressed != NONE && pressed == hit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_target_selects() {
        assert!(selects_on_release(3, 3));
    }

    /// 按下后拖到别的候选再松手 ⇒ 不选。
    #[test]
    fn different_target_does_not_select() {
        assert!(!selects_on_release(3, 5));
    }

    /// 没在候选上按下 ⇒ 松开时落在候选上也不算点选（那是「拖窗后碰巧松在候选上」）。
    #[test]
    fn no_press_never_selects() {
        assert!(!selects_on_release(NONE, 3));
        assert!(!selects_on_release(NONE, NONE));
    }

    /// -1 / -2 是翻页按钮的合法下标，不能被当成「无」。
    #[test]
    fn negative_indices_are_real_targets() {
        assert_eq!(NONE, i32::MIN, "哨兵必须远离 -1/-2");
        assert!(selects_on_release(-1, -1), "上翻页按钮");
        assert!(selects_on_release(-2, -2), "下翻页按钮");
        assert!(!selects_on_release(-1, -2), "从一个翻页钮拖到另一个不算");
    }
}
