//! 端口定义：跨 crate 的 trait，只有声明没有实现。
//!
//! # 为什么单独成 crate
//!
//! 「消费方定义端口、适配器实现」在进程内是对的；但一旦端口与消费方在**同一个 crate**，
//! 跨 crate 的适配器就被迫反向依赖消费方。`KeyInjector` 原在 `aurora-cmdbar`（消费方），
//! 而 `aurora-keys` 是它的实际适配器——于是 `keys → cmdbar` 这条依赖只是为了拿一个
//! trait 声明，方向却从功能层指回基础层。
//!
//! 把 trait 放这里，消费方与适配器都向外依赖本 crate，谁也不必认识谁。
//!
//! ⚠ 本 crate **只放 trait 与它们的错误契约**：任何默认实现、平台代码、状态都不该进来。

/// 按键模拟：`key.tap` / `key.seq` / `key.hold` / `key.release` / `key.type`。
///
/// 组合串的解析（`"Ctrl+Shift+End"` / `"vk:0x5D"`）由适配器负责：它是平台相关的
/// （Windows 用 `SendInput`，macOS 用 `CGEvent`，Linux 走宿主 `forwardKey`），
/// 端口层只约定「给一个组合串，按下去」这一件事。
pub trait KeyInjector: Send + Sync {
    /// 敲一下（按下 + 抬起）。
    fn tap(&self, combo: &str) -> anyhow::Result<()>;
    /// 依次敲多个组合。
    fn sequence(&self, combos: &[String]) -> anyhow::Result<()>;
    /// 按住直到 [`release`](Self::release)。
    fn hold(&self, combo: &str) -> anyhow::Result<()>;
    /// 松开 [`hold`](Self::hold) 按住的组合。
    fn release(&self, combo: &str) -> anyhow::Result<()>;
    /// 直接打一段文本（不经过组合解析）。
    fn type_text(&self, text: &str) -> anyhow::Result<()>;
}
