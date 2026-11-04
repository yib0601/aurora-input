//! 夺取回退（`pipeline::Rewind`）：夺取式模式退到夺取边界再退格时，撤销夺取并把快照
//! 回放回正常码表输入流，而非停在无候选的独占模式里。
//!
//! # 这里为何只有三个函数
//!
//! 本文件原是 `handle_url.rs` 的一部分——网址模式（连同邮箱 / Unicode 码点模式）已整块
//! 删除，而 `active_hijack_buffer` / `can_rewind` / `rewind_hijack` 三个从来不属于它：
//! 它们是 z 夺取（临拼 / 临英 / 快捷输入 / 生僻字 / 反查）共用的骨架，故留下来另立文件。
//!
//! 新增一个夺取式模式 = 在 `try_z_fallback` 里武装 `state.rewind`（`snapshot` 是夺取前的
//! 来源缓冲、`host_text` 是夺取边界），并在下方两处 `match` 里各接住它一条臂：
//! [`Coordinator::active_hijack_buffer`] 给出「当前模式的缓冲」、[`Coordinator::rewind_hijack`]
//! 负责退出该模式。**两处必须一一对应**——那边认得、这边漏了，就会走
//! `reset_exclusive_modes` 兜底：状态清得掉，但各模式自己的收尾（`committed_segs`、光标、
//! mix 的透镜态）不会跑，回退后留下半清理的残局。

use crate::coordinator::{Coordinator, State};
use crate::pipeline::ModeKind;
use aurora_bridge::handler::KeyAction;
use tracing::debug;

impl Coordinator {
    /// 当前夺取式模式的 buffer（用于回退边界判定）。非夺取式模式返回 None。
    pub(crate) fn active_hijack_buffer<'a>(&self, state: &'a State) -> Option<&'a str> {
        match state.active {
            // z 夺取：仅 `try_z_fallback` 会同时武装 `state.rewind`，故 `can_rewind` 只对
            // 夺取式进入成立（符号/字母首键进入的这些模式 rewind=None，不会误回退）。
            Some(ModeKind::TempPinyin) => Some(&state.temp_pinyin_buffer),
            Some(ModeKind::TempEnglish) => Some(&state.temp_english_buffer),
            Some(ModeKind::Mix(_)) => Some(&state.mix_buffer),
            Some(ModeKind::RareChar) => Some(&state.special_buffer),
            Some(ModeKind::Reverse) => Some(&state.special_buffer),
            _ => None,
        }
    }

    /// 是否可回退：已登记 + 当前模式 buffer 已退回到夺取边界（== 登记时的 host_text）
    /// + 没有分步上屏留下的已转换前缀。
    ///
    /// ★ `committed_text` 非空时缓冲等于边界只是巧合：临拼 / 快捷输入打 `lalal` 选「拉拉」
    /// 分步上屏后缓冲剩 `l`，恰等于残余码，此刻回退会经 `exit_*` 把「拉拉」一并清掉、组合区
    /// 跳回 `z`。判据收在这里而不是「分步上屏时清 `rewind`」：分步出口有键盘 / 次选键 keyup /
    /// 鼠标 / 辅助码转交多条，这里是唯一的汇合点；且段全部退回后（`committed_text` 又空、
    /// 缓冲回到边界）那一帧正是夺取边界本身，回退仍应可用，清掉登记就回不去了。
    pub(crate) fn can_rewind(&self, state: &State) -> bool {
        match (&state.rewind, self.active_hijack_buffer(state)) {
            (Some(rw), Some(buf)) => buf == rw.host_text && state.committed_text.is_empty(),
            _ => false,
        }
    }

    /// 执行夺取回退：撤销夺取，把快照回放到正常码表输入流并重算候选。
    pub(crate) fn rewind_hijack(&self, state: &mut State) -> KeyAction {
        let snapshot = state.rewind.take().map(|r| r.snapshot).unwrap_or_default();
        // 退出当前夺取式模式：z-fallback 的临拼、临英、mix、生僻字、反查。
        // ⚠️ 必须与 `active_hijack_buffer` 枚举的模式**一一对应**（理由见文件头）。
        match state.active {
            Some(ModeKind::TempPinyin) => self.exit_temp_pinyin(state),
            Some(ModeKind::TempEnglish) => self.exit_temp_english(state),
            Some(ModeKind::Mix(_)) => self.exit_mix_mode(state),
            Some(ModeKind::RareChar) => self.exit_special_mode(state),
            Some(ModeKind::Reverse) => self.exit_special_mode(state),
            _ => self.reset_exclusive_modes(state),
        }
        state.input_buffer = snapshot;
        state.input_cursor_pos = state.input_buffer.len(); // 光标落到恢复码末尾
        self.update_candidates(state);
        self.notify_ui_update(state);
        let display = state.preedit.clone();
        debug!(
            "rewind_hijack: restored normal input '{}'",
            state.input_buffer
        );
        KeyAction::UpdateComposition {
            text: display,
            caret_pos: state.input_buffer.chars().count() as u32,
        }
    }
}
