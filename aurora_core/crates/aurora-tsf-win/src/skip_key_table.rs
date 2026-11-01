//! `SkipKeyTable` —— 我们自己经 `SendInput` 注入的按键登记表。
//!
//! 原实现：`aurora_tsf/include/SkipKeyTable.h`（C++，154 行）。逐条对齐，语义不变。
//!
//! # 为什么单独成类
//!
//! 注入前压入，`OnTestKeyDown` / `OnTestKeyUp` 见到匹配条目即 `pfEaten = FALSE`
//! 直接放行，不让它进 IME 自己的按键逻辑（否则注入的退格/字符会被当成用户真按的
//! 键重新走一遍输入流程，表现为文字重复上屏、配对跳出失效）。
//!
//! ── 压入者分两类，语义不同，靠 `for_key_up` 显式区分 ─────────────────────
//!
//! - **键是我们凭空造的** ⇒ down 与 up 都该绕过 IME，压两条。
//! - **键是用户真按过、我们只是延后交付** ⇒ 只压 down，注入的 keyup 走正常路径。
//!
//! 此前条目是裸数组、两类语义靠「同一时刻表里只有我这一条」的**隐含约定**共存，
//! 而 down / up 两个消费点又共用它。约定一破就出三种故障，都表现为「注入键被当成
//! 用户真按的键走完整 IME 流程」：
//!
//! 1. 表里排着两条同 vk（连按 ← ←）⇒ 注入的 keyup 把**给下一个 down 的那条**
//!    吃掉，那个 down 于是漏进 IME 逻辑。
//! 2. 批量注入压 N 条却要覆盖 2N 个事件 ⇒ 后一半没得放行。
//! 3. 队首残条挡死其后**全部**条目（注入没能回到本 sink：前台窗口已换、宿主此刻
//!    不走 TSF、`SendInput` 被 UIPI 拦）。
//!
//! 三者在真机上都难以稳定复现——② 只在批量长度 >1 时发生，③ 要碰运气制造注入
//! 丢失，且残条会被 `ResetComposingState`（换焦点/失焦/中英切换）整表清掉 ⇒
//! 切个窗就自愈，报障形态永远是「时灵时不灵」。搬到这里、把时钟参数化之后，
//! 三者各是一条几行的单元测试。
//!
//! ⚠ **本类只管表自身的语义，管不了接线**：哪个消费点传 `for_key_up = true`、
//! 哪个压入者压一条还是两条，接反了这里的测试照样全绿。接线共 6 处，列在原
//! `KeyEventSink.h` 的 skip 表段落，只能靠人工核对 + 真机。

/// 容量：批量注入路径每字符占两条，128 够一次注入 64 个字符。
pub const MAX_KEYS: usize = 128;

/// TTL 是保险不是主体：注入的键在同线程消息队列里几 ms 内就回来，500ms 极宽松。
/// 它的职责只是让形态③ 那类残条自愈，而不是替代 `for_key_up` 对 ①② 的根治。
pub const TTL_MS: u64 = 500;

/// 待打日志的过期条目上限。攒不下的部分由 [`SkipKeyTable::expired_total`] 兜住
/// ——真正要紧的信息是「发生了」，vk 明细有几条就够定位。
pub const EXPIRED_LOG_CAP: usize = 8;

/// 表里的一条登记。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Entry {
    pub vk: u16,
    /// `true` 表示这条只拦 keyup（用户真按过的键、我们延后交付）。
    pub for_key_up: bool,
    /// 压入时刻（调用方给的时钟）。
    pub tick: u64,
}

/// 见模块文档。
#[derive(Debug)]
pub struct SkipKeyTable {
    keys: [Entry; MAX_KEYS],
    count: usize,

    expired_log: [Entry; EXPIRED_LOG_CAP],
    expired_log_count: usize,
    expired_total: u32,
    dropped_total: u32,
}

impl Default for SkipKeyTable {
    fn default() -> Self {
        Self {
            // `[T; N]` 只有 N <= 32 才实现 Default，128 要手写。
            keys: [Entry::default(); MAX_KEYS],
            count: 0,
            expired_log: [Entry::default(); EXPIRED_LOG_CAP],
            expired_log_count: 0,
            expired_total: 0,
            dropped_total: 0,
        }
    }
}

