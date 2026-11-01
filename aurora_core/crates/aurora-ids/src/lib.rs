//! 跨层共享的标识符常量。
//!
//! # 这个 crate 存在的理由
//!
//! 有一类**字符串 id** 被两个不相邻的层同时需要：配置层要用它写默认值、做存量迁移，
//! 功能层要用它分派实现。把常量留在功能 crate 里，配置层就得反向依赖功能层——
//! `aurora-config → aurora-quick-input` 就是这样一处倒置：基础层为了 6 个字符串
//! 依赖了一整个候选来源 crate。
//!
//! 这里只放**标识符**：`&str` 常量与由它们拼成的数组。任何函数、类型、trait 都不该进来——
//! 一旦这里开始长逻辑，它就会变成第二个上帝 crate，比原来的倒置更难拆。
//!
//! 真源仍是各自的 crates：改这里的值等于改协议级标识符（存量配置里存的就是这些字符串）。

/// 快捷输入家族（`mix_modes.members` 里的成员 id）。
///
/// 出处 `aurora-quick-input`；配置层的默认值、存量迁移与 `mix_modes` 校验都读这里。
pub mod quick_input {
    /// 旧的合并成员 id。存量配置里出现时展开为 [`LEGACY_EXPANSION`]。
    pub const MEMBER_LEGACY: &str = "quick_input";
    /// 日期 / 年月来源。
    pub const MEMBER_DATE: &str = "quick_input.date";
    /// 计算来源。
    pub const MEMBER_CALC: &str = "quick_input.calc";
    /// 数字 / 金额来源。
    pub const MEMBER_NUMBER: &str = "quick_input.number";
    /// 重复上屏来源（**由协调器实现**：候选取自上屏历史，本层不产出）。
    pub const MEMBER_REPEAT: &str = "quick_input.repeat";
    /// 快捷输入历史来源（**由协调器实现**）。
    ///
    /// 与 [`MEMBER_REPEAT`] 的分工：那个只在空缓冲时给「上一次上屏」；本来源在有缓冲时
    /// 按前缀给历史。**出厂不在任何融合实例里**，由用户自己加进成员列表。
    pub const MEMBER_HISTORY: &str = "quick_input.history";

    /// 旧值 `quick_input` 的展开序，同时是内置「快捷」融合的默认来源序。
    ///
    /// calc 在最前：它与 date 的输入形态互斥（表达式必含二元运算符，日期只有数字与点），
    /// 与 number 则**刻意共存**（`123*4` 先求值再转金额），而计算结果作首选是明确诉求。
    ///
    /// ★ number 与 date **不会同时出候选**：小数点个数就是归属判据（一个点归数字、
    /// 两个点归日期）。故这两者的相对次序对任何一次输入都不再可观测，此处保留原序
    /// 只为不动 calc 与 number 的相对优先级。
    pub const LEGACY_EXPANSION: &[&str] = &[MEMBER_CALC, MEMBER_NUMBER, MEMBER_DATE, MEMBER_REPEAT];

    /// 是否为快捷输入家族的内置成员 id（含 `quick_input.repeat` 与旧值 `quick_input`）。
    ///
    /// 用于把它们从「真实方案成员」中排除——它们没有对应的 `.schema.toml`。
    pub fn is_quick_member(member: &str) -> bool {
        member == MEMBER_LEGACY || member.starts_with("quick_input.")
    }
}

#[cfg(test)]
mod tests {
    /// 常量拼错是静默的（成员 id 匹配不上就只是「开关没生效」），故钉住字面值。
    /// 改这里的值 = 改存量配置的兼容性，必须是显式决定。
    #[test]
    fn member_ids_are_pinned() {
        use super::quick_input::*;
        assert_eq!(MEMBER_LEGACY, "quick_input");
        assert_eq!(MEMBER_DATE, "quick_input.date");
        assert_eq!(MEMBER_CALC, "quick_input.calc");
        assert_eq!(MEMBER_NUMBER, "quick_input.number");
        assert_eq!(MEMBER_REPEAT, "quick_input.repeat");
        assert_eq!(MEMBER_HISTORY, "quick_input.history");
    }

    #[test]
    fn legacy_expansion_order_is_pinned() {
        use super::quick_input::*;
        assert_eq!(
            LEGACY_EXPANSION,
            &[MEMBER_CALC, MEMBER_NUMBER, MEMBER_DATE, MEMBER_REPEAT]
        );
    }

    #[test]
    fn is_quick_member_covers_legacy_and_family() {
        use super::quick_input::is_quick_member;
        assert!(is_quick_member("quick_input"));
        assert!(is_quick_member("quick_input.repeat"));
        assert!(is_quick_member("quick_input.date"));
        assert!(!is_quick_member("wubi86"));
        assert!(!is_quick_member("quick_inputx"));
    }
}
