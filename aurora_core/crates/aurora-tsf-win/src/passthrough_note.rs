//! `PassthroughNote` —— 「自上一个 keydown 送达服务端以来，有键进了宿主」的记账。
//!
//! 原实现：`aurora_tsf/include/PassthroughNote.h`（C++，147 行）。逐条对齐，语义不变。
//!
//! # 为什么要这张表
//!
//! 服务端的智能符号 press2 判定要知道「press1 与 press2 之间有没有别的输入」。中文模式
//! 下它自己看得见（编码缓冲、按键事件），但**英文半角**下看不见：TSF 只吃标点键，字母
//! 数字直接透传给宿主，服务端既无事件也无缓冲。只有 DLL 知道这件事，于是由它经
//! `KeyPayload` 的 `toggles` 位（`TOGGLE_PASSTHROUGH_KEY`）如实上报。
//!
//! 分工同 `_last_passthrough_digit`：**DLL 报事实，服务端持策略**。
//!
//! # 判据必须失效安全，但「多报」的代价分两种
//!
//! 漏报 ⇒ 服务端把 press2 判成真，`ReplaceBackward` 删掉用户刚打的字。**不可接受**。
//! 多报 ⇒ 武装态被解除，用户重按一次 press1。可接受——**但只在触发源是用户按键时**。
//!
//! ★ 若触发源是**我们自己的注入机制**，多报就从「一次性不便」变成「系统性失效」：
//! `CommitText` 在 TSF 提交失败时回退 `SendInput`（每字符一个 `VK_PACKET`），于是
//! press1 每次上屏都把自己注入的那串字符记成「用户输入」，press2 必被解除 ⇒ 智能符号
//! 在那类宿主上整条功能失效。而走这条兜底的恰恰是 `prev_char` 读不回的那批宿主
//! （微信 / 终端），也就是本功能最需要生效的地方，且换个宿主就好了、极难归因。
//!
//! ⇒ 记账的排除项按这条规则取舍：**凡「我们自己造出来的键」一律不记**，其余一律记。
//!
//! ⚠️ 与 `SkipKeyTable` 同一条局限：**本模块只管记账语义，管不了接线**。守卫挂在哪几个
//! 函数、`suppress` 在哪个分支调，这些单测覆盖不到，只能靠接线处的注释与真机验证。

// 与 `<winuser.h>` 同值。`KeyEventSink.h` 曾用 `static_assert` 把它们与 Win32 常量钉死；
// 在 Rust 侧由 `com` 层的编译期断言承担同样职责（见 `vk_constants_match_winuser`）。

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

/// `SendInput` 的 Unicode 注入载体（`KEYEVENTF_UNICODE` 恒发这个 VK）。
pub const VK_PACKET: u32 = 0xE7;
/// 本 IME 自注入的「异步提交触发键」。
pub const VK_ASYNC_COMMIT_TRIGGER: u32 = 0xE8;

/// 纯修饰键：按下它本身不往文档里写东西，故不记账——否则「按住 Shift 连按两次 `？`」
/// 这类正常 press2 会被自己中间的 Shift 解除掉。
///
/// 比 `KeyEventSink.cpp` 的 `_IsPureModifierKey` **多 Win 键**：那个谓词答的是「该不该
/// 放行给宿主」（Win 键有系统级语义，不归它管），这个答的是「按下它文档会不会变」。
/// 两个问题不同，集合因此不同；共用一个反而会在将来某次「统一」时静默改掉其中一边。
pub fn is_bare_modifier(vk: u32) -> bool {
    matches!(
        vk,
        VK_SHIFT
            | VK_CONTROL
            | VK_MENU
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_LMENU
            | VK_RMENU
            | VK_LWIN
            | VK_RWIN
    )
}

/// 「这个键是我们自己造出来的」——**按 VK 值判，与 skip 表无关**。
///
/// skip 表那条通路（`try_consume_skip_key` 命中即 `suppress`）只在 `OnTestKeyDown` 上，
/// 而 Chrome / QQ 等宿主会**无视 `pfEaten = FALSE` 仍调 `OnKeyDown`**。那时 skip 条目已被
/// test 消费掉，`OnKeyDown` 的守卫无从抑制 ⇒ 注入的 `VK_PACKET` 照样置位 ⇒ 上面说的
/// 系统性失效在那批宿主上并没有被堵住。
///
/// 故这里再加一道**与消费点无关**的兜底。只列 0xE7 / 0xE8：两者都没有物理键能产生，
/// 误抑制真实用户输入的风险为零。
///
/// ⚠️ 为什么不做「记住刚抑制过的 vk + 时间窗」那种更通用的备忘：那会把
/// `_SimulatePairKey` 的 `VK_RIGHT`、`ReplacePrecedingChars` 的 `VK_BACK` 一并纳入，而它们
/// 是**真实存在的物理键**——备忘残留（宿主没调 `OnKeyDown` 时必然残留）会让紧随其后的一次
/// 真实按键被误抑制，那是**漏报**方向。用「多报一次」换「可能漏报一次」是亏的：那两者
/// 造成的多报都只是一次性的（配对跳出、press2 的删改都发生在动作之后），本就在可接受侧。
pub fn is_self_injected_vk(vk: u32) -> bool {
    vk == VK_PACKET || vk == VK_ASYNC_COMMIT_TRIGGER
}

