//! 熔断器：服务不可用时别把每个按键都拖进超时。
//!
//! 原实现：`aurora_tsf/src/IPCClient.cpp` 的 `_RecordSuccess` / `_RecordFailure` /
//! `_ShouldAttemptOperation` / `ResetCircuitBreaker`。
//!
//! # 为什么需要它
//!
//! 服务进程挂掉或卡死时，每次按键都要走一遍「连接超时 + 写入 + 读超时」，用户打字会
//! 逐键卡顿数秒。熔断器在连续失败到阈值后停止尝试，退避一段时间再放一次试探。
//!
//! # 三态
//!
//! ```text
//!           连续失败 >= 阈值
//!   Closed ──────────────────► Open
//!     ▲                          │
//!     │ 成功                      │ 冷却期满
//!     │                          ▼
//!     └──────────────────── HalfOpen
//!               失败 → 回 Open
//! ```
//!
//! `HalfOpen` 是**一次性通行证**：放行一次操作去试探服务是否恢复。
//! 成功回到 `Closed`，失败回到 `Open` 并重新计时。
//!
//! # 时钟参数化
//!
//! 所有涉及时间的入口都收 `now_ms: u64`，不在内部调 `GetTickCount64`——
//! 那既是可测性的前提（无需 sleep 就能测冷却期），也让本模块完全脱离 Win32。

/// 连续失败多少次后熔断。
pub const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// 熔断后多久放一次试探（毫秒）。
pub const CIRCUIT_RESET_INTERVAL_MS: u64 = 3000;

/// 熔断器状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// 正常通行。
    Closed,
    /// 熔断中，一律不尝试。
    Open,
    /// 冷却期满，放行一次试探。
    HalfOpen,
}

