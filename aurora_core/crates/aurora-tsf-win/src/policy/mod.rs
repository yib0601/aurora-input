//! 纯判据：吃键 / 记账 / 上报值的折算规则。
//!
//! 这些模块**不碰任何 Win32**，因此能在本机（Linux）跑 `cargo test`。这不是洁癖——
//! DLL 需要真实 TSF 宿主加载 + COM 激活，而交叉编出来的产物恰恰卡在 COM 激活，
//! 所以纯判据是这套代码唯一能被自动化验证的部分。
//!
//! 每条判据都有一份 C++ 原实现（`aurora_tsf/include/*Policy.h`）。移植时刻意**没有**
//! 让两端共用实现（跨语言 FFI 每次按键过一次边界不值得），而是共用**答案**：
//! `tests/policy_vectors.rs` 读 `cross_platform_policy_vectors.json`，三端同一份。

pub mod caret_geometry;
pub mod caret_source;
pub mod composition;
pub mod composition_placeholder;
pub mod ctrl_alt_cleanup;
pub mod ctrl_space;
pub mod deferred_composition;
pub mod editable_context;
pub mod hold_replay;
pub mod host_click;
pub mod installer_guard;
pub mod key_state;
pub mod mode_status;
pub mod multiline_commit;
pub mod number_key;
pub mod response_type;
pub mod service_ready;
pub mod smart_punct_prev_char;
pub mod space_key;
pub mod toggle_tap;
pub mod ui_element;
pub mod window_title;
