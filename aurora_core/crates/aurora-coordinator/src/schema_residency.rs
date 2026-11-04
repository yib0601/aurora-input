//! 方案常驻策略（`schema.keep_all_loaded`，设计 `docs/design/memory-footprint.md` §5）。
//!
//! - `true`：启动预热全部可用方案 + 临拼目标 + 英文，常驻不淘汰（此前的唯一行为）。
//! - `false`：启动只建用得着的（[`EngineManager::residency_protected`]：当前方案、临拼目标、
//!   临英开着时的英文；混输成员不保护，混输自建子引擎），其余可用方案在后台**只校验派生缓存**
//!   （[`EngineManager::refresh_schema_cache`]，过期即重建、建完即释放），切过去时现建；
//!   清扫拍子每分钟一次，30 分钟没用的方案引擎摘掉（[`EngineManager::evict_idle`]）。
//!
//! **「用得着」里唯一带条件的是临拼目标**：它的词库缓存已经存在才预热，不存在就不建
//! （见 [`Coordinator::prewarm_resident_schemas`]）；后台校验那一圈同理，只校验盘上本就有
//! 缓存的方案，缺的留到真正切过去那一刻（见 [`Coordinator::refresh_schema_caches_on_start`]）。
//! 合起来的效果：新装用户不再为「可能永不使用的方案」付首启的盘与 CPU。
//!
//! 「用过」的记录打在引擎侧取引擎的公共入口（`EngineManager::ensure_loaded`），本模块只管
//! 何时预热、何时清扫。30 分钟与 1 分钟不进配置（R1：「多久算不常用」程序能定）。
//!
//! 同一个清扫拍子还负责懒建缓存的闲置释放（设计 §7：单字全码表、按词查编码用户层），
//! 那部分不看常驻开关（[`EngineManager::evict_idle_caches`]）。
//!
//! [`EngineManager::evict_idle_caches`]: aurora_engine::EngineManager::evict_idle_caches
//!
//! [`EngineManager::residency_protected`]: aurora_engine::EngineManager::residency_protected
//! [`EngineManager::refresh_schema_cache`]: aurora_engine::EngineManager::refresh_schema_cache
//! [`EngineManager::evict_idle`]: aurora_engine::EngineManager::evict_idle

use std::time::Duration;

use tracing::{debug, info};

use crate::coordinator::Coordinator;

/// 方案引擎（`keep_all_loaded = false` 时）与懒建缓存（恒生效，设计 §7）多久没用算闲置。
#[cfg(feature = "desktop-ui")]
pub(crate) const IDLE_EVICT: Duration = Duration::from_secs(30 * 60);
/// 闲置清扫的拍长。
#[cfg(feature = "desktop-ui")]
pub(crate) const IDLE_SWEEP_TICK: Duration = Duration::from_secs(60);

impl Coordinator {
    /// 启动预热的方案部分（[`Self::prewarm_on_start`] 调；阻塞，只在后台线程 / 测试里调）。
    pub(crate) fn prewarm_schemas_on_start(&self) {
        if self.rt().config.schema.keep_all_loaded {
            self.prewarm_all_schemas();
        } else {
            self.prewarm_resident_schemas();
        }
    }

    /// `keep_all_loaded = true`：建好 `available` 全部方案 + 临拼 / 临英目标的引擎。
    pub(crate) fn prewarm_all_schemas(&self) {
        let active = self.engine_mgr.active_schema_id();
        // available_schemas 只含「可切换的方案」。临时拼音 / 临时英文的目标引擎
        // **不在其中**（它们是模式的实现，不是可切换方案），此前因此漏出预热范围：
        // 实测首次按引导键进临拼时才同步加载 52 万词条的拼音库 + 英文库，用户感到
        // 顿一下。两者都只在启用时才预热，不给没开这些功能的用户白付内存。
        let mut targets: Vec<String> = self.engine_mgr.available_schemas().to_vec();
        // ⚠ 这里**不用** `temp_pinyin_target()`：它现在对「缓存不在」改成「后台准备 + 本次
        // 返回 None」（R5，为了让按键线程不被 3.8 s 的拼音首建卡住），而本函数**本身就在
        // 后台预热线程上**、语义就是「现在把全部方案建好」——借它那套门卫会把临拼目标漏掉
        // （`keep_all_loaded = true` 的用户切换前就要求它已就绪）。
        // 故走同一套门卫的**只取 id** 版本 + 就地阻塞建：判据仍只在
        // `resolve_temp_pinyin_target` 一处，不复制「开关 + 方案适用性 + 目标解析」。
        if let Some(t) = self.engine_mgr.temp_pinyin_target_id()
            && self.engine_mgr.prewarm_schema(&t)
        {
            targets.push(t);
        }
        if self.rt().config.input.temp_english.show_candidates {
            targets.push("english".to_string());
        }
        for id in targets {
            // 逐个复核：预热途中用户把常驻关掉了，就别再往里建（已建的交给淘汰）。
            if !self.rt().config.schema.keep_all_loaded {
                debug!("常驻已关，停止预热");
                break;
            }
            if id == active || self.engine_mgr.is_loaded(&id) {
                continue;
            }
            let t0 = std::time::Instant::now();
            if self.engine_mgr.prewarm_schema(&id) {
                debug!("Prewarmed schema {} in {:?}", id, t0.elapsed());
            } else {
                debug!("Prewarm skipped/failed for schema {}", id);
            }
        }
        debug!("Schema prewarm done");
    }

