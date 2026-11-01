//! gen_protocol：协议常量**对账器**。
//!
//! 用法：
//!   `gen_protocol --check`   与真源对账，任一处漂移则非零退出（CI / pre-push 用）
//!   `gen_protocol`           同上（无生成行为；保留裸调用是为了让人不必记两个形态）
//!   可选 `--root <仓库根>`   默认自动向上查找
//!
//! # 为什么是对账而不是生成
//!
//! 最初的设计是「从 protocol.rs 生成 C++ / Swift 两侧常量」。放弃它的原因来自对本仓
//! 现状的实测，而不是偏好：
//!
//! | 侧 | 命令码条数 | 带注释的 |
//! |---|---|---|
//! | `aurora_tsf/include/BinaryProtocol.h` | 55 | **53** |
//! | `ProtocolTypes.swift` | 56 | 23 |
//!
//! 那些注释不是「这个常量叫什么」，而是 TSF/macOS 宿主侧才观察得到的知识：`CMD_UIELEMENT_*`
//! 四件套的拉取/推送语义、`CMD_CANDIDATE_PROBE` 为何只上报不判断、`0x021B`/`0x021C` 被 Linux
//! 占位不能撞、`CMD_REFRESH_ICON` 为何刻意不带载荷……**这些在 protocol.rs 里没有**，只存在于
//! 各自的镜像文件里。生成的产物要么覆盖它们（丢知识），要么得把两侧注释一并搬进真源
//! （protocol.rs 变成三份注释的宿主，且 TSF 专有细节污染跨平台真源）。
//!
//! 而对账能拿到生成的全部收益——**漂移不再可能静默通过**——且不动一个字的注释。
//!
//! # 同时校验的还有「合法复用 vs 真撞码」
//!
//! 上下行共用同一个 u16 码位空间，`0x0101` 上行是 `KEY_EVENT`、下行是 `COMMIT_TEXT`，
//! 真源里这样的跨方向复用有 7 组。**跨方向复用合法**（dispatcher 按上下文区分），
//! **同方向重码才是 bug**。本工具从真源的分节注释推断方向后断言后者。这正是历史上
//! `CMD_SYNC_HOTKEYS` 与 `CMD_CARET_UPDATE` 同占 `0x0301` 那个 bug 的形态。
//!
//! # 覆盖范围
//!
//! - `BinaryProtocol.h`：`CMD_*` 常量，必须全部能在真源里找到且值相等
//! - `ExtProtocol.h`（Linux）：同上
//! - `ProtocolTypes.swift`：`UpstreamCmd` / `DownstreamCmd` 内的常量，按名字映射（screaming
//!   snake → lowerCamel）后对账
//!
//! 不校验 payload 结构体 / 编解码实现——那些由各自的 round-trip 测试负责。

use std::collections::BTreeMap;

use std::path::{Path, PathBuf};

const RUST_SRC: &str = "aurora_core/crates/aurora-ipc/src/protocol.rs";
const CPP_HEADER: &str = "aurora_tsf/include/BinaryProtocol.h";
const CPP_EXT: &str = "aurora_linux/include/ExtProtocol.h";
const SWIFT_SRC: &str = "aurora_macos/Sources/WindInputKit/IPC/ProtocolTypes.swift";

/// 真源里的一条命令码。
struct Code {
    name: String,
    value: u16,
    upstream: bool,
}

