//! 数字类键（主键盘 0-9、小键盘 0-9 与运算符）的吃键判据。
//!
//! 原实现：`aurora_tsf/include/NumberKeyPolicy.h`（C++，28 行）。
//!
//! # 为什么 OnTestKeyDown 与 OnKeyDown 必须调同一个函数
//!
//! Test 放行的数字键，OnKeyDown 也必须放行、不转发服务端。此前 OnKeyDown 在中文模式把
//! Number 一律算输入键转发，靠服务端回 PassThrough 兜着；服务端某条路径一旦出字
//! （论坛 t285 小键盘、主键盘 0、follow_main 运算符先后漏过三次），Chrome 类宿主
//! （Test 不吃仍照调 OnKeyDown）就宿主出一次、我们插一次，PIN 框里一键两个字。
//!
//! # 判据本体不在这里
//!
//! 这条判据是**跨端共用**的：真值表 `cross_platform_policy_vectors.json` 的
//! `number_key_eat` case（8 条向量，2×2×2 全组合）是 TSF / macOS / Linux 三端共同的
//! 答案，`tests/policy_vectors.rs` 直接消费它。本模块只提供 Rust 侧的那个函数。
//!
//! 三端各写各的实现（跨语言 FFI 每次按键过一次边界不值得），但**答案必须一致**——
//! 共享答案、不共享实现。

/// 数字类键该不该吃（转发服务端）。
///
/// - 有输入会话：选词 / 翻页 / 顶屏，吃。
/// - 中文 + 全角、无会话：服务端出全角数字，吃（否则纯 TSF 宿主直接得到半角）。
/// - 其余（中文半角空闲、英文半角）：不吃，宿主自己出字。英文全角另有
///   `english_fullwidth` 分支，不经本函数。
#[inline]
pub fn should_eat_number_key(
    has_input_session: bool,
    chinese_mode: bool,
    full_width: bool,
) -> bool {
    has_input_session || (chinese_mode && full_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真值表的 8 条向量（与 JSON 逐条对应，改动要同步）。
    ///
    /// 这里是一份**冗余副本**，真正把关的是 `tests/policy_vectors.rs`——它读 JSON，
    /// 三端同一份。此处保留是因为它能在没有 JSON 的隔离环境（只检出单个 crate）里
    /// 仍然给出判据的形状，且失败信息更直接。
    #[test]
    fn matches_the_shared_truth_table() {
        let cases = [
            (true, true, true, true, "有会话 + 中文全角：顶码"),
            (true, true, false, true, "有会话 + 中文半角：选词/翻页"),
            (true, false, true, true, "有会话 + 英文全角：会话优先"),
            (true, false, false, true, "有会话 + 英文半角：会话优先"),
            (
                false,
                true,
                true,
                true,
                "无会话 + 中文全角：服务端出全角数字",
            ),
            (false, true, false, false, "无会话 + 中文半角：宿主出半角"),
            (false, false, true, false, "无会话 + 英文全角：走别的分支"),
            (false, false, false, false, "无会话 + 英文半角：宿主出字"),
        ];
        for (sess, cn, fw, want, why) in cases {
            assert_eq!(
                should_eat_number_key(sess, cn, fw),
                want,
                "has_session={sess} chinese={cn} fullwidth={fw}：{why}"
            );
        }
    }
}
