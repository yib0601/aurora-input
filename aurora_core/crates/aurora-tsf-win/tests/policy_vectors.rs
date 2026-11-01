//! 跨端纯判据真值表的 TSF（Rust）侧消费。
//!
//! 读 `aurora_core/crates/aurora-punct/testdata/cross_platform_policy_vectors.json`——
//! **与 TSF(C++)、Linux(C++)、macOS(Swift) 读的是同一份**。三端各写各的实现，答案必须一致。
//!
//! # 为什么单独一个 test 文件
//!
//! 本 crate 的模块单测（`src/policy/*.rs`）测的是「本端自己的理解」；三条实现同时漂到同一个
//! 错误答案上时，三方测试全绿而用户行为已错。这张表把「那边」变成会失败的断言。
//!
//! # 找表的方式
//!
//! `CARGO_MANIFEST_DIR` 往上找 `aurora-punct/testdata/`。找不到就**跳过并打印原因**
//! （与 `aurora-ipc/tests/toggle_bits_match_cpp.rs` 同款取舍：只检出单个 crate 时不制造假红）。

use std::path::PathBuf;

use aurora_tsf_win::policy::number_key::should_eat_number_key;

fn table_path() -> Option<PathBuf> {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..6 {
        let p = dir.join("crates/aurora-punct/testdata/cross_platform_policy_vectors.json");
        if p.exists() {
            return Some(p);
        }
        let p2 = dir.join("aurora-punct/testdata/cross_platform_policy_vectors.json");
        if p2.exists() {
            return Some(p2);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

/// 极简 JSON 取值：本表结构固定（`cases[].vectors[]` 的扁平对象），手写解析足够。
/// 取不到就返回空让调用方跳过并打印原因，**不静默通过**。
fn case_vectors(json: &str, case_id: &str) -> Vec<String> {
    // 定位 "id": "<case_id>" 之后到下一个 "id": 之间的文本，再切出 {...} 对象。
    let needle = format!("\"id\": \"{case_id}\"");
    let Some(start) = json.find(&needle) else {
        return Vec::new();
    };
    let rest = &json[start + needle.len()..];
    let end = rest.find("\"id\": ").unwrap_or(rest.len());
    let seg = &rest[..end];

    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(open) = seg[pos..].find('{') {
        let abs_open = pos + open;
        let Some(close) = seg[abs_open..].find('}') else {
            break;
        };
        out.push(seg[abs_open..abs_open + close + 1].to_string());
        pos = abs_open + close + 1;
    }
    out
}

fn get_bool(obj: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{key}\":");
    let at = obj.find(&needle)?;
    let rest = obj[at + needle.len()..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

#[test]
fn number_key_eat_matches_shared_truth_table() {
    let Some(path) = table_path() else {
        eprintln!("跳过：找不到 cross_platform_policy_vectors.json（只检出单个 crate？）");
        return;
    };
    let json = std::fs::read_to_string(&path).expect("读不到真值表");

    let rows = case_vectors(&json, "number_key_eat");
    assert_eq!(
        rows.len(),
        8,
        "number_key_eat 应有 8 条向量（2×2×2 全组合），实得 {}",
        rows.len()
    );

    for row in &rows {
        let sess = get_bool(row, "has_input_session").unwrap_or_else(|| panic!("缺字段: {row}"));
        let cn = get_bool(row, "chinese_mode").unwrap_or_else(|| panic!("缺字段: {row}"));
        let fw = get_bool(row, "full_width").unwrap_or_else(|| panic!("缺字段: {row}"));
        let want = get_bool(row, "eat").unwrap_or_else(|| panic!("缺字段: {row}"));

        assert_eq!(
            should_eat_number_key(sess, cn, fw),
            want,
            "与跨端真值表不一致：has_session={sess} chinese={cn} fullwidth={fw}\n  {row}"
        );
    }
}

/// 真值表里 `smart_punct_after_digit` 的语义：只有 ASCII 数字才算「数字后」。
///
/// 本 crate 那一侧只负责**产出** prevChar（`policy::smart_punct_prev_char`），最终判定在
/// `aurora-punct`。这条测试核的是「抹掉文档数字之后，那 8 条向量仍能被正确解读」——
/// 即本模块的 `resolve_prev_char` 与该表不矛盾。
#[test]
fn smart_punct_prev_char_is_consistent_with_vectors() {
    use aurora_tsf_win::policy::smart_punct_prev_char::{is_ascii_digit, resolve_prev_char};

    let Some(path) = table_path() else {
        eprintln!("跳过：找不到真值表");
        return;
    };
    let json = std::fs::read_to_string(&path).expect("读不到真值表");
    let rows = case_vectors(&json, "smart_punct_after_digit");
    assert!(!rows.is_empty(), "smart_punct_after_digit 没有向量");

    for row in &rows {
        // 表里带 prev_char（十进制数值）
        let Some(at) = row.find("\"prev_char\":") else {
            continue;
        };
        let rest = row[at + "\"prev_char\":".len()..].trim_start();
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let Ok(prev) = num.parse::<u16>() else {
            continue;
        };

        // 本模块的契约：文档里的 ASCII 数字必须被抹成 0，非数字原样保留。
        let resolved = resolve_prev_char(prev, 0, false);
        if is_ascii_digit(prev) {
            assert_eq!(resolved, 0, "文档 ASCII 数字必须抹掉: prev=0x{prev:02X}");
        } else {
            assert_eq!(resolved, prev, "非数字必须原样上报: prev=0x{prev:02X}");
        }
    }
}