impl SkipKeyTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个即将注入的按键。
    ///
    /// 表满时丢弃并计入 [`dropped_total`](Self::dropped_total)——调用方**必须**
    /// 把它打成日志：静默丢弃意味着那个注入键会被当成用户真按的键走完整 IME 流程。
    pub fn push(&mut self, vk: u16, for_key_up: bool, now_ms: u64) {
        self.purge(now_ms);
        if self.count >= MAX_KEYS {
            self.dropped_total += 1;
            return;
        }
        self.keys[self.count] = Entry {
            vk,
            for_key_up,
            tick: now_ms,
        };
        self.count += 1;
    }

    /// 这个到达的按键是不是我们自己注入的？是则消费掉对应条目并返回 `true`。
    pub fn try_consume(&mut self, vk: u16, for_key_up: bool, now_ms: u64) -> bool {
        self.purge(now_ms);
        // **按值扫整表**，不只比队首：一条对不上的残条会把它后面的全部条目挡死，
        // 而残条恰恰是最常见的失效形态（形态③）。表通常 0~2 条，线性扫可忽略。
        for i in 0..self.count {
            if self.keys[i].vk != vk || self.keys[i].for_key_up != for_key_up {
                continue;
            }
            for j in (i + 1)..self.count {
                self.keys[j - 1] = self.keys[j];
            }
            self.count -= 1;
            return true;
        }
        false
    }

    /// 清掉超时未被消费的条目。
    ///
    /// `push` / `try_consume` 内部各调一次即可覆盖全部时机——这张表只在这两个动作
    /// 里变化，不需要定时器。
    pub fn purge(&mut self, now_ms: u64) {
        if self.count == 0 {
            return;
        }
        // 用 `saturating_sub`：时钟回退时 `now_ms - tick` 在 C++ 里靠无符号回绕成
        // 一个巨大值（意外地正确，条目被判过期）；这里显式表达「未来时刻视为 0 龄」，
        // 行为等价但不依赖回绕。
        let mut kept = 0;
        for i in 0..self.count {
            if now_ms.saturating_sub(self.keys[i].tick) <= TTL_MS {
                self.keys[kept] = self.keys[i];
                kept += 1;
                continue;
            }
            self.expired_total += 1;
            if self.expired_log_count < EXPIRED_LOG_CAP {
                self.expired_log[self.expired_log_count] = self.keys[i];
                self.expired_log_count += 1;
            }
        }
        self.count = kept;
    }

    /// 组合状态复位（换焦点/失焦/中英切换）时整表清零。
    ///
    /// 累计计数**不清**：它们是跨会话的诊断信号，清掉就看不出「这台机器上到底发生
    /// 过没有」。
    pub fn clear(&mut self) {
        self.count = 0;
        self.expired_log_count = 0;
    }

    pub fn count(&self) -> usize {
        self.count
    }

    // ── 诊断探针 ──────────────────────────────────────────────────────────

    /// 过期条目累计数。`>0` 即证明形态③ 在这台机器上真实发生过。
    pub fn expired_total(&self) -> u32 {
        self.expired_total
    }

    /// 因表满被丢弃的累计数。
    pub fn dropped_total(&self) -> u32 {
        self.dropped_total
    }

    /// 取走待打日志的过期条目（取完即清）。返回写入 `out` 的条数。
    pub fn drain_expired(&mut self, out: &mut [Entry]) -> usize {
        let n = self.expired_log_count.min(out.len());
        out[..n].copy_from_slice(&self.expired_log[..n]);
        self.expired_log_count = 0;
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK_LEFT: u16 = 0x25;
    const VK_BACK: u16 = 0x08;

    /// 形态①：同一个键连按两次，注入的 keyup 不能把给下一个 down 的那条吃掉。
    ///
    /// 旧实现（只比队首）在这里全红：第二条 down 的登记被第一次 up 消费掉，
    /// 于是那个 down 漏进 IME 逻辑。
    #[test]
    fn same_key_twice_does_not_steal_next_down() {
        let mut t = SkipKeyTable::new();
        // 连按 ← ← ：两条 down 各一，两条 up 各一（键是我们凭空造的，down/up 都压）
        t.push(VK_LEFT, false, 1000);
        t.push(VK_LEFT, true, 1000);
        t.push(VK_LEFT, false, 1001);
        t.push(VK_LEFT, true, 1001);
        assert_eq!(t.count(), 4);

        // 注入回来：down / up / down / up
        assert!(t.try_consume(VK_LEFT, false, 1005));
        assert!(t.try_consume(VK_LEFT, true, 1006));
        assert!(t.try_consume(VK_LEFT, false, 1007));
        assert!(t.try_consume(VK_LEFT, true, 1008));
        assert_eq!(t.count(), 0);
    }

    /// 形态②：批量注入压 N 条要覆盖 2N 个事件。
    ///
    /// 一次注入一整段文本时，每个字符 down+up 各一条；旧实现只压一条，后一半
    /// 事件没得放行。
    #[test]
    fn batch_injection_covers_all_events() {
        let mut t = SkipKeyTable::new();
        // 3 个字符 × (down + up)
        for i in 0..3u16 {
            let vk = 0x41 + i;
            t.push(vk, false, 1000);
            t.push(vk, true, 1000);
        }
        assert_eq!(t.count(), 6);

        for i in 0..3u16 {
            let vk = 0x41 + i;
            assert!(t.try_consume(vk, false, 1010), "char {i} 的 down 没被放行");
            assert!(t.try_consume(vk, true, 1011), "char {i} 的 up 没被放行");
        }
        assert_eq!(t.count(), 0);
    }

    /// 形态③：队首残条不能挡死其后的全部条目。
    ///
    /// 残条来自「注入没回到本 sink」：前台窗口已换、宿主此刻不走 TSF、SendInput
    /// 被 UIPI 拦。旧实现只看队首，这条残条会让**之后所有**注入键都漏进 IME。
    #[test]
    fn stale_head_does_not_block_later_entries() {
        let mut t = SkipKeyTable::new();
        t.push(VK_BACK, false, 1000); // 残条：永远不会被消费
        t.push(VK_LEFT, false, 1001);
        t.push(VK_LEFT, true, 1001);

        // 后压的两条仍能被正常消费
        assert!(t.try_consume(VK_LEFT, false, 1005));
        assert!(t.try_consume(VK_LEFT, true, 1006));
        assert_eq!(t.count(), 1, "只剩那条残条");
    }

    /// TTL 到期后残条自愈，且计入诊断。
    #[test]
    fn ttl_expiry_reclaims_stale_entries() {
        let mut t = SkipKeyTable::new();
        t.push(VK_BACK, false, 1000);
        assert_eq!(t.count(), 1);

        // 恰好 TTL 内仍是活的
        t.purge(TTL_MS + 1000);
        assert_eq!(t.count(), 1);

        // 超过 TTL 被回收
        t.purge(TTL_MS + 1001);
        assert_eq!(t.count(), 0);
        assert_eq!(t.expired_total(), 1);
    }

    /// down / up 两个通道互不干扰：只压了 down 时，up 必须走正常路径。
    #[test]
    fn channels_are_isolated() {
        let mut t = SkipKeyTable::new();
        // 「用户真按过、我们延后交付」：只压 down
        t.push(VK_LEFT, false, 1000);

        assert!(
            !t.try_consume(VK_LEFT, true, 1001),
            "没压 up 就不该拦 up——那个 up 必须经过 TSF 的轻敲判定"
        );
        assert!(t.try_consume(VK_LEFT, false, 1002), "压了的 down 要拦住");
    }

    /// 表满时丢弃要可见（调用方必须打日志）。
    #[test]
    fn overflow_is_visible() {
        let mut t = SkipKeyTable::new();
        for i in 0..(MAX_KEYS as u16 + 10) {
            t.push(i, false, 1000);
        }
        assert_eq!(t.count(), MAX_KEYS);
        assert_eq!(t.dropped_total(), 10, "丢弃数必须可观测");
    }

    /// `clear` 清表但保留跨会话诊断计数。
    #[test]
    fn clear_keeps_diagnostics() {
        let mut t = SkipKeyTable::new();
        t.push(VK_BACK, false, 1000);
        t.purge(TTL_MS + 1001); // 制造一条过期
        t.push(VK_LEFT, false, 2000);

        t.clear();
        assert_eq!(t.count(), 0);
        // 累计计数是跨会话的诊断信号，被 ResetComposingState 清掉就看不出这台机器上
        // 到底发生过没有。
        assert_eq!(t.expired_total(), 1);
    }

    /// `drain_expired` 取完即清，且受 `cap` 限制。
    #[test]
    fn drain_expired_returns_and_clears() {
        let mut t = SkipKeyTable::new();
        for i in 0..3u16 {
            t.push(0x41 + i, false, 1000);
        }
        t.purge(TTL_MS + 1001);
        assert_eq!(t.expired_total(), 3);

        let mut out = [Entry::default(); EXPIRED_LOG_CAP];
        assert_eq!(t.drain_expired(&mut out), 3);
        assert_eq!(out[0].vk, 0x41);
        assert_eq!(out[2].vk, 0x43);

        // 再取一次是空的
        assert_eq!(t.drain_expired(&mut out), 0);
    }

    /// 时钟回退（`now < tick`）不 panic、不当成过期。
    #[test]
    fn clock_going_backwards_is_safe() {
        let mut t = SkipKeyTable::new();
        t.push(VK_LEFT, false, 5000);
        t.purge(1000); // 回退
        assert_eq!(t.count(), 1, "未来时刻的条目视为 0 龄，不算过期");
        assert_eq!(t.expired_total(), 0);
    }
}
