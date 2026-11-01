//! 多行文本以剪贴板粘贴方式上屏（compat 规则 `multiline_commit = "paste"`）。
//!
//! 原实现：`aurora_tsf/include/MultilineCommit.h`。
//!
//! 有的宿主（新版微信）收到含换行的 TSF 上屏时，会把组合区的编码显示残留在界面上。
//! 对这类宿主改为：结束组合 → 文本写入剪贴板 → 注入 Ctrl+V。是否开启由服务端按应用
//! 决定，经 `CONFIG_KEY_MULTILINE_COMMIT` 下发。
//!
//! ⚠ 原实现用 `std::wstring`（UTF-16）；Rust 侧统一用 `&str`（UTF-8）。换行判断与
//! CRLF 规整对两种编码是同一逻辑（`\r` / `\n` 在两个编码里都是单字节/单码元且值相同），
//! 故不引入 UTF-16 中间层。**但上报给 TSF 时要转回 UTF-16**，那一步在 COM 层做。

/// `CONFIG_KEY_MULTILINE_COMMIT` 的值是否表示 paste。
///
/// 只认 1，其余（含将来的新档）按 commit——未知档位退回更安全的那一侧。
#[inline]
pub fn is_paste_mode(mode: u8) -> bool {
    mode == 1
}

/// 文本里有没有换行（CR 或 LF）。
#[inline]
pub fn contains_newline(text: &str) -> bool {
    text.contains('\r') || text.contains('\n')
}

/// 换行统一成 CRLF（`CF_UNICODETEXT` 的约定）。
///
/// `\r\n` 算一个换行，孤立的 `\r` 或 `\n` 都转成 `\r\n`。
pub fn normalize_to_crlf(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8 + 2);
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                // `\r\n` 只算一个换行
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\r\n");
            }
            '\n' => out.push_str("\r\n"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mode_1_is_paste() {
        assert!(is_paste_mode(1));
        assert!(!is_paste_mode(0));
        assert!(!is_paste_mode(2), "未知档位按 commit（更安全的一侧）");
        assert!(!is_paste_mode(255));
    }

    #[test]
    fn newline_detection() {
        assert!(contains_newline("a\nb"));
        assert!(contains_newline("a\rb"));
        assert!(contains_newline("a\r\nb"));
        assert!(!contains_newline("ab"));
        assert!(!contains_newline(""));
    }

    #[test]
    fn lf_becomes_crlf() {
        assert_eq!(normalize_to_crlf("a\nb"), "a\r\nb");
    }

    #[test]
    fn cr_becomes_crlf() {
        assert_eq!(normalize_to_crlf("a\rb"), "a\r\nb");
    }

    /// `\r\n` 必须算**一个**换行，不能变成 `\r\r\n`。
    #[test]
    fn crlf_stays_single() {
        assert_eq!(normalize_to_crlf("a\r\nb"), "a\r\nb");
    }

    #[test]
    fn already_crlf_is_idempotent() {
        let once = normalize_to_crlf("a\nb\r\nc\rd");
        assert_eq!(once, "a\r\nb\r\nc\r\nd");
        assert_eq!(normalize_to_crlf(&once), once, "规整过一次后再规整不变");
    }

    #[test]
    fn text_without_newlines_is_unchanged() {
        assert_eq!(normalize_to_crlf("hello 世界"), "hello 世界");
        assert_eq!(normalize_to_crlf(""), "");
    }

    /// 中文与 emoji 原样保留（UTF-8 多字节不能被按字节处理打散）。
    #[test]
    fn multibyte_text_survives() {
        assert_eq!(normalize_to_crlf("你好\n世界🌍"), "你好\r\n世界🌍");
    }
}