/// 熔断器。见模块文档。
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    state: CircuitState,
    consecutive_failures: u32,
    last_failure_ms: u64,
    /// 已经为「拉起服务」尝试过几次。熔断复位时清零，让下次有资格再拉。
    ///
    /// 单独存在的理由：服务没起来 ≠ 服务该被无限拉起。拉起是一次性动作，
    /// 与「本次操作要不要试」是两件事。
    service_start_attempted: bool,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            last_failure_ms: 0,
            service_start_attempted: false,
        }
    }

    pub fn state(&self) -> CircuitState {
        self.state
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// 本次是否该尝试操作。
    ///
    /// `Open` 且冷却期满 ⇒ 转 `HalfOpen` 并放行一次（同时清零
    /// `service_start_attempted`，允许重新尝试拉起服务）。
    pub fn should_attempt(&mut self, now_ms: u64) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => true,
            CircuitState::Open => {
                if now_ms.saturating_sub(self.last_failure_ms) >= CIRCUIT_RESET_INTERVAL_MS {
                    self.state = CircuitState::HalfOpen;
                    self.service_start_attempted = false;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// 记一次成功：计数清零，熔断复位。
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.state = CircuitState::Closed;
    }

    /// 记一次失败：累计到阈值即熔断。
    ///
    /// `HalfOpen` 下失败也走这里——`consecutive_failures` 已在阈值之上，于是立即回到 `Open`。
    pub fn record_failure(&mut self, now_ms: u64) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.last_failure_ms = now_ms;

        if self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            self.state = CircuitState::Open;
        }
    }

    /// 手动复位（收到 `SERVICE_READY` 时调）。
    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
        self.state = CircuitState::Closed;
        self.service_start_attempted = false;
    }

    /// 服务是否可用：熔断允许尝试，且（已连接 或 连接成功）。
    ///
    /// 这里只判「要不要往下走」，实际连接动作由调用方做。
    pub fn service_available(&mut self, now_ms: u64, connected: bool) -> bool {
        self.should_attempt(now_ms) && connected
    }

    /// 本次是否该尝试拉起服务（一次性）。
    pub fn should_attempt_service_start(&mut self) -> bool {
        if self.service_start_attempted {
            return false;
        }
        self.service_start_attempted = true;
        true
    }

    /// 拉起动作已完成（无论成败），标记为「本周期试过」。
    pub fn mark_service_start_attempted(&mut self) {
        self.service_start_attempted = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_closed_and_attempts() {
        let mut b = CircuitBreaker::new();
        assert_eq!(b.state(), CircuitState::Closed);
        assert!(b.should_attempt(0));
    }

    /// 连续失败到阈值才熔断，不到就照常尝试。
    #[test]
    fn opens_after_threshold() {
        let mut b = CircuitBreaker::new();
        for i in 0..MAX_CONSECUTIVE_FAILURES - 1 {
            b.record_failure(1000 + i as u64);
            assert_ne!(
                b.state(),
                CircuitState::Open,
                "第 {} 次失败还不该熔断",
                i + 1
            );
        }
        b.record_failure(1000 + MAX_CONSECUTIVE_FAILURES as u64);
        assert_eq!(b.state(), CircuitState::Open);
    }

    #[test]
    fn open_blocks_until_cooldown() {
        let mut b = CircuitBreaker::new();
        for i in 0..MAX_CONSECUTIVE_FAILURES {
            b.record_failure(1000 + i as u64);
        }
        let last = 1000 + MAX_CONSECUTIVE_FAILURES as u64 - 1;

        assert!(!b.should_attempt(last), "冷却期内不尝试");
        assert!(
            !b.should_attempt(last + CIRCUIT_RESET_INTERVAL_MS - 1),
            "差 1ms 也不行"
        );
    }

    /// 冷却期满 ⇒ 转 HalfOpen 并放行一次。
    #[test]
    fn cooldown_elapsed_allows_one_probe() {
        let mut b = CircuitBreaker::new();
        for i in 0..MAX_CONSECUTIVE_FAILURES {
            b.record_failure(1000 + i as u64);
        }
        let last = 1000 + MAX_CONSECUTIVE_FAILURES as u64 - 1;

        assert!(b.should_attempt(last + CIRCUIT_RESET_INTERVAL_MS));
        assert_eq!(b.state(), CircuitState::HalfOpen);
    }

    /// 试探成功 ⇒ 回到 Closed，计数清零。
    #[test]
    fn probe_success_closes() {
        let mut b = CircuitBreaker::new();
        for i in 0..MAX_CONSECUTIVE_FAILURES {
            b.record_failure(1000 + i as u64);
        }
        b.should_attempt(10_000);
        assert_eq!(b.state(), CircuitState::HalfOpen);

        b.record_success();
        assert_eq!(b.state(), CircuitState::Closed);
        assert_eq!(b.consecutive_failures(), 0);
    }

    /// 试探失败 ⇒ 回 Open 并重新计时。
    #[test]
    fn probe_failure_reopens_and_restarts_cooldown() {
        let mut b = CircuitBreaker::new();
        for i in 0..MAX_CONSECUTIVE_FAILURES {
            b.record_failure(1000 + i as u64);
        }
        b.should_attempt(10_000);
        assert_eq!(b.state(), CircuitState::HalfOpen);

        b.record_failure(10_000);
        assert_eq!(b.state(), CircuitState::Open, "试探失败立即回熔断");

        // 冷却期从这次失败重新算
        assert!(!b.should_attempt(10_000));
        assert!(b.should_attempt(10_000 + CIRCUIT_RESET_INTERVAL_MS));
    }

    /// 一次成功就清零计数：熔断只认**连续**失败。
    #[test]
    fn success_resets_the_streak() {
        let mut b = CircuitBreaker::new();
        b.record_failure(1000);
        b.record_failure(1001);
        b.record_success();
        assert_eq!(b.consecutive_failures(), 0);
        // 已清零，再失败两次也不该熔断
        b.record_failure(1002);
        b.record_failure(1003);
        assert_eq!(b.state(), CircuitState::Closed);
    }

    #[test]
    fn manual_reset_closes_immediately() {
        let mut b = CircuitBreaker::new();
        for i in 0..MAX_CONSECUTIVE_FAILURES {
            b.record_failure(1000 + i as u64);
        }
        assert_eq!(b.state(), CircuitState::Open);
        b.reset();
        assert_eq!(b.state(), CircuitState::Closed);
        assert_eq!(b.consecutive_failures(), 0);
        assert!(b.should_attempt(2000));
    }

    /// 拉起服务是一性次动作：同一周期内只试一次。
    #[test]
    fn service_start_is_attempted_once_per_cycle() {
        let mut b = CircuitBreaker::new();
        assert!(b.should_attempt_service_start());
        assert!(!b.should_attempt_service_start(), "本周期已试过");

        // 熔断复位后重新有资格
        b.reset();
        assert!(b.should_attempt_service_start());
    }

    /// 冷却期满转 HalfOpen 时也重新获得拉起资格——
    /// 服务可能在冷却期里被人手动杀掉，恢复时需要再拉一次。
    #[test]
    fn cooldown_regrants_service_start() {
        let mut b = CircuitBreaker::new();
        let _ = b.should_attempt_service_start();
        for i in 0..MAX_CONSECUTIVE_FAILURES {
            b.record_failure(1000 + i as u64);
        }
        let last = 1000 + MAX_CONSECUTIVE_FAILURES as u64 - 1;
        b.should_attempt(last + CIRCUIT_RESET_INTERVAL_MS);
        assert!(b.should_attempt_service_start(), "冷却期满后应能再拉");
    }

    /// 时钟回退不 panic（`saturating_sub`）。
    #[test]
    fn clock_going_backwards_is_safe() {
        let mut b = CircuitBreaker::new();
        for i in 0..MAX_CONSECUTIVE_FAILURES {
            b.record_failure(5000 + i as u64);
        }
        // 时钟回退：elapsed 算作 0，不尝试
        assert!(!b.should_attempt(1000));
    }

    #[test]
    fn service_available_requires_connection() {
        let mut b = CircuitBreaker::new();
        assert!(!b.service_available(0, false), "没连上就不可用");
        assert!(b.service_available(0, true));
    }
}
