use std::path::PathBuf;

fn main() {
    emit_platform_aliases();
    // 产品版本唯一真源 = WindInput/docs/VERSION（与 aurora_input.exe 资源、安装包一致）,
    // 供「关于」菜单等运行时显示。本 crate 位于 aurora_core/crates/aurora-coordinator,
    // 上溯三级即仓库根 WindInput。缺失时回退 CARGO_PKG_VERSION。
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let version_file = manifest_dir
        .join("..")
        .join("..")
        .join("..")
        .join("docs")
        .join("VERSION");
    let ver = std::fs::read_to_string(&version_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into()));
    println!("cargo:rustc-env=AURORA_APP_VERSION={ver}");
    println!("cargo:rerun-if-changed={}", version_file.display());
}

/// 平台语义别名（三个 crate 的 build.rs 各存一份，改动须同步：aurora-ui / aurora-bridge / aurora-coordinator）。
///
/// - `ext_presenter`：呈现层由**外部宿主进程**承担（macOS `.app` / Linux Fcitx5 addon）——服务进程只
///   光栅化，窗口、菜单、设置入口、按键合成都归宿主。macOS 恒为真；Linux 须开 `linux-host`
///   feature，默认关，使 Linux 开发机上的 `cargo test` 仍走 mock 文本，行为与原先一致。
/// - `mock_text`：文本后端是 mock（等宽近似）。含文本的布局测试据此门控，取代原先的
///   `not(windows), not(target_os = "macos")`——Linux 开 `linux-host` 后文本是真的，数值不再确定。
fn emit_platform_aliases() {
    println!("cargo:rustc-check-cfg=cfg(ext_presenter)");
    println!("cargo:rustc-check-cfg=cfg(mock_text)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let linux_host = os == "linux" && std::env::var_os("CARGO_FEATURE_LINUX_HOST").is_some();
    if os == "macos" || linux_host {
        println!("cargo:rustc-cfg=ext_presenter");
    }
    if os != "windows" && os != "macos" && !linux_host {
        println!("cargo:rustc-cfg=mock_text");
    }
}
