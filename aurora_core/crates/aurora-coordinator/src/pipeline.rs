//! 按键管线 / 模式决策（对齐 Go `pipeline_decider.go` 的*意图*，按 Rust 现状裁剪）。
//!
//! 设计依据：`docs/redesign/key-pipeline.md`。本模块是 S1「模式收编为单点决策」的落点。
//!
//! ## 与 Go 决策器的差异（刻意为之）
//!
//! Go 的 `decider` 持有一组 `Processor` trait 对象，并用 `applyEngineDiff(Capability)`
//! 在切换 processor 时挂卸**共享引擎**的词典层。Rust 现状不需要这套机制：
//!
//! - **Capability 不移植**：Rust 各模式按 schema id 独立查询引擎
//!   （`EngineManager::convert_with(schema_id, ...)`），不存在被多模式改写的共享引擎，
//!   因此没有「引擎副作用」需要单点统一。强行引入 Capability 只是死抽象。
//! - **CommitStrategy 推迟到 S4**：全码/空码上屏逻辑在 Rust coordinator 里尚未实现
//!   （`codetable::engine::should_auto_commit` 为空实现），现在没有调用点消费策略归属。
//!   按「避免过早抽象」原则，等 S4 实现全码上屏时再与真实调用点一起设计。
//!
//! ## 本阶段交付：单一活跃模式 + 单点分派
//!
//! 原先三个散装 bool（`temp_pinyin_mode/quick_input_mode/temp_english_mode`）+ 三处串行
//! `if` 分派 + 分散的激活/复位入口，收敛为单一 [`ModeKind`] 字段 `State.active`：
//!
//! - 结构上保证「同一时刻至多一个独占模式」（三 bool 无法表达这一不变式）。
//! - 键事件分派变为对 `state.active` 的单次 `match`（唯一入口）。
//! - 新增一种模式时，只需加 `ModeKind` 变体 + 一条 `match` 臂，而非再加 bool + 分支。
//!
//! 决策链顺序仍由 `Coordinator::handle_key_event` 承载（对齐 key-pipeline.md §2.1）；
//! 各模式的具体按键处理仍是 `Coordinator` 上与其紧耦合的 `handle_*_key` 方法，
//! 故采用 enum 分派而非 trait 对象（避免把 ~600 行逻辑套进 `Ctx` 间接层，更「boring」）。

/// 当前激活的独占输入模式。`None` 表示普通码表/拼音输入（default processor）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModeKind {
    /// 临时拼音：码表方案下经触发键临时切到拼音反查。
    TempPinyin,
    /// 临时英文：Shift+字母触发的临时英文输入。
    TempEnglish,
    /// 特殊模式：引导键触发，自带码表 + 全码上屏策略。载荷为 `features.special_modes` 下标。
    Special(u8),
    /// 生僻字模式：引导键/热键触发，**用当前活跃方案的编码**输入，候选只留生僻字。
    ///
    /// # 为什么它与 [`Self::Special`] 共用整套按键处理
    ///
    /// special 模式的实质是「overlay 生命周期 + 一个码表引擎 + 候选」。本模式只换其中一项
    /// ——引擎从「overlay 方案自带的那张码表」换成「当前活跃方案」（`overlay_engine_schema`
    /// 的分支），再给候选加一道生僻准入。故按键处理、缓冲、光标、选词、退格全部复用
    /// `handle_special_key` 那一套，**不另写一份**：另写等于把同一条输入流实现两遍，
    /// 两份迟早分叉，而分叉的表现是「生僻字模式里退格/翻页行为和别处不一样」。
    ///
    /// ⚠️ 由此本模式会走到一切 `Special(_)` 的既有路径上。凡是从 `overlay_spec`
    /// （方案 `[overlay]` 段快照）取值的地方，本模式恒为 `None` ⇒ 落到该项的默认档，
    /// 这是**刻意**的：那些是「被叠加使用的那张码表怎么表现」，本模式没有那张码表。
    ///
    /// 无载荷：实例是单例（不像 special 那样一个引导键一份码表），身份不来自任何方案，
    /// 故也没有下标可带——同 [`Self::TempPinyin`] / [`Self::AuxCode`]。
    RareChar,
    /// 反查模式：用户自绑键进入，**用当前活跃方案的编码**查字，缓冲里通配键在任何位置
    /// （含首位）都作通配，不看 `schema.codetable.wildcard` 主开关（spec
    /// `codetable-reverse-mode.md` §3）。
    ///
    /// 与 [`Self::RareChar`] 同理是 special 的参数变体：缓冲 / 光标 / 退格 / 选词 / 退出
    /// 全部复用 `handle_special_key` 与 `special_buffer`，只有候选另由 `handle_reverse`
    /// 构建（`convert_reverse` + 检索范围过滤，不做自动上屏、不吃词频重排与候选调整）。
    /// 与 special 族不同的是它会写 `has_more` / `candidate_limit` / `scope_relaxed`，
    /// 故 `exit_special_mode` 一并复位这三位。
    ///
    /// 无载荷：单例，身份不来自任何方案——同 [`Self::RareChar`]。
    Reverse,
    /// 临时 mix：引导键触发，合并多个成员方案候选。载荷为 `features.mix_modes` 下标。
    Mix(u8),
    /// 辅助码：拼音候选的字形二次筛选。独占输入流（组码中无法同时打拼音），但候选
    /// 列表**只保留命中者**——被滤候选直接丢弃，候选窗只显示匹配词（还原不靠残留
    /// 标记，退出/退格都从会话快照恢复）。
    AuxCode,
}

/// 统一夺取回退登记（对齐 Go decider 的 armRewind/canRewind/rewindHijack）。
///
/// 「夺取式」模式从正常输入流中抢走若干字符进入独占模式（z 抢前导拼音：临拼 / 临英 /
/// 快捷输入 / 生僻字 / 反查）。登记此结构后，在前缀边界退格时撤销夺取、把 `snapshot`
/// 回放回正常码表输入流，而非停留在无候选的独占模式里。
///
/// 只有 z 夺取一个来源，故回放目标恒定是 `State::input_buffer`；实现与判定在
/// [`crate::hijack_rewind`]。
#[derive(Clone, Debug)]
pub struct Rewind {
    /// 夺取前的正常输入流缓冲快照（回放的**唯一**目标）。
    pub snapshot: String,
    /// 夺取瞬间的模式 buffer（「是否退到边界」判定：当前模式 buffer == 此值即在边界）。
    pub host_text: String,
}
