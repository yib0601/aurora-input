//! 日志初始化：写入文件，序号轮转最多保留 5 份。
//!
//! 文件命名：aurora_setting.1.log（最新）… aurora_setting.5.log（最旧）。
//! 每次启动将旧文件后移一位，超出 KEEP 的删除，1 号由 appender 新建。
//!
//! 日志目录优先级：
//!   <本机状态目录>\logs  →  <exe 同级>\logs  →  ./logs
//!
//! 级别：`RUST_LOG` > `aurora_setting.toml` 的 `[log] level` > `info`。`off` = 整个关掉，
//! 连文件都不创建（见 [`init`]）。
//!
//! ⚠ 两处覆盖都只认**纯级别**（`off`/`error`/`warn`/`info`/`debug`/`trace`），不支持
//! `target=level` 细粒度指令——理由见 [`parse_level`]（省掉整套 regex 依赖）。给了指令串
//! 会回落到配置级别，并在日志里留一条 warn，不会静默失效。
//!
//! 本机状态目录由 [`crate::user_dir::local_dir_for_mode`] 决定，与 core 的
//! `Config::local_dir()` 同源：便携部署落 `<exe 目录>\localdata\logs`，
//! 否则 `%LOCALAPPDATA%\AuroraInput[Dev]\logs`（macOS `~/Library/Application Support/…`）。
//!
//! 注：应用以 windows_subsystem="windows" 运行，无控制台，仅写文件。
//! 开发时可用 `RUST_LOG=debug cargo run` 覆盖级别。

use std::path::{Path, PathBuf};
use tracing_subscriber::filter::LevelFilter;

const PREFIX: &str = "aurora_setting";
const KEEP: usize = 5;

/// 初始化日志。
///
/// ⚠️ 调用点必须留在单实例闸门**之后**（`main` 里的 `claim_instance`）：轮转会动首实例
/// 正在写的文件，只有确定自己会活下去的进程才有资格做，理由见 [`rotate`]。
pub fn init() {
    // 级别：`RUST_LOG` > `aurora_setting.toml` 的 `[log] level` > `info`。
    // 与 core 服务同一条优先级链（那边见 `startup_trace::effective_log_level`）。
    //
    // 此处读的是设置程序自己的配置文件而非 core config：本函数跑在 main 最早期，
    // RPC 还没连上。见 `setting_config::LogConfig` 的文档。
    let configured = crate::setting_config::load().config.log.level;
    let env_log = std::env::var("RUST_LOG")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let (level, ignored_env) = effective_level(env_log.as_deref(), &configured);

    // ★ 关闭档：目录不建、不轮转、不开 appender，直接返回。
    //
    // 光把 max_level 设成 OFF 不够——`rotate` + `rolling::never` 会实打实地把
    // `aurora_setting.1.log` 建出来并把历史往后挪。用户选"关闭"要的是"这台机器上
    // 别留痕迹"，一个空文件同样是痕迹，历史文件被挪动更是平白的意外。
    //
    // panic 钩子照装：它在 off 档下确实无处可写，但装着不花钱，而 `RUST_LOG`
    // 覆盖时它必须在场。
    if level == LevelFilter::OFF {
        install_panic_hook();
        return;
    }

    let log_dir = resolve_log_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    rotate(&log_dir);

    let file_name = format!("{PREFIX}.1.log");
    let file_appender = tracing_appender::rolling::never(&log_dir, &file_name);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_writer(file_writer)
        .with_target(true)
        .with_thread_ids(false)
        .with_ansi(false)
        .with_max_level(level)
        .init();

    // guard 必须存活至进程退出，否则 non_blocking 缓冲无法刷盘
    Box::leak(Box::new(guard));

    install_panic_hook();

    // `RUST_LOG` 给了细粒度指令（`target=level`）而本程序只支持按级别覆盖：说明一声，
    // 免得「明明设了却一点没变」被当成日志系统坏了。放在 init 之后——之前没有任何
    // subscriber，这条 warn 会掉进虚空。
    if let Some(raw) = ignored_env {
        tracing::warn!(
            "RUST_LOG={raw} 含细粒度指令（target=level），本程序只认纯级别\
             （off/error/warn/info/debug/trace），已按 {level} 记录"
        );
    }

    tracing::info!(
        "aurora_setting v{} 启动，日志: {}",
        env!("CARGO_PKG_VERSION"),
        log_dir.join(&file_name).display()
    );
}

/// 把 panic 写进日志，而不是随 stderr 一起消失。
///
/// 本程序编译为 `windows_subsystem = "windows"`（无控制台），默认 panic handler 那句
/// "thread 'main' panicked at ..." 写的是 stderr —— 而 stderr 没有去处。表现就是**窗口
/// 直接消失、日志停在最后一条正常记录上**，事后完全无从判断是崩了还是正常退出。
///
/// 保留默认钩子并在其后调用：调试器附着时那条 stderr 仍然有用。
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // payload 常见两种形态：`panic!("字面量")` 给 &str，`format!` 过的给 String。
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(无 panic 消息)".to_string());
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "(位置未知)".to_string());
        tracing::error!(
            "panic 于 {loc}: {msg}\n栈回溯:\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        default(info);
    }));
}

fn resolve_log_dir() -> PathBuf {
    // 目录选择整个交给 `user_dir`：便携部署要把日志一并留在 `<exe>\localdata\logs`，
    // 否则"拔盘走人不留痕"落空 —— 配置跟着便携包走了，日志还散在宿主机的
    // `%LOCALAPPDATA%` 里。内部走 `dirs` 而非直接读 `LOCALAPPDATA`：后者在
    // Windows 之外恒为空，会一路回落到 exe 同级 —— 在 macOS 的 .app 里那就是往
    // bundle 内部写，签名后直接失败、日志静默丢失。
    if let Some(local) = crate::user_dir::local_dir_for_mode(crate::mode::get()) {
        return local.join("logs");
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("logs")))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

