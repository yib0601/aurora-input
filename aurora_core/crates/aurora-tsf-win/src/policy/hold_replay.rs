//! 智能符号 hold 预览态（组合态预览）下「吃键 → 收口 → 重放」的触发判据。
//!
//! 原实现：`aurora_tsf/include/HoldReplayPolicy.h`。
//!
//! # 背景
//!
//! hold 期间符号挂在 TSF 组合里，OnTestKeyDown 按「有会话」把键吃下；OnKeyDown 里服务端
//! 因缓冲为空回 PassThrough、并已同步收口符号。此时若吐成 `pfEaten=FALSE` 就是「吃了再吐」
//! ——补发 WM_KEYDOWN 的宿主无感，不补发的宿主直接丢键。凡命中本判据的键，改为保持吃下 +
//! SendInput 重放，宿主先看到收口后的文档、再看到一个与组合无关的普通按键。

/// 本次 keydown 是否走 hold 重放。
///
/// - `hold_active_before_response`：处理服务端响应**之前**采样的 hold 状态
///   （PassThrough 分支会收口，之后就查不到了）。
/// - `eaten_by_response`：服务端响应处理后的 `pfEaten`。为真说明服务端自己出了字
///   （全角空格等），键已被正当消费，不能再重放。
/// - `is_hold_replay_key`：回车 / 空格 / 数字 / 方向键等的结论。
/// - `is_ctrl_alt_cleanup`：会话中的 Ctrl/Alt 组合（不含修饰键本身、不含已注册热键）。
///   A2-65 / t263：漏了这一条时，hold 期间的 Ctrl+V 在微信、WPS 表格里粘贴无效。
#[inline]
pub fn should_replay_after_hold(
    hold_active_before_response: bool,
    eaten_by_response: bool,
    is_hold_replay_key: bool,
    is_ctrl_alt_cleanup: bool,
) -> bool {
    hold_active_before_response && !eaten_by_response && (is_hold_replay_key || is_ctrl_alt_cleanup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_when_hold_active_and_not_eaten() {
        assert!(
            should_replay_after_hold(true, false, true, false),
            "hold 中 + 重放键"
        );
    }

    /// 服务端自己出了字（全角空格等）就已被正当消费，不能再重放。
    #[test]
    fn does_not_replay_when_server_emitted() {
        assert!(!should_replay_after_hold(true, true, true, false));
    }

    #[test]
    fn does_not_replay_without_hold() {
        assert!(!should_replay_after_hold(false, false, true, false));
        assert!(!should_replay_after_hold(false, false, false, true));
    }

    /// A2-65 / t263 回归：hold 期间的 Ctrl+V 必须重放，否则微信 / WPS 粘贴无效。
    #[test]
    fn ctrl_alt_cleanup_triggers_replay() {
        assert!(
            should_replay_after_hold(true, false, false, true),
            "hold 中的 Ctrl+组合也必须重放"
        );
    }

    #[test]
    fn neither_key_class_does_not_replay() {
        assert!(!should_replay_after_hold(true, false, false, false));
    }
}
