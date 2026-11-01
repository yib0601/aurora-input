//! 中文模式空缓冲空格的吃键判据（论坛 t337）。
//!
//! 原实现：`aurora_tsf/include/SpaceKeyPolicy.h`。
//!
//! 空缓冲空格默认透传，保留宿主对空格键的原生语义（网页滚动等）；全角态吃下出 U+3000。
//! 自定义标点「空格」行配了半角列时也得吃：只在 OnKeyDown 转发不够——Notepad++、WPS 这类
//! 宿主按 OnTestKeyDown 的结论走，不吃就直接收到半角空格；照调 OnKeyDown 的 Chrome 类宿主
//! 才「碰巧正常」。列掩码由 core 推送（`CONFIG_KEY_SPACE_CUSTOM_COLS`）。

/// 自定义标点表的列号，与 core `aurora_punct::punct_col_idx` 逐条对应：
/// 0=中半 1=英全 2=中全 3=英半。
#[inline]
pub fn punct_column(chinese_punct: bool, full_width: bool) -> u32 {
    if chinese_punct {
        if full_width { 2 } else { 0 }
    } else if full_width {
        1
    } else {
        3
    }
}

/// 中文模式、无输入会话时空格该不该吃。
#[inline]
pub fn should_eat_idle_space(chinese_punct: bool, full_width: bool, custom_cols: u8) -> bool {
    full_width || (custom_cols & (1u8 << punct_column(chinese_punct, full_width))) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punct_columns_match_core() {
        assert_eq!(punct_column(true, false), 0, "中半");
        assert_eq!(punct_column(false, true), 1, "英全");
        assert_eq!(punct_column(true, true), 2, "中全");
        assert_eq!(punct_column(false, false), 3, "英半");
    }

    #[test]
    fn fullwidth_always_eats() {
        assert!(
            should_eat_idle_space(true, true, 0),
            "全角必须吃，否则纯 TSF 宿主收到半角空格"
        );
        assert!(should_eat_idle_space(false, true, 0));
    }

    #[test]
    fn halfwidth_without_custom_col_eats_nothing() {
        assert!(!should_eat_idle_space(true, false, 0));
        assert!(!should_eat_idle_space(false, false, 0));
    }

    /// 自定义标点「空格」行配了对应列 ⇒ 吃（Notepad++ / WPS 按 Test 结论走）。
    #[test]
    fn custom_column_makes_it_eat() {
        // 中文半角态 = 列 0
        assert!(should_eat_idle_space(true, false, 1 << 0));
        // 英文半角态 = 列 3
        assert!(should_eat_idle_space(false, false, 1 << 3));
        // 配的是别的列，不吃
        assert!(!should_eat_idle_space(true, false, 1 << 3));
    }
}
