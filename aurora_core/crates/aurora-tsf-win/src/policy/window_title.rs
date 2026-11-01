//! 焦点顶层窗口**标题**的采集判据（compat.toml 的 `title` 条件）。
//!
//! 原实现：`aurora_tsf/include/WindowTitlePolicy.h`。
//!
//! # 标题是用户数据
//!
//! 网页标题、文件名、聊天对象都属于隐私：
//!
//! - 只在服务端经 `CONFIG_KEY_COMPAT_TITLE_MATCH` 推开时才取——规则表里没有标题规则就
//!   一次都不取，DLL 每次重连从「关」起步；
//! - 取法是 `InternalGetWindowText`（不发 `WM_GETTEXT`，宿主 UI 线程卡住也不阻塞），
//!   截断到 [`MAX_TITLE_CHARS`] 个 UTF-16 码元，随 `FocusGained` 的变长段 ③ 上报；
//! - 任何级别的日志只记长度，不记原文。
//!
//! ⚠ 覆盖边界：本模块只有纯判据。Win32 取标题、开关接收、拼帧分别在 COM 层。

/// 标题截断上限（UTF-16 码元）。
pub const MAX_TITLE_CHARS: usize = 256;

/// `CONFIG_KEY_COMPAT_TITLE_MATCH` 的值 → 开关。空值视为畸形，保持原状。
#[inline]
pub fn parse_title_match_value(data: Option<&[u8]>, current: bool) -> bool {
    match data {
        Some(d) if !d.is_empty() => d[0] != 0,
        _ => current,
    }
}

/// 服务端有没有要求采集标题。
///
/// 单独一个函数而不是直接用布尔：将来若加「按进程白名单」之类的额外条件，改这里即可，
/// 调用点不必动。
#[inline]
pub fn should_collect_title(server_enabled: bool) -> bool {
    server_enabled
}

/// 焦点切换时要不要补发一次 `FocusGained`（带上刚能读到的标题）。
///
/// 触发条件：上次报的是「读不到标题」的盲报、标题采集已开启、当前有焦点、且不在组字中。
/// 组字中重报焦点会打断用户的输入。
#[inline]
pub fn should_resync_focus_on_switch(
    sent_title_blind: bool,
    enabled: bool,
    has_focus: bool,
    composing: bool,
) -> bool {
    sent_title_blind && enabled && has_focus && !composing
}

/// 按 UTF-16 码元截断标题，**不切断代理对**。
///
/// 原实现接收 `std::wstring_view`（天然 UTF-16 码元序列）。Rust 侧输入是 `&str`（UTF-8），
/// 若直接按 char 截断，语义会与 C++ 版不同（char 是码点，C++ 是码元）。故这里显式按
/// UTF-16 码元计数并回切到 UTF-8 边界，保持与 C++ 版**逐码元等价**。
///
/// 截断点落在代理对中间时少截一个码元（宁可少发一个字符，不可发半个代理对——TSF 收到
/// 孤立代理会显示成方框或直接丢弃）。
pub fn truncate_title(title: &str) -> &str {
    // 快速路径：码元数不超限就原样返回
    if title.encode_utf16().count() <= MAX_TITLE_CHARS {
        return title;
    }

    // 逐 char 累加码元宽度，停在最后一个不越过上限的 char 边界
    let mut units = 0usize;
    for (idx, ch) in title.char_indices() {
        let w = ch.len_utf16();
        if units + w > MAX_TITLE_CHARS {
            return &title[..idx];
        }
        units += w;
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_value_rules() {
        assert!(parse_title_match_value(Some(&[1]), false));
        assert!(!parse_title_match_value(Some(&[0]), true));
        assert!(
            parse_title_match_value(Some(&[]), true),
            "空值畸形，保持原状"
        );
        assert!(!parse_title_match_value(None, false));
    }

    #[test]
    fn resync_requires_all_four_conditions() {
        assert!(should_resync_focus_on_switch(true, true, true, false));
        // 任一不成立都不补发
        assert!(
            !should_resync_focus_on_switch(false, true, true, false),
            "上次不是盲报"
        );
        assert!(
            !should_resync_focus_on_switch(true, false, true, false),
            "标题采集没开"
        );
        assert!(
            !should_resync_focus_on_switch(true, true, false, false),
            "没焦点"
        );
        assert!(
            !should_resync_focus_on_switch(true, true, true, true),
            "组字中重报会打断输入"
        );
    }

    #[test]
    fn short_title_is_unchanged() {
        assert_eq!(truncate_title("hello"), "hello");
        assert_eq!(truncate_title(""), "");
    }

    /// 纯 ASCII 按码元 = 按 char，截断到上限。
    #[test]
    fn ascii_title_truncates_at_limit() {
        let s = "a".repeat(MAX_TITLE_CHARS + 50);
        assert_eq!(truncate_title(&s).encode_utf16().count(), MAX_TITLE_CHARS);
    }

    /// 中文（每字 1 个 UTF-16 码元，但 3 个 UTF-8 字节）：
    /// 若按 UTF-8 字节或按 char 计数都会与 C++ 版不一致。
    #[test]
    fn cjk_counts_as_one_unit_each() {
        let s = "中".repeat(MAX_TITLE_CHARS + 10);
        let t = truncate_title(&s);
        assert_eq!(t.encode_utf16().count(), MAX_TITLE_CHARS);
        assert_eq!(
            t.chars().count(),
            MAX_TITLE_CHARS,
            "BMP 内 char 与 code unit 1:1"
        );
    }

    /// emoji（星形平面，2 个 UTF-16 码元）：截断绝不能切出孤立代理。
    #[test]
    fn emoji_never_splits_surrogate_pair() {
        // 每个 emoji 占 2 个码元 ⇒ 上限 256 能放下 128 个
        let s = "🌍".repeat(200);
        let t = truncate_title(&s);
        let units = t.encode_utf16().count();
        assert!(units <= MAX_TITLE_CHARS, "不能超上限，实得 {units}");
        assert_eq!(units % 2, 0, "每个 emoji 2 码元，截断后必须成对");
        // 关键：结果仍是合法 UTF-8（`&str` 本身保证），且 char 数 = 128
        assert_eq!(t.chars().count(), 128);
    }

    /// 混合宽度：ASCII(1) + emoji(2) 交错，截断点仍需落在 char 边界。
    #[test]
    fn mixed_width_truncation_is_safe() {
        let mut s = String::new();
        for _ in 0..200 {
            s.push('a'); // 1 unit
            s.push('🌍'); // 2 units
        }
        let t = truncate_title(&s);
        assert!(t.encode_utf16().count() <= MAX_TITLE_CHARS);
        assert!(s.starts_with(t), "必须是原串的前缀");
    }
}
