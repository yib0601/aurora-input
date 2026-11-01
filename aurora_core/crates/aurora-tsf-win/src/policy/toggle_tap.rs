//! 切换键「轻敲」判定中的鼠标取消信号。
//!
//! 原实现：`aurora_tsf/include/ToggleTapPolicy.h`。
//!
//! Shift / Ctrl 的干净单击（按下 → 中途无别的键 → 500ms 内松开）用来切中英。但用户
//! **Shift+鼠标拖选**时也是「按下 Shift、松开 Shift」，中间只有鼠标动作——若不看鼠标，
//! 每次划词选中都会误切中英。四个信号任一成立即取消。

/// 按下切换键到松开之间可能出现的鼠标信号。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TapSignals {
    /// 信号 1：按下切换键的那一刻鼠标正按着。
    pub mouse_down_at_press: bool,
    /// 信号 2：期间选区变了（拖选的核心特征）。
    pub selection_changed: bool,
    /// 信号 3：松开切换键的那一刻鼠标正按着。
    pub mouse_down_at_release: bool,
    /// 信号 4：期间鼠标按下过（哪怕已松开）。
    pub mouse_pressed_since_press: bool,
}

/// 任一鼠标信号成立即不是「干净单击」。
#[inline]
pub fn is_tap_cancelled_by_mouse(s: &TapSignals) -> bool {
    s.mouse_down_at_press
        || s.selection_changed
        || s.mouse_down_at_release
        || s.mouse_pressed_since_press
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_tap_is_not_cancelled() {
        assert!(!is_tap_cancelled_by_mouse(&TapSignals::default()));
    }

    /// 四个信号各自独立生效——漏掉任一条都会让某类划词误切中英。
    #[test]
    fn each_signal_cancels_independently() {
        for (name, s) in [
            (
                "mouse_down_at_press",
                TapSignals {
                    mouse_down_at_press: true,
                    ..Default::default()
                },
            ),
            (
                "selection_changed",
                TapSignals {
                    selection_changed: true,
                    ..Default::default()
                },
            ),
            (
                "mouse_down_at_release",
                TapSignals {
                    mouse_down_at_release: true,
                    ..Default::default()
                },
            ),
            (
                "mouse_pressed_since_press",
                TapSignals {
                    mouse_pressed_since_press: true,
                    ..Default::default()
                },
            ),
        ] {
            assert!(is_tap_cancelled_by_mouse(&s), "{name} 必须取消轻敲");
        }
    }
}
