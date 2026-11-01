//! aurora-transform: 文本变换（标点、全角、自动配对、简繁）
//!
//! 与 Go 版本 `aurora_core/internal/transform/` 对齐。

pub mod fullwidth;
pub mod pair_tracker;
pub mod punctuation;
pub mod s2t;