/// 这一 keydown 是否算「用户的键进了宿主」（排除项见模块文档）。数字后智能标点的输入序列
/// 记账用的也是这一判据，两者必须同口径。
pub fn is_user_key_into_host(vk: u32, eaten: bool, suppressed: bool) -> bool {
    !eaten && !suppressed && !is_self_injected_vk(vk) && !is_bare_modifier(vk)
}

/// 记账状态：一个布尔 + 三条规则。放进类型里是为了让规则可被单测钉住。
#[derive(Debug, Default)]
pub struct PassthroughState {
    pending: bool,
}

impl PassthroughState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 一次 keydown 的结论。
    ///
    /// - `eaten` —— 最终的 `*pfEaten`；吃下了就不是「进了宿主」。
    /// - `suppressed` —— skip 表命中（我们自己注入的键）。
    ///
    /// ★ 抑制**优先于**记账：两个排除项（`suppressed` 与 `is_self_injected_vk`）任一成立
    /// 即不记。
    pub fn note_key_down(&mut self, vk: u32, eaten: bool, suppressed: bool) {
        if is_user_key_into_host(vk, eaten, suppressed) {
            self.pending = true;
        }
    }

    /// keydown 事件发往服务端时消费：返回该不该置 `TOGGLE_PASSTHROUGH_KEY`，并清零。
    ///
    /// ⚠️ **keyup 不得调用本函数**。toggle 键的 keyup 也会走发帧路径，让它顺手消费会把
    /// 事实丢在一个服务端根本不读该位的事件上——症状是「中间按过 Shift 就又能误删一次」。
    /// 事实的归属者永远是下一个 keydown。
    pub fn take_on_key_down_send(&mut self) -> bool {
        let p = self.pending;
        self.pending = false;
        p
    }

    /// 发送失败（IPC 断开 / 服务端重启）：事实从未送达，必须放回去。
    ///
    /// 不放回就是**漏报**——按模块文档立的失效方向，那是唯一不可接受的方向。
    pub fn restore_on_send_failure(&mut self) {
        self.pending = true;
    }

    /// 焦点 / 文档切换：与 skip 表同处清理。跨应用带一个陈旧的事实过去没有意义，
    /// 虽然服务端那边焦点变更也会解除武装、实际无害。
    pub fn reset(&mut self) {
        self.pending = false;
    }

    pub fn pending(&self) -> bool {
        self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK_A: u32 = 0x41;

    #[test]
    fn eaten_key_is_not_noted() {
        let mut s = PassthroughState::new();
        s.note_key_down(VK_A, true, false); // 吃了
        assert!(!s.pending(), "吃下的键没有进宿主，不该记账");
    }

    #[test]
    fn passthrough_key_is_noted() {
        let mut s = PassthroughState::new();
        s.note_key_down(VK_A, false, false); // 透传
        assert!(s.pending());
    }

    #[test]
    fn suppressed_key_is_not_noted() {
        // skip 表命中的键是我们自己注入的，不算用户输入
        let mut s = PassthroughState::new();
        s.note_key_down(VK_A, false, true);
        assert!(!s.pending(), "已抑制的键不该记账");
    }

    #[test]
    fn self_injected_vks_are_never_noted() {
        for vk in [VK_PACKET, VK_ASYNC_COMMIT_TRIGGER] {
            let mut s = PassthroughState::new();
            s.note_key_down(vk, false, false);
            assert!(!s.pending(), "vk=0x{vk:X} 是自己造的，不记");
        }
    }

    #[test]
    fn bare_modifiers_are_not_noted() {
        for vk in [VK_SHIFT, VK_CONTROL, VK_LWIN, VK_RMENU] {
            let mut s = PassthroughState::new();
            s.note_key_down(vk, false, false);
            assert!(!s.pending(), "vk=0x{vk:X} 是纯修饰键，文档没变，不记");
        }
    }

    #[test]
    fn take_clears_the_fact() {
        let mut s = PassthroughState::new();
        s.note_key_down(VK_A, false, false);
        assert!(s.take_on_key_down_send());
        assert!(!s.take_on_key_down_send(), "取过一次就没了");
    }

    /// 发送失败必须放回——漏报是唯一不可接受的方向。
    #[test]
    fn send_failure_restores_the_fact() {
        let mut s = PassthroughState::new();
        s.note_key_down(VK_A, false, false);
        assert!(s.take_on_key_down_send());
        assert!(!s.pending());

        s.restore_on_send_failure();
        assert!(s.pending(), "事实从未送达，必须放回去");
        assert!(s.take_on_key_down_send());
    }

    #[test]
    fn reset_drops_the_fact() {
        let mut s = PassthroughState::new();
        s.note_key_down(VK_A, false, false);
        s.reset();
        assert!(!s.pending());
    }

    /// 连续多个透传键只记一个事实（布尔量，不是计数）。
    #[test]
    fn multiple_passthrough_keys_collapse_to_one_fact() {
        let mut s = PassthroughState::new();
        s.note_key_down(0x41, false, false);
        s.note_key_down(0x42, false, false);
        s.note_key_down(0x43, false, false);
        assert!(s.take_on_key_down_send());
        assert!(!s.take_on_key_down_send(), "只该有一个事实");
    }
}