    /// `keep_all_loaded = false`：只建常驻集合（当前方案已在构造时建好）。
    fn prewarm_resident_schemas(&self) {
        // ★ 临时拼音目标**只在词库缓存已经存在时**才预热，缓存不在就**不建**。
        //
        // 为什么（行为改变，2026-11）：
        // - 临时拼音是**码表用户的旁路功能**，出厂默认方案是五笔，绝大多数用户一辈子不按
        //   引导键；而此前这里无条件走 `temp_pinyin_target()`（它的语义是「可用才返回」，
        //   自身就会 ensure_loaded），于是每个新装用户都要替一个可能永不使用的模式编译并
        //   落盘一份拼音词库：实测 base 31.2 MB + ext 27.8 MB + merged 82 MB ≈ 141 MB 盘、
        //   3.8 s CPU、峰值内存 ~341 MB（runtime 审计 E3/E4/E10）。
        // - 现在：缓存已在 ⇒ 预热只是 mmap（毫秒级），**「首次按引导键不卡」照旧成立**；
        //   缓存不在 ⇒ 什么都不做，等用户真的按引导键时按**同一条既有路径**同步建一次
        //   ——与今天首次按 backtick 的路径逐字一致，只是不再提前替所有人付。
        // - **代价**：装完第一次按 backtick 会等一次词库编译（实测约 3.8 s，pinyin-cold
        //   口径；键盘上的表现是那次按键仍被 DeferredHandler 直通给宿主，不吞键）。
        // - **为什么不做成配置开关**：该不该建取决于「用户以后会不会按反引号」，程序拿不准、
        //   用户也答不上来（没人会去配置页勾「我以后可能用临时拼音」）。探测式按需让实际
        //   行为自己回答这个问题；多一个开关只是多一份「设了不生效」的排查面。
        // - **探测不能写成 `temp_pinyin_target()`**：那个门卫自己会 ensure_loaded，探一下
        //   等于先把词库建了（探了白探）。用 `temp_pinyin_cache_present()`——只读判据，
        //   不构建、不拿构建锁，判不出时一律按「缓存不存在」处理（退回按需，安全方向）。
        if self.engine_mgr.temp_pinyin_cache_present() {
            // 与 `prewarm_all_schemas` 同一个门卫（它自己会 ensure_loaded）。
            let _ = self.engine_mgr.temp_pinyin_target();
        } else {
            debug!("临时拼音目标的词库缓存不存在，本次不预热（首次按引导键时按需构建）");
        }
        if self.rt().config.input.temp_english.show_candidates {
            let _ = self.engine_mgr.prewarm_schema("english");
        }
    }

    /// `keep_all_loaded = false` 时，校验其余可用方案的派生缓存（过期 / 缺失即重建），
    /// 建完即释放引擎（[`Self::prewarm_on_start`] 在反查索引预热之后调；阻塞）。
    ///
    /// 必须在启动后台做，不能留到用户切换那一刻：缓存过期时重建是秒级（雾凇 merged 实测
    /// 4 秒、峰值 491 MB），挪到这里之后切换只剩 mmap + 建外壳。排在反查索引之后：那是
    /// 打字马上要用的，这一步只为以后的切换。构建期峰值是一次性的，校验完整理一次堆落回。
    ///
    /// ★ **只校验「盘上本来就有缓存」的方案，缺的不在这里现建**（2026-11，与
    /// [`Self::prewarm_resident_schemas`] 同一把尺子）。不这么改，「不预热临拼」那条是白做
    /// 的：出厂的 `available` 含 `wubi86_pinyin`（混输，成员 = 五笔 + 拼音），这里一圈
    /// ensure_loaded 就会把拼音 merged 建出来——141 MB 一样落盘，只是换了个线程。
    ///
    /// 两条判据各有明确职责：
    /// - 缓存**在**（含「陈旧」——旧变体还在也算在）⇒ 校验 / 重建。升级包刷新了词库时，
    ///   陈旧缓存正是在这一步被修好的，这条路径必须保留。
    /// - 缓存**不在** ⇒ 不预建。代价与上一条相同：首次真切到这个方案时现建（雾凇拼音
    ///   3.8 s），而省下的是「绝大多数用户根本不切的方案」的那份盘与首启 CPU。
    pub(crate) fn refresh_schema_caches_on_start(&self) {
        if self.rt().config.schema.keep_all_loaded {
            return;
        }
        let t0 = std::time::Instant::now();
        let mut checked = 0usize;
        let mut deferred = 0usize;
        for id in self.engine_mgr.available_schemas() {
            if self.engine_mgr.is_loaded(&id) {
                continue;
            }
            if !self.engine_mgr.schema_cache_present(&id) {
                debug!(
                    "跳过预建 {} 的词库缓存（盘上还没有 ⇒ 等真正切过去时按需建）",
                    id
                );
                deferred += 1;
                continue;
            }
            if self.engine_mgr.refresh_schema_cache(&id) {
                checked += 1;
            } else {
                debug!("缓存校验跳过/失败：{}", id);
            }
        }
        // 校验时混输取走了共享英文引擎；眼下若没有消费者，让缓存放手，英文不再受保护。
        self.engine_mgr.release_shared_english_if_unused();
        crate::heap_trim::release_free_heap();
        info!(
            "方案按需加载：常驻 {:?}，校验 {} 个方案的词库缓存用时 {:?}（{} 个无缓存、留待首次切换）",
            self.engine_mgr.loaded_schemas(),
            checked,
            t0.elapsed(),
            deferred
        );
    }