fn main() {
    let mut root: Option<PathBuf> = None;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            // 现在没有生成行为，--check 与裸调用等价。仍接受它，因为脚本/文档里写的是
            // `--check`，且将来若真加了生成路径，调用点不必改。
            "--check" => {}
            "--root" => {
                i += 1;
                root = args.get(i).map(PathBuf::from);
            }
            "-h" | "--help" => {
                println!("用法: gen_protocol [--check] [--root <仓库根>]");
                return;
            }
            other => {
                eprintln!("未知参数: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let root = match root.or_else(repo_root) {
        Some(p) => p,
        None => {
            eprintln!("找不到仓库根（向上未找到 aurora_core/ 与 aurora_tsf/）。用 --root 指定。");
            std::process::exit(2);
        }
    };

    let mut failures: Vec<String> = Vec::new();

    let src = match read(&root, RUST_SRC) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let codes = match parse_rust(&src) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("解析真源失败: {msg}");
            std::process::exit(2);
        }
    };
    println!(
        "真源 {} 条命令码（上行 {} / 下行 {}）",
        codes.len(),
        codes.iter().filter(|c| c.upstream).count(),
        codes.iter().filter(|c| !c.upstream).count()
    );

    // ── 真源自检：同方向不许重码 ──
    let mut seen: BTreeMap<(bool, u16), &str> = BTreeMap::new();
    for c in &codes {
        if let Some(prev) = seen.insert((c.upstream, c.value), &c.name) {
            let dir = if c.upstream { "上行" } else { "下行" };
            failures.push(format!(
                "真源 {dir} 重码 {:#06x}: {} 与 {} —— 跨方向复用合法，同方向重码是 bug",
                c.value, prev, c.name
            ));
        }
    }

    // ── C++ Windows ──
    check_cpp(&root, CPP_HEADER, "BinaryProtocol.h", &codes, &mut failures);
    // ── C++ Linux 扩展 ──
    check_cpp(&root, CPP_EXT, "ExtProtocol.h", &codes, &mut failures);
    // ── Swift ──
    check_swift(&root, &codes, &mut failures);

    if failures.is_empty() {
        println!("✓ 三侧常量与真源一致，无同方向重码");
    } else {
        eprintln!("\n✗ 发现 {} 处问题:", failures.len());
        for f in &failures {
            eprintln!("  - {f}");
        }
        std::process::exit(1);
    }
}

fn read(root: &Path, rel: &str) -> Result<String, String> {
    std::fs::read_to_string(root.join(rel)).map_err(|e| format!("读不到 {rel}: {e}"))
}

fn repo_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        candidates.extend(exe.ancestors().map(|p| p.to_path_buf()));
    }
    for c in candidates {
        let mut dir = c;
        loop {
            if dir.join("aurora_core").is_dir() && dir.join("aurora_tsf").is_dir() {
                return Some(dir);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    None
}

/// 解析真源：`pub const CMD_*: u16 = 0x....;`，方向按「上游 / 下游」分节注释推断。
///
/// 找不到两个分节注释就**失败**而不是猜——猜错方向的后果正是本工具要消除的那一类
/// （把合法复用报成撞码，或把撞码当成复用放过去）。
fn parse_rust(src: &str) -> Result<Vec<Code>, String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut up = None;
    let mut down = None;
    for (i, l) in lines.iter().enumerate() {
        if l.contains("Command codes") && l.contains("上游") {
            up = Some(i);
        } else if l.contains("Command codes") && l.contains("下游") {
            down = Some(i);
        }
    }
    let up = up.ok_or("真源里找不到 `Command codes — 上游` 分节注释")?;
    let down = down.ok_or("真源里找不到 `Command codes — 下游` 分节注释")?;
    if down <= up {
        return Err("上游分节必须排在下游分节之前".into());
    }

    let mut out = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        if let Some((name, value)) = parse_rust_const(raw.trim()) {
            out.push(Code {
                name,
                value,
                upstream: i > up && i < down,
            });
        }
    }
    if out.is_empty() {
        return Err("一条 CMD_ 常量都没解析到——真源的常量写法变了？".into());
    }
    Ok(out)
}

