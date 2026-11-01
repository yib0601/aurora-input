//! 发往服务端的 `KeyPayload.prevChar` 怎么取。
//!
//! 原实现：`aurora_tsf/include/SmartPunctPrevChar.h`（C++，50 行）。
//!
//! # prevChar 在服务端有两个消费者，判据不同
//!
//! 1. 数字后智能标点（`aurora-punct::is_smart_punct_after_digit`）：只问「是不是 ASCII 数字」。
//! 2. 智能符号 press2 守卫：问「光标前是不是 press1 刚上屏的那个符号」。
//!
//! ★ ① 只认**输入法自己记录的输入序列**（`last_passthrough_digit`：透传出去的数字键 +
//! 经引擎上屏文本的末位），不认 TSF 现读的文档。理由（2026-10-13 定）：
//!
//! - 现读文档受宿主 TSF 支持度左右，只在部分宿主、部分时机生效，同一操作时灵时不灵；
//! - 现读文档会把「光标恰好落在旧数字后面」（点鼠标、方向键挪过去、上次会话留下的数字）
//!   也当成数字后，用户在那里想打全角 `。` `，` 打不出来。
//!
//! 「刚刚打了数字」是用户意图；「光标前碰巧是数字」不是。
//!
//! ⛔ 别让文档值「否决」序列（序列有数字、文档读到别的字就不算数字后）。看似能治「打完数字
//! 用鼠标点到别处」，实则把 TSF 时序依赖带回来：微信这类 `OnEndEdit` 晚到的宿主，快打「对3.」
//! 时缓存里是晚到的「对」，`.` 被误否决成 `。`——正是本次要去掉的时灵时不灵。
//!
//! 只认序列的已知代价（与 macOS 同，见 `SmartPunctDigitTracker.swift`）：
//!
//! - 打完数字后用鼠标把光标点到别处，下一个标点仍按数字后出半角（键盘移动会清零，鼠标看不见）；
//! - 「34 → Backspace → .」出 `。`（退格把序列清零，而用户多半想要 `.`）；
//! - 记账漏掉的上屏路径不再有文档兜底。
//!
//! ② 仍需要文档现读值：宿主能读回时它是 press2 守卫唯一的事实来源。所以现读值照常上报，
//! 只把其中的 ASCII 数字抹成 0——数字对 ② 从来不会匹配（武装串末位恒为符号），抹掉只让 ②
//! 退回「读不回」这一档，与 macOS（只上报序列）同语义；而不抹就会让 ① 重新吃到文档数字。
//!
//! # 判据本体的另一半在别处
//!
//! ① 的最终判定（`is_smart_punct_after_digit`）在 `aurora-punct`，跨端真值表的
//! `smart_punct_after_digit` case 是三端共享的答案。本模块只负责**产出 prevChar**，
//! 即把「文档值 / 序列值 / 本键是不是标点键」折算成一个 u16。

/// ASCII 数字判定。与 `aurora-punct` 的同名判据必须同口径——那边是 ① 的最终消费者。
#[inline]
pub fn is_ascii_digit(c: u16) -> bool {
    (0x30..=0x39).contains(&c)
}

/// 折算要上报的 `prevChar`。
///
/// - `doc_char`：TSF 现读的光标前字符（0 = 读不回）。
/// - `seq_digit`：输入序列记下的数字（0 = 最近一笔不是数字）。
/// - `is_punct_key`：本键是否标点键（序列值只给标点键消费，其余键由调用方清零）。
///
/// 返回 0 表示「服务端按没有前字符处理」。
#[inline]
pub fn resolve_prev_char(doc_char: u16, seq_digit: u16, is_punct_key: bool) -> u16 {
    if is_punct_key && seq_digit != 0 {
        return seq_digit;
    }
    if is_ascii_digit(doc_char) {
        0
    } else {
        doc_char
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO: u16 = 0;
    const DIGIT_3: u16 = 0x33; // '3'
    const DIGIT_5: u16 = 0x35; // '5'
    const LETTER_A: u16 = 0x61; // 'a'
    const FULLWIDTH_3: u16 = 0xFF13; // '３' —— 全角，不算 ASCII 数字

    #[test]
    fn ascii_digits_only() {
        assert!(is_ascii_digit(DIGIT_3));
        assert!(is_ascii_digit(0x30));
        assert!(is_ascii_digit(0x39));
        assert!(!is_ascii_digit(0x2F));
        assert!(!is_ascii_digit(0x3A));
        assert!(!is_ascii_digit(FULLWIDTH_3), "全角数字不算");
    }

    /// 标点键 + 序列有数字 ⇒ 用序列值（不看文档）。
    #[test]
    fn punct_key_prefers_sequence_digit() {
        assert_eq!(resolve_prev_char(LETTER_A, DIGIT_3, true), DIGIT_3);
        // 文档读到数字也不能否决序列值
        assert_eq!(resolve_prev_char(DIGIT_5, DIGIT_3, true), DIGIT_3);
        // 文档读不回也一样
        assert_eq!(resolve_prev_char(ZERO, DIGIT_3, true), DIGIT_3);
    }

    /// 非标点键不给序列值（序列值只给标点键消费）。
    #[test]
    fn non_punct_key_ignores_sequence() {
        assert_eq!(resolve_prev_char(LETTER_A, DIGIT_3, false), LETTER_A);
        assert_eq!(resolve_prev_char(ZERO, DIGIT_3, false), ZERO);
    }

    /// 文档里的 ASCII 数字一律抹成 0——否则 ① 会重新吃到文档数字。
    #[test]
    fn doc_ascii_digits_are_erased() {
        assert_eq!(resolve_prev_char(DIGIT_5, ZERO, false), 0);
        assert_eq!(
            resolve_prev_char(DIGIT_5, ZERO, true),
            0,
            "序列没数字时文档数字也要抹"
        );
    }

    /// 非数字的文档值原样上报（② 守卫需要它）。
    #[test]
    fn non_digit_doc_char_passes_through() {
        assert_eq!(resolve_prev_char(LETTER_A, ZERO, false), LETTER_A);
        assert_eq!(resolve_prev_char(LETTER_A, ZERO, true), LETTER_A);
        // 全角数字不是 ASCII 数字，照传
        assert_eq!(resolve_prev_char(FULLWIDTH_3, ZERO, false), FULLWIDTH_3);
    }

    /// 关键回归：微信这类 OnEndEdit 晚到的宿主，快打「对3.」时文档缓存是「对」。
    ///
    /// 旧实现若让文档值否决序列，`.` 会被误判成非数字后、出全角 `。`。
    #[test]
    fn late_onendedit_does_not_veto_sequence() {
        let doc_char = 0x5BF9; // '对' —— 宿主晚到，缓存里是它而不是 3
        assert_eq!(
            resolve_prev_char(doc_char, DIGIT_3, true),
            DIGIT_3,
            "序列有数字就必须用序列，文档值不得否决"
        );
    }
}
