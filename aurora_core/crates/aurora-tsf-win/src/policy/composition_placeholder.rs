//! 空文本兜底占位字符（GH#175）。
//!
//! 原实现：`aurora_tsf/include/CompositionPlaceholder.h`。
//!
//! 某些宿主要求组合字符串非空，否则直接不显示。占位档位由 core 按本进程 compat 规则
//! 经 `CONFIG_KEY_COMPOSITION_PLACEHOLDER` 推送：0 = 空格（默认）、1 = ZWSP、
//! 2 = 盲文空白。
//!
//! ⚠ 用 `char` 而非 `u16`：三个占位字符都在 BMP 内（`U+0020` / `U+200B` / `U+2800`），
//! 没有代理对问题。若将来要加星形平面的字符，类型要换成能表达代理对的形式。

/// 空格（默认档）。
pub const SPACE_PLACEHOLDER: char = ' ';
/// 零宽空格。不占位但让组合非空。
pub const ZWSP_PLACEHOLDER: char = '\u{200B}';
/// 盲文空白。占一个字符宽度但不可见。
pub const BLANK_PLACEHOLDER: char = '\u{2800}';

/// 档位 → 占位字符。
#[inline]
pub fn placeholder_for_kind(kind: u8) -> char {
    match kind {
        1 => ZWSP_PLACEHOLDER,
        2 => BLANK_PLACEHOLDER,
        _ => SPACE_PLACEHOLDER,
    }
}

/// 这段文本是否只是占位（用来避免把占位当成用户真实组合）。
#[inline]
pub fn is_placeholder_text(text: &str) -> bool {
    text == SPACE_PLACEHOLDER.to_string()
        || text == ZWSP_PLACEHOLDER.to_string()
        || text == BLANK_PLACEHOLDER.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_maps_to_placeholder() {
        assert_eq!(
            placeholder_for_kind(0),
            SPACE_PLACEHOLDER,
            "0 = 空格（默认）"
        );
        assert_eq!(placeholder_for_kind(1), ZWSP_PLACEHOLDER);
        assert_eq!(placeholder_for_kind(2), BLANK_PLACEHOLDER);
        // 未知档位回落默认，不前向失败
        assert_eq!(placeholder_for_kind(99), SPACE_PLACEHOLDER);
    }

    #[test]
    fn placeholder_detection() {
        assert!(is_placeholder_text(" "));
        assert!(is_placeholder_text("\u{200B}"));
        assert!(is_placeholder_text("\u{2800}"));
    }

    #[test]
    fn real_text_is_not_placeholder() {
        assert!(!is_placeholder_text("ni"));
        assert!(!is_placeholder_text(""));
        assert!(!is_placeholder_text("  "), "两个空格不是占位");
    }
}