/// 启动时将旧日志后移一位，删除超出 KEEP 的最旧文件。
/// 结果：.1.log 不存在（由 appender 新建），.2~.N 是历史记录。
///
/// ⚠️ **只有首实例可以调用**，这由 `main` 里「先过单实例闸门再 `logger::init`」的顺序
/// 保证 —— 别把 `init` 挪到闸门之前。二次实例(深链再次启动)若也 rotate，会把首实例正在
/// 写的 .1.log 改名 —— 首实例的 appender 攥着旧 fd 继续写，于是日志内容与文件名对不上，
/// 连着 5 次深链就能把首实例那份轮出去删掉。排查深链问题时这会精准地把人引向错误结论
/// （「首实例没收到 argv」其实是「首实例的日志被挪走了」）。
fn rotate(log_dir: &Path) {
    let _ = std::fs::remove_file(log_dir.join(format!("{PREFIX}.{KEEP}.log")));
    for i in (1..KEEP).rev() {
        let from = log_dir.join(format!("{PREFIX}.{i}.log"));
        let to = log_dir.join(format!("{PREFIX}.{}.log", i + 1));
        let _ = std::fs::rename(from, to);
    }
}

/// 纯级别串 → [`LevelFilter`]。
///
/// **有意不支持** `target=level` 细粒度指令。那需要 `tracing-subscriber` 的 `env-filter`
/// feature，而它的代价是 matchers + regex-automata + regex-syntax 一整套——本程序除它以外
/// **没有任何 regex 使用者**（`cargo tree -p aurora_setting -i regex` 里那条边只来自
/// env-filter），等于为一处启动期解析把整套正则引擎编进 exe（审计在 release 二进制里实测
/// 这部分符号合计 0.41 MiB）。而模块头承诺的契约本来就是「`RUST_LOG` 覆盖**级别**」
/// （AGENTS.md「日志规范」同此口径）。给了指令串不算静默：回落配置级别 + 留一条 warn，
/// 见 [`effective_level`] 与 [`init`]。
///
/// 不用自己 match 字符串：`tracing_core` 的 `FromStr` 已按大小写不敏感认这六个值
/// （另外还认 `0`..=`5` 与空串，空串 → `ERROR`）。空串在这里到不了——`RUST_LOG` 的空值与
/// 配置空串都已在上层折成 `info`。
fn parse_level(s: &str) -> Option<LevelFilter> {
    s.trim().parse::<LevelFilter>().ok()
}

/// 算出生效级别，并交回被忽略掉的 `RUST_LOG` 原文（只有含细粒度指令时才是 `Some`）。
///
/// 优先级链与 core 服务一致：`RUST_LOG` > 配置 `[log] level` > `info`；配置空串 = 没选过
/// ⇒ `info`（不是"选了空"）。认不出的配置值同样回落到 `info`。
fn effective_level(env: Option<&str>, configured: &str) -> (LevelFilter, Option<String>) {
    let configured = {
        let l = configured.trim();
        if l.is_empty() { "info" } else { l }
    };
    let from_config = || parse_level(configured).unwrap_or(LevelFilter::INFO);
    match env {
        Some(raw) => match parse_level(raw) {
            Some(level) => (level, None),
            // 典型是 `aurora_setting=debug,info` 这类指令串：不支持，回落配置级别并回报原文。
            None => (from_config(), Some(raw.to_string())),
        },
        None => (from_config(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `RUST_LOG` 压过配置；纯级别一律认。
    #[test]
    fn env_level_wins_over_config() {
        assert_eq!(effective_level(Some("debug"), "warn").0, LevelFilter::DEBUG);
        assert_eq!(effective_level(None, "warn").0, LevelFilter::WARN);
        // 大小写不敏感（用户手敲 `RUST_LOG=DEBUG` 很常见）
        assert_eq!(effective_level(Some("DEBUG"), "info").0, LevelFilter::DEBUG);
    }

    /// 细粒度指令不支持：回落配置级别，**并把原文交回去**让 init 留 warn。
    /// 交回 None 就等于静默失效——「设了没用、也没人说为什么」是最难查的一类。
    #[test]
    fn fine_grained_directive_falls_back_to_config_and_is_reported() {
        let (level, ignored) = effective_level(Some("aurora_setting=debug"), "warn");
        assert_eq!(level, LevelFilter::WARN);
        assert_eq!(ignored.as_deref(), Some("aurora_setting=debug"));
        // 混着来的也算指令串（EnvFilter 本来能吃，现在不吃）
        let (level, ignored) = effective_level(Some("debug,ureq=off"), "");
        assert_eq!(level, LevelFilter::INFO, "配置空串 ⇒ info");
        assert!(ignored.is_some());
    }

    /// 空配置 = 没选过 ⇒ `info`；`off` 仍是 `off`（用户显式关日志，不能折成 info）。
    #[test]
    fn empty_config_is_info_and_off_stays_off() {
        assert_eq!(effective_level(None, "").0, LevelFilter::INFO);
        assert_eq!(effective_level(None, "   ").0, LevelFilter::INFO);
        assert_eq!(effective_level(None, "off").0, LevelFilter::OFF);
        assert_eq!(effective_level(Some("OFF"), "info").0, LevelFilter::OFF);
        // 配置里写了认不出的东西：按 info，不 panic、不静默关日志
        assert_eq!(effective_level(None, "verbose").0, LevelFilter::INFO);
    }
}
