//! 服务就绪后的重试策略。
//!
//! 原实现：`aurora_tsf/include/ServiceReadyRetryPolicy.h`。
//!
//! # 两套独立的补发
//!
//! 1. `IME_ACTIVATED`（就绪同步）：收到 `SERVICE_READY` 后发不出去时按抖动退避重试。
//! 2. `FOCUS_GAINED`（焦点补发）：与上面**分开计数、分开定时器**。
//!
//! 分开的理由见 [`FocusAction`] 的注释：补发窗口里用户可能已手动切过中英，再发
//! `IME_ACTIVATED` 会按初始模式重置，比工具栏晚出更糟。

/// 最多重试次数。
pub const MAX_RETRIES: u32 = 10;
/// 退避基数（毫秒）。
pub const BASE_DELAY_MS: u32 = 300;
/// 抖动幅度（毫秒）。多个宿主同时重连时错开，避免齐刷刷打服务端。
pub const JITTER_MS: u32 = 200;

/// 还要不要再试。
///
/// `failures` = 已失败次数。第 1 次失败后开始重试，到上限停止。
#[inline]
pub fn should_retry(failures: u32) -> bool {
    (1..=MAX_RETRIES).contains(&failures)
}

/// 本次重试前该等多久（毫秒）。
///
/// 确定性哈希而非随机数：同一 `(seed, failures)` 恒得同一个值，测试才能断言；
/// `seed` 由调用方给（通常是连接序号），使不同宿主不同相。
#[inline]
pub fn retry_delay_ms(seed: u32, failures: u32) -> u32 {
    let mut h = seed.wrapping_mul(2_654_435_761) ^ failures.wrapping_mul(40_503);
    h ^= h >> 15;
    h = h.wrapping_mul(2_246_822_519);
    h ^= h >> 13;
    BASE_DELAY_MS + h % (JITTER_MS + 1)
}

/// 就绪同步该做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 无输入焦点：不弹工具栏，等 `OnSetFocus` 建立服务端状态。
    StopNoFocus,
    /// 重试时发现本连接已由按键 / 焦点路径同步过：再发会重置中英状态。
    StopAlreadySynced,
    /// 重试时本线程不在前台：后台宿主不值得反复阻塞 UI 线程。
    StopBackground,
    /// 组合进行中：推迟，免得清掉服务端输入缓冲（**不计失败次数**）。
    DeferComposition,
    /// 发 `IME_ACTIVATED`。
    Sync,
}

/// 就绪同步的决策。
pub fn decide(
    is_retry: bool,
    has_focus: bool,
    has_thread_focus: bool,
    composing: bool,
    connected: bool,
    state_established: bool,
) -> Action {
    if !has_focus {
        return Action::StopNoFocus;
    }
    if is_retry && connected && state_established {
        return Action::StopAlreadySynced;
    }
    if is_retry && !has_thread_focus {
        return Action::StopBackground;
    }
    if composing {
        return Action::DeferComposition;
    }
    Action::Sync
}

/// 焦点补发该做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusAction {
    /// 已没有待补的焦点（别的路径补上了，或焦点已换）。
    StopRecovered,
    /// 无输入焦点。
    StopNoFocus,
    /// 本线程不在前台：补发会冒充前台焦点。
    StopBackground,
    /// 本连接上已有 `IME_ACTIVATED` / `FOCUS_GAINED` / 按键往来。
    StopAlreadySynced,
    /// 连上（必要时新建）后补发 `focus_gained`。
    Resend,
}

