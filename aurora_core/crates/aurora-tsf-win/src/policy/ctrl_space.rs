//! Ctrl+空格 中英切换的吃键判据（GH#172）。
//!
//! 原实现：`aurora_tsf/include/CtrlSpacePolicy.h`。
//!
//! # 背景
//!
//! DLL 在 OnTestKeyDown 无条件吃掉 Ctrl+空格（防 Space 落进输入），OnKeyDown 里还有
//! 系统热键失效时的按键侧兜底切换。IDEA / Android Studio 的代码提示正是 Ctrl+空格，
//! 用户关不掉。
//!
//! # 修法
//!
//! 服务端把 `keys.ctrl_space_toggle` 的**有效值**（开关 + 自定义按键里绑 none）经
//! `CONFIG_KEY_CTRL_SPACE_TOGGLE` 下发，关闭时本 DLL 不吃、不兜底，键原样交给宿主。
//!
//! ⚠ Windows 系统输入法开关热键在 msctf 层（keystroke sink 之下）就消费了按键并翻
//! OPENCLOSE compartment，本 DLL 拦不住——那条由服务端拒绝翻转、回包把 compartment 拉回，
//! 而宿主仍收不到键，只能让用户去系统设置里关掉那个热键。

pub const VK_SPACE: u32 = 0x20;
/// 与 `BinaryProtocol.h` 的 `KEYMOD_*` 同值。
pub const MOD_SHIFT: u32 = 0x0001;
pub const MOD_CTRL: u32 = 0x0002;
pub const MOD_ALT: u32 = 0x0004;

/// 未收到服务端下发前的默认值：开（与历史行为一致；DLL 每次重连都从这里起步）。
pub const DEFAULT_ENABLED: bool = true;

/// 是不是**光** Ctrl+空格：多一个 Shift / Alt 就是另一个键（Windows 默认热键也只认光 Ctrl）。
#[inline]
pub fn is_ctrl_space_chord(vk: u32, modifiers: u32) -> bool {
    vk == VK_SPACE && (modifiers & MOD_CTRL) != 0 && (modifiers & (MOD_ALT | MOD_SHIFT)) == 0
}

/// OnTestKeyDown 要不要吃这个键当中英切换。开关关着 ⇒ 不吃，交给宿主。
#[inline]
pub fn should_intercept_ctrl_space(enabled: bool, vk: u32, modifiers: u32) -> bool {
    enabled && is_ctrl_space_chord(vk, modifiers)
}

/// `CONFIG_KEY_CTRL_SPACE_TOGGLE` 的值 → 开关。格式 `enabled(u8)`；空值视为畸形，保持原状。
#[inline]
pub fn decode_toggle_value(data: Option<&[u8]>, current: bool) -> bool {
    match data {
        Some(d) if !d.is_empty() => d[0] != 0,
        _ => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_ctrl_space_is_the_chord() {
        assert!(is_ctrl_space_chord(VK_SPACE, MOD_CTRL));
    }

    #[test]
    fn extra_modifiers_are_different_keys() {
        assert!(
            !is_ctrl_space_chord(VK_SPACE, MOD_CTRL | MOD_SHIFT),
            "Ctrl+Shift+空格 是另一个键"
        );
        assert!(
            !is_ctrl_space_chord(VK_SPACE, MOD_CTRL | MOD_ALT),
            "Ctrl+Alt+空格 是 AltGr"
        );
    }

    #[test]
    fn space_without_ctrl_is_not_the_chord() {
        assert!(!is_ctrl_space_chord(VK_SPACE, 0));
        assert!(!is_ctrl_space_chord(VK_SPACE, MOD_SHIFT));
    }

    #[test]
    fn other_keys_with_ctrl_are_not_the_chord() {
        assert!(!is_ctrl_space_chord(0x41, MOD_CTRL));
    }

    /// 开关关着 ⇒ 一律不吃，键交给宿主（IDEA 的代码提示要靠它）。
    #[test]
    fn disabled_never_intercepts() {
        assert!(!should_intercept_ctrl_space(false, VK_SPACE, MOD_CTRL));
    }

    #[test]
    fn enabled_intercepts_only_the_bare_chord() {
        assert!(should_intercept_ctrl_space(true, VK_SPACE, MOD_CTRL));
        assert!(!should_intercept_ctrl_space(
            true,
            VK_SPACE,
            MOD_CTRL | MOD_SHIFT
        ));
        assert!(!should_intercept_ctrl_space(true, 0x41, MOD_CTRL));
    }

    #[test]
    fn decode_toggle_value_rules() {
        assert!(decode_toggle_value(Some(&[1]), false));
        assert!(!decode_toggle_value(Some(&[0]), true));
        // 空值 / None 是畸形，保持原状
        assert!(decode_toggle_value(Some(&[]), true));
        assert!(!decode_toggle_value(None, false));
    }

    // `DEFAULT_ENABLED` 是编译期常量，用 const 断言表达而不是运行期 assert
    // （clippy::assertions_on_constants）。改它必须是一次有意的决定。
    const _: () = assert!(DEFAULT_ENABLED, "与历史行为一致：DLL 重连后从「开」起步");
}
