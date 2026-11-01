//! 服务端响应类型。
//!
//! 原实现：`aurora_tsf/include/ResponseType.h`。C++ 侧它是 enum class，值来自
//! `BinaryProtocol.h` 的命令码；Rust 侧直接复用 `aurora-ipc` 的命令码常量，
//! 不再抄第二份数值表。
//!
//! ⚠ 只列**响应语义**需要的那些。完整命令码表在 `aurora-ipc::protocol`。

use aurora_ipc::protocol::*;

/// 服务端响应的语义分类（看行为，不看码值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseType {
    /// 键未被消费，交还宿主。
    PassThrough,
    /// 键已被消费但不出字。
    Consumed,
    /// 上屏一段文本。
    CommitText,
    /// 上屏并把光标回移。
    InsertTextWithCursor,
    /// 上屏后延迟重开组合。
    CommitThenDefer,
    /// 回删若干字符再插入文本。
    ReplaceBackward,
    /// 更新组合文本。
    UpdateComposition,
    /// 清掉组合。
    ClearComposition,
    /// 其它（ACK / 状态推送等）。
    Other,
}

impl ResponseType {
    /// 由命令码得出语义分类。
    pub fn from_command(cmd: u16) -> Self {
        match cmd {
            CMD_PASS_THROUGH => Self::PassThrough,
            CMD_CONSUMED => Self::Consumed,
            CMD_COMMIT_TEXT => Self::CommitText,
            CMD_COMMIT_TEXT_WITH_CURSOR => Self::InsertTextWithCursor,
            CMD_COMMIT_THEN_DEFER => Self::CommitThenDefer,
            CMD_REPLACE_BACKWARD => Self::ReplaceBackward,
            CMD_UPDATE_COMPOSITION => Self::UpdateComposition,
            CMD_CLEAR_COMPOSITION => Self::ClearComposition,
            _ => Self::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_commands() {
        assert_eq!(
            ResponseType::from_command(CMD_PASS_THROUGH),
            ResponseType::PassThrough
        );
        assert_eq!(
            ResponseType::from_command(CMD_CONSUMED),
            ResponseType::Consumed
        );
        assert_eq!(
            ResponseType::from_command(CMD_COMMIT_TEXT),
            ResponseType::CommitText
        );
        assert_eq!(
            ResponseType::from_command(CMD_COMMIT_TEXT_WITH_CURSOR),
            ResponseType::InsertTextWithCursor
        );
        assert_eq!(
            ResponseType::from_command(CMD_REPLACE_BACKWARD),
            ResponseType::ReplaceBackward
        );
    }

    #[test]
    fn unknown_command_is_other() {
        assert_eq!(ResponseType::from_command(0xFFFF), ResponseType::Other);
    }
}
