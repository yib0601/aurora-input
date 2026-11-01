//! 候选 UI 元素的**纯判据**：什么时候答快照、什么时候补拉、谁算在画候选。
//!
//! 原实现：`aurora_tsf/include/UiElementPolicy.h`（C++，91 行）。
//!
//! # 为什么单独成模块
//!
//! 这个状态机有四个互相咬合的输入（宿主声明 / 读取闩 / 快照空否 / 脏位）×
//! 两种调用来路（取内容 vs 取元信息），而它跑在 TSF 宿主进程里、需要真机才能观察——
//! 真值表单测是唯一能把它钉住的手段。
//!
//! ⚠️ 覆盖边界：这里只有判据，没有取数。快照怎么拉、IPC 超时多久、`UpdateUIElement`
//! 什么时候发，全在 COM 层，本模块一行也覆盖不到。**判据全绿 ≠ 候选喂对了**。
//!
//! 背景与实测证据见 `docs/design/game-compat-tsf-uielement.md` §1.4。

/// 「宿主在画候选」——声明接管**或**实际读走过候选串。
///
/// 两个入参语气不同，合并只发生在这一个函数里：`declared` 是宿主自己说的
/// （`BeginUIElement` 回 `pbShow=FALSE` / `Show(FALSE)`），是事实；
/// `read_candidates` 是「它把候选文本取走了」的推断。上报给服务端时**必须分两位**，
/// 因为推断那一半在 core 侧默认就不收窗（opt-in，compat 写
/// `host_drawn_candidates = true` 才生效）。
///
/// ⚠ `declared` **刻意不含 `ui_less_thread`**（调用方只传 `host_draws`）：UI-less 线程的
/// `BeginUIElement` 按规范必回 `pbShow=FALSE`，`host_draws` 随即置真，这里无须再并一次；
/// 并进来反而会在「UI-less 线程的 Begin 意外回了 TRUE」时改变行为。
///
/// 同一份代码里还有两处口径**更宽**，勿混用：
/// - `CaretQuerySuppressed` = `ui_less_thread || host_draws`；
/// - 服务端 `UiElementStatePayload::host_draws()` = `HOST_DRAWS | UI_LESS_THREAD`。
#[inline]
pub fn host_draws(declared: bool, read_candidates: bool) -> bool {
    declared || read_candidates
}

/// getter 答真快照（`true`）还是答占位（`false`）。
///
/// - 已认定宿主在画 ⇒ 恒答快照。与 `SetSelection` / `Finalize` / `Abort` 的放行判据
///   同源；两边不同源会出现「GetCount 说有 1 条、GetString 给占位『…』、宿主照着选
///   却被 SetSelection 回 E_INVALIDARG」这种自相矛盾。
/// - 否则：快照非空且不脏时答快照，其余答占位。
#[inline]
pub fn use_snapshot(
    declared: bool,
    read_candidates: bool,
    snapshot_empty: bool,
    dirty: bool,
) -> bool {
    if host_draws(declared, read_candidates) {
        return true;
    }
    !snapshot_empty && !dirty
}

/// 取内容 / 取元信息时要不要**同步补拉**。
///
/// 三个条件同时成立：脏位真（有变更未同步）、已读过内容或宿主读走过候选、且**不在**
/// 「宿主在画」的状态下（那种情况由 eager 拉取负责）。
///
/// ⚠ 这里不看 `declared`：宿主声明接管时 snapshot 恒可用（见 [`use_snapshot`]），
/// 补拉由 [`should_refresh_eagerly`] 统一驱动，两条路径分工不同。
#[inline]
pub fn should_refresh_on_get(dirty: bool, content_read: bool, read_candidates: bool) -> bool {
    if !dirty {
        return false;
    }
    content_read || read_candidates
}

/// 要不要**立即**（不等 getter 触发）刷快照。
#[inline]
pub fn should_refresh_eagerly(declared: bool, read_candidates: bool) -> bool {
    host_draws(declared, read_candidates)
}

/// 不变量破了：脏位为真却在「宿主在画」态。
///
/// 这是个**诊断判据**——正常流程下不该出现（宿主接管时 eager 刷新会把脏位清掉）。
/// 出现即说明有一条路径漏了刷新，用它可以定位。
#[inline]
pub fn invariant_broken(dirty: bool, declared: bool, read_candidates: bool) -> bool {
    dirty && host_draws(declared, read_candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 声明或读走过，任一成立即算宿主在画。
    #[test]
    fn host_draws_truth_table() {
        assert!(!host_draws(false, false));
        assert!(host_draws(true, false), "声明接管");
        assert!(host_draws(false, true), "读走了候选串");
        assert!(host_draws(true, true));
    }

    #[test]
    fn host_draws_always_uses_snapshot() {
        // 即使快照空、脏，也答快照——否则宿主拿到占位『…』却去 SetSelection
        for &(e, d) in &[(false, false), (false, true), (true, false), (true, true)] {
            assert!(use_snapshot(true, false, e, d), "declared 恒答快照");
            assert!(use_snapshot(false, true, e, d), "read_candidates 恒答快照");
        }
    }

    /// 宿主没在画时：快照非空且不脏才答真快照。
    #[test]
    fn non_host_drawn_snapshot_rules() {
        assert!(use_snapshot(false, false, false, false), "有数据、不脏");
        assert!(!use_snapshot(false, false, true, false), "快照空 ⇒ 占位");
        assert!(!use_snapshot(false, false, false, true), "脏 ⇒ 占位");
        assert!(!use_snapshot(false, false, true, true));
    }

    /// 不脏就不补拉——脏位是补拉的唯一理由。
    #[test]
    fn refresh_requires_dirty() {
        assert!(!should_refresh_on_get(false, true, true));
        assert!(!should_refresh_on_get(false, false, false));
    }

    /// 脏了但还没人读过内容 ⇒ 不补拉（省一次 IPC）。
    #[test]
    fn refresh_requires_someone_read() {
        assert!(
            !should_refresh_on_get(true, false, false),
            "没人读过，补拉没意义"
        );
        assert!(should_refresh_on_get(true, true, false), "读过内容");
        assert!(should_refresh_on_get(true, false, true), "宿主读过候选");
    }

    /// eager 刷新与 host_draws 同源。
    #[test]
    fn eager_refresh_matches_host_draws() {
        for &(d, rc) in &[(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(should_refresh_eagerly(d, rc), host_draws(d, rc));
        }
    }

    /// 不变量：脏 + 宿主在画 = 破了。这是诊断信号，不是正常状态。
    #[test]
    fn invariant_only_breaks_when_dirty_and_host_drawn() {
        assert!(!invariant_broken(false, true, true), "不脏就没问题");
        assert!(!invariant_broken(true, false, false), "没在画就没问题");
        assert!(invariant_broken(true, true, false), "脏 + 声明接管");
        assert!(invariant_broken(true, false, true), "脏 + 读走过");
    }

    /// 关键自洽性：`use_snapshot` 为真时，不该同时被判「不变量破」——
    /// 否则会出现「答了快照但系统认为状态坏了」的矛盾。
    #[test]
    fn snapshot_and_invariant_do_not_contradict_in_normal_states() {
        // 正常状态：不脏
        for &(hd, rc) in &[(false, false), (true, false), (false, true)] {
            assert!(!invariant_broken(false, hd, rc));
            // 不脏时 use_snapshot 只看快照空否
            let _ = use_snapshot(hd, rc, false, false);
        }
    }
}