/// 认 `pub const CMD_NAME: u16 = 0xNNNN;`（等号两侧空格随意，行尾注释随意）。
///
/// **必须 `CMD_` 前缀**：真源里另有 7 条非命令码的 u16——`PROTOCOL_VERSION` / `ASYNC_FLAG` /
/// `VERSION_MASK` 是版本与标志位，`COMMIT_FLAG_*` 是 `CommitResult` 载荷内的位标志
/// （`0x0001` 恰好与 `CMD_ACK` 同值，但分属不同命名空间）。按类型收会把它们卷进来，
/// 然后报一条「`COMMIT_FLAG_MODE_CHANGED` 与 `CMD_ACK` 同方向重码」的假红——假红会让人
/// 把这道闸门当噪音关掉，比没有更糟。
fn parse_rust_const(line: &str) -> Option<(String, u16)> {
    let rest = line.strip_prefix("pub const CMD_")?;
    let (body, rest) = rest.split_once(':')?;
    let body = body.trim();
    if body.is_empty()
        || !body
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    let rest = rest.trim_start().strip_prefix("u16")?;
    let rest = rest.trim_start().strip_prefix('=')?;
    let rest = rest.trim_start().strip_prefix("0x")?;
    let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    if hex.is_empty() || hex.len() > 4 {
        return None;
    }
    let tail = &rest[hex.len()..];
    if !tail.trim_start().starts_with(';') {
        return None;
    }
    let value = u16::from_str_radix(&hex, 16).ok()?;
    Some((format!("CMD_{body}"), value))
}

/// 对账一个 C++ 头。
fn check_cpp(root: &Path, rel: &str, label: &str, codes: &[Code], failures: &mut Vec<String>) {
    let src = match read(root, rel) {
        Ok(s) => s,
        Err(e) => {
            failures.push(e);
            return;
        }
    };
    let found = parse_cpp(&src);
    if found.is_empty() {
        failures.push(format!("{label}: 一条 CMD_ 常量都没解析到——写法变了？"));
        return;
    }
    reconcile(label, &found, codes, failures);
}

/// 抠出 C++ 头里所有的 `constexpr uint16_t CMD_X = 0xNNNN;`。
fn parse_cpp(src: &str) -> BTreeMap<String, u16> {
    let mut out = BTreeMap::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("constexpr uint16_t CMD_") else {
            continue;
        };
        let Some((body, rest)) = rest.split_once('=') else {
            continue;
        };
        let body = body.trim();
        if body.is_empty()
            || !body
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            continue;
        }
        let Ok(Some(v)) = parse_hex_u16(rest.trim()) else {
            continue;
        };
        out.insert(format!("CMD_{body}"), v);
    }
    out
}

/// 解析 `0xNNNN;...` 形态的十六进制。`Ok(None)` = 不是 0x 字面量（跳过），`Err` = 超出 u16。
fn parse_hex_u16(rest: &str) -> Result<Option<u16>, ()> {
    let Some(rest) = rest.strip_prefix("0x") else {
        return Ok(None);
    };
    let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    if hex.is_empty() {
        return Ok(None);
    }
    // 必须紧跟 `;`（允许中间空格），否则不是常量定义行
    if !rest[hex.len()..].trim_start().starts_with(';') {
        return Ok(None);
    }
    u16::from_str_radix(&hex, 16).map(Some).map_err(|_| ())
}

/// Swift 版：**不带 `;`**（Swift 语句无须分号，写了反而少见）。
/// 取到十六进制数字为止，其余（空白 / 行尾注释）忽略。
fn parse_hex_u16_swift(rest: &str) -> Option<u16> {
    let rest = rest.strip_prefix("0x")?;
    let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    if hex.is_empty() {
        return None;
    }
    u16::from_str_radix(&hex, 16).ok()
}

/// `CMD_KEY_EVENT` → `keyEvent`。与 Swift 侧既有的命名法一致。
fn swift_name(rust_name: &str) -> String {
    let base = rust_name.strip_prefix("CMD_").unwrap_or(rust_name);
    let parts: Vec<&str> = base.split('_').filter(|p| !p.is_empty()).collect();
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            out.push_str(&p.to_ascii_lowercase());
        } else {
            let mut cs = p.chars();
            if let Some(f) = cs.next() {
                out.push(f.to_ascii_uppercase());
                out.push_str(&cs.as_str().to_ascii_lowercase());
            }
        }
    }
    out
}

