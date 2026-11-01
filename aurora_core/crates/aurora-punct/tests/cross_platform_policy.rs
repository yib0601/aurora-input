//! 跨端纯判据的**真值表对账**（真源侧）。
//!
//! `testdata/cross_platform_policy_vectors.json` 是 TSF（C++）、`.app`（Swift）、
//! Linux addon（C++）三端单测的**共同输入**。同一份 JSON 三方各读一遍、各自断言自己那套
//! 实现给出同样的答案——共享的是答案，不是实现（跨语言 FFI 每次按键过一次边界不值得，
//! 这些判据都是几行布尔运算）。
//!
//! # 为什么需要这一层
//!
//! 三端各自都有单测，但**测的是各自的理解**：三条实现同时漂到同一个错误答案上，三方测试
//! 全绿而用户行为已错。源文件里那些「对位 macOS xxx 那边」是注释，不是断言——本仓已经
//! 吃过这个形态的亏（`SmartPunct` 的 prevChar 记账三份实现靠注释维持一致）。
//!
//! # 这一份的角色
//!
//! 多数判据的**最终判定**在 Rust（数字后智能标的半角/全角由本 crate 决定），三端只负责
//! 记账并上报。故本测试钉两件事：
//!
//! 1. 真值表本身没写错（本 crate 的判定与表一致）——不然三端会一起「对齐到一个错误答案」；
//! 2. 表里每条 `id` 都被某个本侧判定认领，防止加了 case 却没人消费。
//!
//! 三端各自的读表测试见：
//! - `aurora_tsf/tests/cross_platform_policy_test.cpp`
//! - `aurora_macos/Tests/WindInputKitTests/CrossPlatformPolicyTests.swift`
//! - `aurora_linux/tests/cross_platform_policy_test.cpp`

use std::path::PathBuf;

use aurora_config::config::PunctConfig;

fn table() -> serde_json::Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/cross_platform_policy_vectors.json");
    let src = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读不到 {}: {e}", p.display()));
    serde_json::from_str(&src).unwrap_or_else(|e| panic!("{} 不是合法 JSON: {e}", p.display()))
}

fn vectors(v: &serde_json::Value, id: &str) -> Vec<serde_json::Value> {
    let cases = v["cases"].as_array().expect("cases 必须是数组");
    let case = cases
        .iter()
        .find(|c| c["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("真值表里没有 case `{id}`"));
    let rows = case["vectors"]
        .as_array()
        .unwrap_or_else(|| panic!("case `{id}` 的 vectors 必须是数组"));
    assert!(!rows.is_empty(), "case `{id}` 的 vectors 空着");
    rows.clone()
}

fn b(v: &serde_json::Value, key: &str) -> bool {
    v[key]
        .as_bool()
        .unwrap_or_else(|| panic!("字段 `{key}` 必须是 bool: {v}"))
}

fn u(v: &serde_json::Value, key: &str) -> u16 {
    let n = v[key]
        .as_u64()
        .unwrap_or_else(|| panic!("字段 `{key}` 必须是整数: {v}"));
    u16::try_from(n).unwrap_or_else(|_| panic!("字段 `{key}` 超出 u16: {n}"))
}

/// 数字后智能标点的**端到端**判定：开了 `smart_after_digit`、标点走默认白名单（`.`/`,`）时，
/// 数字前导降级、非数字前导不降级。三端的记账实现必须与这一列真值一致。
#[test]
fn smart_punct_engine_agrees_with_table() {
    let v = table();
    // 默认 PunctConfig 就是 smart_after_digit=true + smart_list=".,"，正是真值表的语境。
    let punct = PunctConfig::default();
    let mut n = 0;
    for row in vectors(&v, "smart_punct_after_digit") {
        let prev = u(&row, "prev_char");
        let want = b(&row, "is_digit");
        // `.` 在默认白名单里，故「是否降级」==「是否数字后」。
        let got = aurora_punct::is_smart_punct_after_digit(&punct, '.', prev);
        assert_eq!(
            got,
            want,
            "prev_char={prev} ({prev:#06x})：实现得 {got}，真值表要 {want}（{}）",
            row["why"].as_str().unwrap_or("")
        );
        n += 1;
    }
    assert!(n >= 8, "真值表只给了 {n} 条，覆盖不足");
    // 全角数字（U+FF13）不能被当成 ASCII 数字 —— 这是真值表里最容易写错的一条。
    assert!(!aurora_punct::is_smart_punct_after_digit(
        &punct, '.', 0xFF13
    ));
    // 关闭总开关后，任何前导都不降级。
    let off = PunctConfig {
        smart_after_digit: false,
        ..punct
    };
    assert!(!aurora_punct::is_smart_punct_after_digit(&off, '.', 0x30));
}

/// 数字类键吃键判据。本 crate 没有同名函数（该判据住在宿主层），故这里用与
/// `NumberKeyPolicy.h` 逐字相同的表达式，钉住真值表的语义；三端各自读同一份表。
#[test]
fn number_key_eat_matches_table() {
    let v = table();
    let mut n = 0;
    for row in vectors(&v, "number_key_eat") {
        let session = b(&row, "has_input_session");
        let chinese = b(&row, "chinese_mode");
        let full = b(&row, "full_width");
        let want = b(&row, "eat");
        let got = session || (chinese && full);
        assert_eq!(
            got,
            want,
            "session={session} chinese={chinese} full={full}：判据得 {got}，真值表要 {want}（{}）",
            row["why"].as_str().unwrap_or("")
        );
        n += 1;
    }
    assert_eq!(n, 8, "数字键判据应覆盖 2×2×2 全部组合");
}

/// 表里每个 case 都要有本侧的消费者。
///
/// 新增 case 却没人认领，等于那张表只是文档——三端照抄一遍谁也不会发现漏了。
#[test]
fn every_case_has_a_consumer_on_this_side() {
    let v = table();
    let known = ["smart_punct_after_digit", "number_key_eat"];
    for case in v["cases"].as_array().expect("cases 必须是数组") {
        let id = case["id"].as_str().expect("case 必须有 id");
        assert!(
            known.contains(&id),
            "case `{id}` 没有本侧消费者——要么在本测试补一条，要么从表里删掉"
        );
    }
}
