//! 按键状态机：修饰键跟踪、轻敲检测、事件序号。
//!
//! 原实现：`aurora_tsf/src/KeyEventSink.cpp` 中的 `_modsState` / `_pendingKeyUp*` /
//! `_eventSeq` 相关逻辑（分散在 3683 行里）。
//!
//! # 这一层为什么独立出来
//!
//! 修饰键状态必须由 DLL **自己**跟踪，不能用 `GetKeyState`：
//!
//! - `GetKeyState` 反映的是**消息队列处理时**的状态，而 TSF 回调拿到的按键
//!   可能已经排了一会儿队 —— 用户快速按 `Shift+A` 再松开 Shift，等 `A` 的
//!   keydown 回调到达时 `GetKeyState(VK_SHIFT)` 可能已经是「松开」。
//! - 更糟的是注入键：`SendInput` 的键会经过消息队列，`GetKeyState` 能看见，
//!   于是我们自己注入的键会污染状态。
//!
//! 所以这里按**事件流**累加状态，与 `GetKeyState` 完全解耦。
//!
//! # 轻敲检测
//!
//! 「干净单击」= 按下某个修饰键 → 中途没有别的键 → 500ms 内松开。
//! 用来实现 Shift 单击切中英。中途出现任何别的键就作废。

/// 修饰键位，与 `BinaryProtocol.h` 的 `KEYMOD_*` 同值。
pub mod mods {
    pub const SHIFT: u32 = 0x0001;
    pub const CTRL: u32 = 0x0002;
    pub const ALT: u32 = 0x0004;
    pub const WIN: u32 = 0x0008;
}

/// 轻敲判定窗口（毫秒）。与 C++ `TOGGLE_TAP_THRESHOLD_MS` 同值。
pub const TAP_THRESHOLD_MS: u64 = 500;

/// 修饰键状态机。
#[derive(Debug, Default, Clone)]
pub struct ModifierState {
    /// 当前按下的修饰位（`mods::*` 的组合）。
    current: u32,
    /// 上一次事件的序号（单调递增，用于服务端排序与去重）。
    event_seq: u16,
    /// 待判定的轻敲：按下时刻 + 是哪个修饰键。
    pending_tap: Option<PendingTap>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingTap {
    /// 被按下的修饰键对应的 `mods` 位。
    key: u32,
    /// 按下时刻（毫秒）。
    down_at: u64,
    /// 期间是否出现过别的键（出现即作废）。
    polluted: bool,
}

impl ModifierState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前修饰位。
    pub fn current(&self) -> u32 {
        self.current
    }

    /// 取下一个事件序号（环绕）。
    pub fn next_event_seq(&mut self) -> u16 {
        let s = self.event_seq;
        self.event_seq = self.event_seq.wrapping_add(1);
        s
    }

    pub fn peek_event_seq(&self) -> u16 {
        self.event_seq
    }

    /// 修饰键按下。
    ///
    /// 返回该修饰键对应的位（若是修饰键），否则 `None`。
    pub fn on_modifier_down(&mut self, vk: u32, now_ms: u64) -> Option<u32> {
        let bit = modifier_bit(vk)?;
        self.current |= bit;
        // 只有「首个」修饰键按下才开启轻敲判定——按 Ctrl 再按 Shift，
        // 那是组合键，不是两次单击。
        if self.pending_tap.is_none() {
            self.pending_tap = Some(PendingTap {
                key: bit,
                down_at: now_ms,
                polluted: false,
            });
        } else if let Some(t) = self.pending_tap.as_mut() {
            // 已经有一个待判定的修饰键，再按一个就算污染
            t.polluted = true;
        }
        Some(bit)
    }

    /// 修饰键松开。返回 `true` 表示构成一次干净单击。
    pub fn on_modifier_up(&mut self, vk: u32, now_ms: u64) -> bool {
        let Some(bit) = modifier_bit(vk) else {
            return false;
        };
        self.current &= !bit;

        let Some(tap) = self.pending_tap.take() else {
            return false;
        };
        // 必须是自己那一位、没被污染、且在时限内
        tap.key == bit && !tap.polluted && now_ms.saturating_sub(tap.down_at) <= TAP_THRESHOLD_MS
    }