/// 焦点补发的决策。
///
/// 只补 `focus_gained`，**没有**退回全量同步这一档：`IME_ACTIVATED` 会让服务端按初始
/// 模式重置中英 / 全半角 / 标点，而补发窗口里用户可能已手动切过。工具栏晚出比重置用户
/// 状态好接受。
pub fn decide_focus_recovery(
    needs_recovery: bool,
    has_focus: bool,
    has_thread_focus: bool,
    connected: bool,
    state_established: bool,
) -> FocusAction {
    if !needs_recovery {
        return FocusAction::StopRecovered;
    }
    if !has_focus {
        return FocusAction::StopNoFocus;
    }
    if !has_thread_focus {
        return FocusAction::StopBackground;
    }
    if connected && state_established {
        return FocusAction::StopAlreadySynced;
    }
    FocusAction::Resend
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_window() {
        assert!(!should_retry(0), "还没失败，无需重试");
        assert!(should_retry(1));
        assert!(should_retry(MAX_RETRIES));
        assert!(!should_retry(MAX_RETRIES + 1), "到上限停止");
    }

    /// 延迟必须落在 [BASE, BASE+JITTER]，且确定性（同输入同输出）。
    #[test]
    fn delay_is_bounded_and_deterministic() {
        for seed in 0..20u32 {
            for failures in 1..=MAX_RETRIES {
                let d = retry_delay_ms(seed, failures);
                assert!(
                    (BASE_DELAY_MS..=BASE_DELAY_MS + JITTER_MS).contains(&d),
                    "seed={seed} failures={failures} 得 {d}"
                );
                assert_eq!(d, retry_delay_ms(seed, failures), "必须确定性");
            }
        }
    }

    /// 不同 seed 应产生不同相位（否则多宿主同时重连会齐刷刷）。
    #[test]
    fn different_seeds_spread_out() {
        let vals: std::collections::HashSet<u32> =
            (0..20u32).map(|s| retry_delay_ms(s, 3)).collect();
        assert!(
            vals.len() > 5,
            "抖动应产生足够的相位分散，实得 {} 种",
            vals.len()
        );
    }

    #[test]
    fn no_focus_stops_immediately() {
        assert_eq!(
            decide(false, false, true, false, true, true),
            Action::StopNoFocus
        );
        assert_eq!(
            decide(true, false, true, false, true, true),
            Action::StopNoFocus
        );
    }

    /// 重试时本连接已同步过 ⇒ 停。再发 IME_ACTIVATED 会重置中英。
    #[test]
    fn already_synced_stops_retry() {
        assert_eq!(
            decide(true, true, true, false, true, true),
            Action::StopAlreadySynced
        );
    }

    /// 首次（非重试）即使已建立状态也要发——那是正常的握手。
    #[test]
    fn first_attempt_is_not_treated_as_already_synced() {
        assert_eq!(decide(false, true, true, false, true, true), Action::Sync);
    }

    #[test]
    fn background_stops_retry() {
        assert_eq!(
            decide(true, true, false, false, true, false),
            Action::StopBackground
        );
        // 首次不在前台也照发（前台判定只约束重试）
        assert_eq!(decide(false, true, false, false, true, false), Action::Sync);
    }

    /// 组字中推迟，且该分支优先于 Sync。
    #[test]
    fn composing_defers() {
        assert_eq!(
            decide(false, true, true, true, true, false),
            Action::DeferComposition
        );
    }

    // ── 焦点补发 ──

    #[test]
    fn focus_no_longer_needed() {
        assert_eq!(
            decide_focus_recovery(false, true, true, false, false),
            FocusAction::StopRecovered
        );
    }

    #[test]
    fn focus_without_focus_stops() {
        assert_eq!(
            decide_focus_recovery(true, false, true, false, false),
            FocusAction::StopNoFocus
        );
    }

    #[test]
    fn focus_in_background_stops() {
        assert_eq!(
            decide_focus_recovery(true, true, false, false, false),
            FocusAction::StopBackground
        );
    }

    /// 本连接已有往来 ⇒ 停，不重复补发。
    #[test]
    fn focus_already_synced_stops() {
        assert_eq!(
            decide_focus_recovery(true, true, true, true, true),
            FocusAction::StopAlreadySynced
        );
    }

    /// 仍欠着、在前台、本连接还没往来 ⇒ 补发。
    #[test]
    fn focus_resends_when_genuinely_needed() {
        assert_eq!(
            decide_focus_recovery(true, true, true, true, false),
            FocusAction::Resend
        );
        assert_eq!(
            decide_focus_recovery(true, true, true, false, false),
            FocusAction::Resend
        );
    }
}
