//! 会话中 Ctrl/Alt 组合的收口判据。
//!
//! 原实现：`aurora_tsf/include/CtrlAltCleanupPolicy.h`。
//!
//! 输入法会话中的 Ctrl/Alt 组合（Ctrl+V 粘贴、Ctrl+Z 撤销…）不属于输入，要收回组合态
//! 再交给宿主。但服务端若先在这次响应里出了字，键已被正当消费，就**不该**再交回宿主
//! ——否则同一键既上屏又执行宿主命令。

use crate::policy::response_type::ResponseType;

/// 这个响应类型是否已经出了字（键被正当消费）。
#[inline]
pub fn response_commits_text(t: ResponseType) -> bool {
    matches!(
        t,
        ResponseType::CommitText
            | ResponseType::InsertTextWithCursor
            | ResponseType::CommitThenDefer
            | ResponseType::ReplaceBackward
    )
}

/// 是否把键交回宿主。
///
/// 三条同时成立才交回：没有候选、不在组字、服务端这次也没出字。
#[inline]
pub fn should_return_to_host(
    has_candidates: bool,
    is_composing: bool,
    committed_by_response: bool,
) -> bool {
    !has_candidates && !is_composing && !committed_by_response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_like_responses_commit_text() {
        for t in [
            ResponseType::CommitText,
            ResponseType::InsertTextWithCursor,
            ResponseType::CommitThenDefer,
            ResponseType::ReplaceBackward,
        ] {
            assert!(response_commits_text(t), "{t:?} 出了字");
        }
    }

    #[test]
    fn other_responses_do_not_commit() {
        for t in [
            ResponseType::PassThrough,
            ResponseType::Consumed,
            ResponseType::ClearComposition,
            ResponseType::UpdateComposition,
        ] {
            assert!(!response_commits_text(t), "{t:?} 没出字");
        }
    }

    #[test]
    fn returns_to_host_only_when_idle_and_nothing_committed() {
        assert!(should_return_to_host(false, false, false));
    }

    /// 三条任一成立都不交回——尤其 `committed_by_response`：
    /// 服务端出了字还交回宿主，就是同一键既上屏又执行宿主命令。
    #[test]
    fn does_not_return_when_busy_or_committed() {
        assert!(!should_return_to_host(true, false, false), "有候选");
        assert!(!should_return_to_host(false, true, false), "还在组字");
        assert!(!should_return_to_host(false, false, true), "服务端已出字");
    }
}