fn check_swift(root: &Path, codes: &[Code], failures: &mut Vec<String>) {
    let src = match read(root, SWIFT_SRC) {
        Ok(s) => s,
        Err(e) => {
            failures.push(e);
            return;
        }
    };
    let found = parse_swift(&src);
    if found.is_empty() {
        failures.push("ProtocolTypes.swift: 一条 UInt16 常量都没解析到——写法变了？".into());
        return;
    }
    let by_swift: BTreeMap<String, String> = codes
        .iter()
        .map(|c| (swift_name(&c.name), c.name.clone()))
        .collect();
    reconcile_swift("ProtocolTypes.swift", &found, codes, &by_swift, failures);
}

/// 抠出 Swift 里 `static let x: UInt16 = 0xNNNN` 形态（含 `public`）。
///
/// **只在 `UpstreamCmd` / `DownstreamCmd` 两个枚举体内扫**：同一个文件里 `WireProtocol`
/// 还有 `version` / `asyncFlag`，它们是帧头字段而非命令码，真源里对应的是
/// `PROTOCOL_VERSION` / `ASYNC_FLAG`（非 `CMD_` 前缀，被 `parse_rust_const` 排除）。
/// 不划这个范围，对账会稳定报两条假红。
fn parse_swift(src: &str) -> BTreeMap<String, u16> {
    let mut out = BTreeMap::new();
    let mut in_cmd_enum = false;
    let mut depth = 0i32;
    for line in src.lines() {
        let line = line.trim();
        if line.starts_with("public enum UpstreamCmd")
            || line.starts_with("public enum DownstreamCmd")
        {
            in_cmd_enum = true;
            depth = 0;
        } else if in_cmd_enum && line == "}" && depth == 0 {
            in_cmd_enum = false;
            continue;
        }
        if !in_cmd_enum {
            continue;
        }
        if line.ends_with('{') {
            depth += 1;
        }
        let Some(rest) = line.strip_prefix("public static let ") else {
            continue;
        };
        let Some((name, rest)) = rest.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let Some(rest) = rest.trim_start().strip_prefix("UInt16") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        if let Some(v) = parse_hex_u16_swift(rest.trim()) {
            out.insert(name.to_string(), v);
        }
    }
    out
}

fn reconcile(
    label: &str,
    found: &BTreeMap<String, u16>,
    codes: &[Code],
    failures: &mut Vec<String>,
) {
    let truth: BTreeMap<&str, u16> = codes.iter().map(|c| (c.name.as_str(), c.value)).collect();
    for (name, v) in found {
        match truth.get(name.as_str()) {
            Some(tv) if tv == v => {}
            Some(tv) => failures.push(format!("{label}: {name} = {v:#06x}，真源是 {tv:#06x}")),
            None => failures.push(format!(
                "{label}: {name} = {v:#06x} 在真源里不存在（真源删了它？还是名字打错了？）"
            )),
        }
    }
    println!("  {label}: {} 条，已逐条对账", found.len());
}

