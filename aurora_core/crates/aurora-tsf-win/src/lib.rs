//! Windows TSF DLL 的 Rust 实现。
//!
//! 对位 `aurora_tsf/`（C++17）。两者**不是并存关系**：本 crate 逐模块替换 C++ 版，
//! 每替换一个模块就删掉对应的 C++ 实现，绝不维护两套。
//!
//! # 分层
//!
//! ```text
//! src/
//!   skip_key_table.rs   纯逻辑，无 Win32      ← 本机可测
//!   policy/*.rs         纯判据，无 Win32      ← 本机可测
//!   ipc/*.rs            帧收发（复用 aurora-ipc）
//!   com/*.rs            COM 接口实现          ← cfg(windows)
//! ```
//!
//! 边界只有一条：**不碰 Win32 的模块必须能在 Linux 上 `cargo test`**。这不是洁癖，
//! 是这套代码唯一能被自动化验证的部分——DLL 需要真实 TSF 宿主加载 + COM 激活，
//! 而交叉编出来的产物恰恰卡在 COM 激活（见仓库 AGENTS.md 的 6dbc8595）。
//!
//! # 构建约束
//!
//! 产物必须自包含：目标机可能没装 VC++ 运行库。用
//! `RUSTFLAGS="-C target-feature=+crt-static"` 构建，依赖只剩 `kernel32.dll`。
//! 这与 `aurora_tsf/CMakeLists.txt` 的 `CMAKE_MSVC_RUNTIME_LIBRARY = MultiThreaded`
//! 同一个约束。**不加这个 flag 会引入 `VCRUNTIME140.dll`**，在没装运行库的宿主里
//! 等同「加载失败」。

#![cfg_attr(windows, allow(non_snake_case))]

#[cfg(windows)]
pub mod com;
pub mod ipc;
pub mod passthrough_note;
pub mod policy;
pub mod skip_key_table;

pub use passthrough_note::PassthroughState;
pub use skip_key_table::SkipKeyTable;

/// 编译期断言：本 crate 在非 Windows 上也能编过（只有纯逻辑部分）。
///
/// 这条测试存在的意思是「Linux CI 上这个 crate 不许因为缺 Win32 而整块跳过」——
/// 那种跳过会让纯逻辑测试静默消失，而它们正是唯一能跑的部分。
#[cfg(test)]
mod build_guard {
    #[test]
    fn pure_logic_modules_are_always_compiled() {
        // 只要这个测试能链接，`skip_key_table` 就在场。
        let mut t = crate::SkipKeyTable::new();
        t.push(0x41, false, 0);
        assert_eq!(t.count(), 1);
    }
}

/// Windows 下把四个导出钉住，防止 `cdylib` 链接时因「未使用」被剥掉。
///
/// `#[no_mangle]` 已经让它们可见，但 release + LTO 下链接器仍可能判死；
/// 显式引用一次是最省事的保险（与原 `wind_tsf.def` 的 EXPORTS 同效）。
#[cfg(windows)]
#[allow(dead_code)]
fn _link_exports() {
    // 用 `&` 取函数指针，避免 `as usize` 的 direct-cast 警告
    let keep: [*const (); 4] = [
        com::DllCanUnloadNow as *const (),
        com::DllGetClassObject as *const (),
        com::DllRegisterServer as *const (),
        com::DllUnregisterServer as *const (),
    ];
    std::hint::black_box(keep);
}