    /// 清扫拍子的主体：
    ///
    /// - 懒建缓存（单字全码表、按词查编码用户层，设计 §7）闲置 `idle` 以上就释放——**恒生效**，
    ///   与常驻策略无关（[`aurora_engine::EngineManager::evict_idle_caches`]）；
    /// - 常驻关着时再摘掉闲置 `idle` 以上的方案引擎。
    ///
    /// 放掉了东西就整理一次堆（释放只是回到分配器手里）。返回被摘的方案 id。
    pub(crate) fn idle_sweep(&self, idle: Duration) -> Vec<String> {
        let released = self.engine_mgr.evict_idle_caches(idle);
        let evicted = if self.rt().config.schema.keep_all_loaded {
            Vec::new()
        } else {
            self.engine_mgr.evict_idle(idle)
        };
        if !evicted.is_empty() || !released.is_empty() {
            crate::heap_trim::release_free_heap();
        }
        evicted
    }

    /// 生产构造器在 `build` 之后起闲置清扫线程（每 [`IDLE_SWEEP_TICK`] 一拍）。
    ///
    /// 单起一条而不是挂在已有线程上：协调器现有的后台线程要么默认挂起（全屏复查只在工具栏
    /// 显示时醒）、要么属于别的 crate（redb 的空闲回收在 aurora-store，store 还可能不存在）。
    /// 一分钟醒一次的成本可忽略；线程只持 `Weak`，协调器析构后下一拍自行退出。
    /// 以后的其它闲置清扫（设计 §7）接在 [`Self::idle_sweep`] 上，不再另起线程。
    #[cfg(feature = "desktop-ui")]
    pub(crate) fn spawn_idle_sweeper(&self) {
        let Some(weak) = self.self_weak.get().cloned() else {
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("idle-sweep".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(IDLE_SWEEP_TICK);
                    let Some(c) = weak.upgrade() else {
                        break;
                    };
                    c.idle_sweep(IDLE_EVICT);
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("无法启动闲置清扫线程: {e}（方案引擎不会自动卸载）");
        }
    }

    /// 配置重载时 `schema.keep_all_loaded` 变了：立即按新策略对齐，不需要重启。
    ///
    /// - true → false：保护名单外的引擎立即摘掉（`idle = 0`）。
    /// - false → true：后台预热全部（与启动走同一个 [`Self::prewarm_schemas_on_start`]，
    ///   此时它读到的已是新配置）。
    ///
    /// 这个键刻意不进 `engine_reload_needed`：改它不该把全部引擎丢掉重建。
    pub(crate) fn apply_residency_change(&self, was_keep_all: bool) {
        let keep_all = self.rt().config.schema.keep_all_loaded;
        if keep_all == was_keep_all {
            return;
        }
        if keep_all {
            // 宿主声明按需加载（移动端 `set_eager_prewarm(false)`）时，同启动一样不建。
            if !self
                .eager_prewarm
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return;
            }
            let Some(weak) = self.self_weak.get().cloned() else {
                return;
            };
            let _ = std::thread::Builder::new()
                .name("residency-prewarm".into())
                .spawn(move || {
                    if let Some(c) = weak.upgrade() {
                        c.prewarm_schemas_on_start();
                    }
                });
        } else {
            if !self.engine_mgr.evict_idle(Duration::ZERO).is_empty() {
                crate::heap_trim::release_free_heap();
            }
        }
    }
}