fn reconcile_swift(
    label: &str,
    found: &BTreeMap<String, u16>,
    codes: &[Code],
    by_swift: &BTreeMap<String, String>,
    failures: &mut Vec<String>,
) {
    let truth: BTreeMap<&str, u16> = codes.iter().map(|c| (c.name.as_str(), c.value)).collect();
    for (name, v) in found {
        match by_swift.get(name) {
            Some(rust_name) => {
                let tv = truth[rust_name.as_str()];
                if tv != *v {
                    failures.push(format!(
                        "{label}: {name} = {v:#06x}，真源 {rust_name} = {tv:#06x}"
                    ));
                }
            }
            None => failures.push(format!(
                "{label}: {name} = {v:#06x} 映射不到真源常量\n      \
                 （真源里没有对应的 CMD_{}；或它属于 WireProtocol 这类非命令码，\
                 不该出现在 UpstreamCmd/DownstreamCmd 里）",
                name.to_uppercase()
            )),
        }
    }
    // 反向：真源的跨平台命令码是否都被 Swift 接住了。
    //
    // **只对 Swift 报「缺」不报「多」**：Windows 专有的命令码（host render、UIElement、
    // diag snapshot 等）macOS 本就不该有，反过来 Linux 专用的在 ExtProtocol.h。
    // 故这里只在「Rust 有、Swift 没有」时提示，且不当作失败——真正的判据是
    // aurora_macos/AGENTS.md 的「协议同步铁律」，那条需要人判断谁产出。
    let swift_names: std::collections::BTreeSet<&String> = found.keys().collect();
    let mut missing: Vec<&str> = by_swift
        .iter()
        .filter(|(s, _)| !swift_names.contains(s))
        .map(|(_, r)| r.as_str())
        .collect();
    missing.sort_unstable();
    println!(
        "  {label}: {} 条，已对账（真源另有 {} 条本端未接，属平台差异，不计失败）",
        found.len(),
        missing.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_rust_const() {
        assert_eq!(
            parse_rust_const("pub const CMD_ACK: u16 = 0x0001;"),
            Some(("CMD_ACK".into(), 1))
        );
        assert_eq!(
            parse_rust_const("pub const CMD_X:u16=0x0101; // 行尾注释"),
            Some(("CMD_X".into(), 0x0101))
        );
    }

    #[test]
    fn rejects_other_types_and_shapes() {
        assert_eq!(
            parse_rust_const("pub const STATUS_CHINESE_MODE: u32 = 0x0001;"),
            None
        );
        assert_eq!(
            parse_rust_const("pub const UIELEMENT_MAX_ITEMS: usize = 200;"),
            None
        );
        // 十进制不认：产物/对账只以 0x 为准，避免误解析出「一致」的结论
        assert_eq!(parse_rust_const("pub const CMD_X: u16 = 1;"), None);
        assert_eq!(parse_rust_const("pub const cmd_x: u16 = 0x1;"), None);
    }

    /// 非 `CMD_` 前缀的 u16 必须排除。`COMMIT_FLAG_MODE_CHANGED = 0x0001` 与
    /// `CMD_ACK = 0x0001` 同值但分属不同命名空间（前者是 CommitResult 载荷内的位标志），
    /// 卷进来就会报一条假的重码，把闸门变成噪音。
    #[test]
    fn non_cmd_u16_constants_are_excluded() {
        assert_eq!(
            parse_rust_const("pub const COMMIT_FLAG_MODE_CHANGED: u16 = 0x0001;"),
            None
        );
        assert_eq!(
            parse_rust_const("pub const PROTOCOL_VERSION: u16 = 0x1001;"),
            None
        );
        assert_eq!(
            parse_rust_const("pub const ASYNC_FLAG: u16 = 0x8000;"),
            None
        );
    }

    #[test]
    fn direction_comes_from_section_comments() {
        let src = "\
// Command codes — 上游 (C++ → Go/Rust)
pub const CMD_UP: u16 = 0x0101;
// Command codes — 下游 (Go/Rust → C++ 响应)
pub const CMD_DOWN: u16 = 0x0101;
";
        let codes = parse_rust(src).expect("应能解析");
        assert_eq!(codes.len(), 2);
        assert!(codes[0].upstream);
        assert!(!codes[1].upstream);
    }

    /// 缺分节注释必须报错而不是猜方向：猜错的后果正是本工具要消除的那一类。
    #[test]
    fn missing_sections_is_an_error_not_a_guess() {
        assert!(parse_rust("pub const CMD_X: u16 = 0x1;\n").is_err());
    }

    #[test]
    fn swift_naming_maps_screaming_snake_to_camel() {
        assert_eq!(swift_name("CMD_KEY_EVENT"), "keyEvent");
        assert_eq!(swift_name("CMD_ACK"), "ack");
        assert_eq!(swift_name("CMD_UIELEMENT_PAGE"), "uielementPage");
        assert_eq!(
            swift_name("CMD_CLEAR_THEN_PASS_THROUGH"),
            "clearThenPassThrough"
        );
    }

    #[test]
    fn parse_cpp_finds_constexpr_and_ignores_other_types() {
        let src = "\
constexpr uint16_t CMD_ACK = 0x0001; // 注释
constexpr uint32_t STATUS_X = 0x0001;
constexpr uint16_t HOST_RENDER_FAIL_WINDOW_CREATE = 1;
constexpr uint16_t CMD_BIG = 0x0101;
";
        let m = parse_cpp(src);
        assert_eq!(m.get("CMD_ACK"), Some(&1));
        assert_eq!(m.get("CMD_BIG"), Some(&0x0101));
        assert_eq!(
            m.len(),
            2,
            "只有 CMD_ 前缀的 uint16_t 常量该被收进来: {m:?}"
        );
    }

    #[test]
    fn parse_swift_only_scans_cmd_enums() {
        let src = "\
public enum WireProtocol {
    public static let version: UInt16 = 0x1001
    public static let asyncFlag: UInt16 = 0x8000
}

public enum UpstreamCmd {
    public static let keyEvent: UInt16 = 0x0101
    public static let commitRequest: UInt16 = 0x0104
}

public enum DownstreamCmd {
    public static let ack: UInt16 = 0x0001
}
";
        let m = parse_swift(src);
        assert_eq!(m.get("keyEvent"), Some(&0x0101));
        assert_eq!(m.get("ack"), Some(&1));
        assert!(
            !m.contains_key("version"),
            "WireProtocol 不是命令码，不该被收: {m:?}"
        );
        assert!(
            !m.contains_key("asyncFlag"),
            "WireProtocol 不是命令码，不该被收: {m:?}"
        );
    }

    #[test]
    fn reconcile_catches_value_drift() {
        let codes = vec![Code {
            name: "CMD_ACK".into(),
            value: 0x0001,
            upstream: false,
        }];
        let mut found = BTreeMap::new();
        found.insert("CMD_ACK".to_string(), 0x0002u16);
        let mut fails = Vec::new();
        reconcile("t.h", &found, &codes, &mut fails);
        assert_eq!(fails.len(), 1);
        assert!(fails[0].contains("真源是 0x0001"), "{fails:?}");
    }

    #[test]
    fn reconcile_catches_constant_missing_from_truth() {
        let codes = vec![Code {
            name: "CMD_ACK".into(),
            value: 0x0001,
            upstream: false,
        }];
        let mut found = BTreeMap::new();
        found.insert("CMD_GHOST".to_string(), 0x0009u16);
        let mut fails = Vec::new();
        reconcile("t.h", &found, &codes, &mut fails);
        assert_eq!(fails.len(), 1);
        assert!(fails[0].contains("在真源里不存在"), "{fails:?}");
    }

    #[test]
    fn reconcile_swift_catches_value_drift() {
        let codes = vec![Code {
            name: "CMD_KEY_EVENT".into(),
            value: 0x0101,
            upstream: true,
        }];
        let mut found = BTreeMap::new();
        found.insert("keyEvent".to_string(), 0x0102u16);
        let by_swift: BTreeMap<String, String> = codes
            .iter()
            .map(|c| (swift_name(&c.name), c.name.clone()))
            .collect();
        let mut fails = Vec::new();
        reconcile_swift("t.swift", &found, &codes, &by_swift, &mut fails);
        assert_eq!(fails.len(), 1);
        assert!(fails[0].contains("CMD_KEY_EVENT"), "{fails:?}");
    }
}