    /// 非修饰键按下：作废任何待判定的轻敲。
    ///
    /// 这是「Shift+鼠标拖选不算单击」之外的另一半——Shift+字母 也不是单击。
    pub fn on_non_modifier_key(&mut self) {
        if let Some(t) = self.pending_tap.as_mut() {
            t.polluted = true;
        }
    }

    /// 鼠标动作也作废轻敲（Shift+划词）。
    pub fn on_mouse_activity(&mut self) {
        self.pending_tap = None;
    }

    /// 强制复位（失去焦点 / 切换输入法）。
    ///
    /// ⚠ 必须清 `current`：否则失焦时按着 Shift，回来后会以为 Shift 还按着。
    pub fn reset(&mut self) {
        self.current = 0;
        self.pending_tap = None;
    }

    /// 是否有待判定的轻敲（诊断用）。
    pub fn has_pending_tap(&self) -> bool {
        self.pending_tap.is_some()
    }
}

/// VK → 修饰位。非修饰键返回 `None`。
///
/// 同时接受左右通码（`VK_SHIFT`）与分码（`VK_LSHIFT`/`VK_RSHIFT`）：
/// TSF 的不同阶段给的是不同的码，两种都要认。
pub fn modifier_bit(vk: u32) -> Option<u32> {
    match vk {
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Some(mods::SHIFT),
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => Some(mods::CTRL),
        VK_MENU | VK_LMENU | VK_RMENU => Some(mods::ALT),
        VK_LWIN | VK_RWIN => Some(mods::WIN),
        _ => None,
    }
}

// 与 <winuser.h> 同值。这些常量在原实现里由 `KeyEventSink.h` 的 static_assert 钉住。
pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_MENU: u32 = 0x12;
pub const VK_LWIN: u32 = 0x5B;
pub const VK_RWIN: u32 = 0x5C;
pub const VK_LSHIFT: u32 = 0xA0;
pub const VK_RSHIFT: u32 = 0xA1;
pub const VK_LCONTROL: u32 = 0xA2;
pub const VK_RCONTROL: u32 = 0xA3;
pub const VK_LMENU: u32 = 0xA4;
pub const VK_RMENU: u32 = 0xA5;

#[cfg(test)]
mod tests {
    use super::*;

    const VK_A: u32 = 0x41;

    #[test]
    fn modifier_bits_are_distinct() {
        let bits = [
            modifier_bit(VK_SHIFT).unwrap(),
            modifier_bit(VK_CONTROL).unwrap(),
            modifier_bit(VK_MENU).unwrap(),
            modifier_bit(VK_LWIN).unwrap(),
        ];
        let set: std::collections::HashSet<_> = bits.iter().collect();
        assert_eq!(set.len(), 4, "四个修饰位必须互不相同");
    }

    /// 左右分码与通码映射到同一位——TSF 不同阶段给不同的码。
    #[test]
    fn left_right_map_to_same_bit() {
        assert_eq!(modifier_bit(VK_LSHIFT), modifier_bit(VK_SHIFT));
        assert_eq!(modifier_bit(VK_RSHIFT), modifier_bit(VK_SHIFT));
        assert_eq!(modifier_bit(VK_LCONTROL), modifier_bit(VK_CONTROL));
        assert_eq!(modifier_bit(VK_RMENU), modifier_bit(VK_MENU));
    }

    #[test]
    fn non_modifier_has_no_bit() {
        assert!(modifier_bit(VK_A).is_none());
        assert!(modifier_bit(0x20).is_none(), "空格不是修饰键");
    }

    #[test]
    fn modifier_state_accumulates() {
        let mut s = ModifierState::new();
        assert_eq!(s.current(), 0);
        s.on_modifier_down(VK_SHIFT, 0);
        assert_eq!(s.current(), mods::SHIFT);
        s.on_modifier_down(VK_CONTROL, 1);
        assert_eq!(s.current(), mods::SHIFT | mods::CTRL);
        s.on_modifier_up(VK_SHIFT, 2);
        assert_eq!(s.current(), mods::CTRL);
    }

    /// 干净单击：按下 → 松开，中间没别的键。
    #[test]
    fn clean_tap_is_detected() {
        let mut s = ModifierState::new();
        s.on_modifier_down(VK_SHIFT, 1000);
        assert!(s.on_modifier_up(VK_SHIFT, 1200), "200ms 内松开 = 干净单击");
    }

    /// 超时就作废。
    #[test]
    fn slow_release_is_not_a_tap() {
        let mut s = ModifierState::new();
        s.on_modifier_down(VK_SHIFT, 1000);
        assert!(!s.on_modifier_up(VK_SHIFT, 1000 + TAP_THRESHOLD_MS + 1));
    }

    /// 边界：恰好等于阈值仍算单击。
    #[test]
    fn tap_threshold_boundary() {
        let mut s = ModifierState::new();
        s.on_modifier_down(VK_SHIFT, 1000);
        assert!(s.on_modifier_up(VK_SHIFT, 1000 + TAP_THRESHOLD_MS));
    }

    /// Shift+字母 不是单击——中间有别的键就作废。
    #[test]
    fn key_between_cancels_tap() {
        let mut s = ModifierState::new();
        s.on_modifier_down(VK_SHIFT, 1000);
        s.on_non_modifier_key(); // 按了 A
        assert!(!s.on_modifier_up(VK_SHIFT, 1100), "Shift+A 不是单击");
    }

    /// Shift+鼠标拖选 不是单击。
    #[test]
    fn mouse_activity_cancels_tap() {
        let mut s = ModifierState::new();
        s.on_modifier_down(VK_SHIFT, 1000);
        s.on_mouse_activity();
        assert!(!s.on_modifier_up(VK_SHIFT, 1100));
    }

    /// 组合键不是两次单击：Ctrl 按下后再按 Shift，两者都不算单击。
    #[test]
    fn chained_modifiers_are_not_taps() {
        let mut s = ModifierState::new();
        s.on_modifier_down(VK_CONTROL, 1000);
        s.on_modifier_down(VK_SHIFT, 1050);
        assert!(!s.on_modifier_up(VK_SHIFT, 1100), "第二个修饰键不算");
        assert!(!s.on_modifier_up(VK_CONTROL, 1150), "第一个被污染");
    }

    /// 松开一个从未按下的修饰键不该产生轻敲。
    #[test]
    fn release_without_press_is_not_a_tap() {
        let mut s = ModifierState::new();
        assert!(!s.on_modifier_up(VK_SHIFT, 1000));
    }

    /// 松开 A 却挂着 Shift 的待判定，不算 Shift 的单击。
    #[test]
    fn releasing_different_modifier_does_not_fire() {
        let mut s = ModifierState::new();
        s.on_modifier_down(VK_SHIFT, 1000);
        // 松开的是 Ctrl（虽然没按过）
        assert!(!s.on_modifier_up(VK_CONTROL, 1100));
    }

    /// 复位必须清修饰位——否则失焦时按着 Shift，回来会以为还按着。
    #[test]
    fn reset_clears_modifier_state() {
        let mut s = ModifierState::new();
        s.on_modifier_down(VK_SHIFT, 1000);
        s.reset();
        assert_eq!(s.current(), 0, "复位后不该记得 Shift 还按着");
        assert!(!s.has_pending_tap());
    }

    #[test]
    fn event_seq_monotonic_and_wraps() {
        let mut s = ModifierState::new();
        assert_eq!(s.next_event_seq(), 0);
        assert_eq!(s.next_event_seq(), 1);
        assert_eq!(s.peek_event_seq(), 2);
        // 环绕不 panic
        for _ in 0..70000 {
            s.next_event_seq();
        }
    }

    /// 时钟回退时 `saturating_sub` 得到 0，仍算在时限内（不 panic）。
    #[test]
    fn clock_backwards_is_safe() {
        let mut s = ModifierState::new();
        s.on_modifier_down(VK_SHIFT, 5000);
        assert!(s.on_modifier_up(VK_SHIFT, 1000), "时钟回退时距离算 0");
    }
}
