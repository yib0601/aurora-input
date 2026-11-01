//! 线格式真值表：**同一组帧的期望字节**，四个实现共同消费。
//!
//! # 守的是什么
//!
//! 二进制协议有四个独立实现：Rust 编解码器（`aurora-ipc`）、C++ TSF
//! （`aurora_tsf/src/IPCClient.cpp`）、C++ Linux（`aurora_linux/src/core/Codec.cpp`）、
//! Swift macOS（`aurora_macos/.../BinaryCodec.swift`）。命令码本身已有机器对账
//! （`aurora-tools gen_protocol` 与 `toggle_bits_match_cpp.rs`），但**payload 的字节布局没有**。
//!
//! 两侧手抄一个 `flags:u32 + textLen:u32 + compLen:u32` 头部，字段顺序、有无长度前缀、
//! UTF-16 还是 UTF-8 计数，全靠读对方源码对齐。错了不报错：C++ 读到的 `textLen` 恰好落在
//! `compLen` 上，就上屏一串错字或半截 preedit，日志里一个字都没有。
//!
//! 这正是本仓反复吃亏的形态（同一事实写两处、无编译期约束），所以把**答案**钉进一份 JSON：
//! 每个实现都拿同一组输入算出字节，与表逐字比对。表本身由真源生成，不会「三端一起错到同
//! 一个答案上」——Rust 那份是基准，它先与自己的解码器闭环。
//!
//! # 为什么是「答案」不是「实现」
//!
//! 共享实现要么走 FFI（每次按键跨语言调用，热路径代价离谱），要么把编解码塞进一个
//! 「谁都能编」的库（TSF 是 `/MT` 静态 CRT 的 COM DLL，多引一个运行时就多一份加载失败风险）。
//! 共享答案没有这两个问题，且能覆盖 C++ 直接 `memcpy` 结构体的地方——那是 FFI 也管不到的。
//!
//! # 重录
//!
//! 线格式是**对外契约**，改动它意味着新旧版本不兼容。确实要改时：
//!
//! ```bash
//! AURORA_WIRE_BLESS=1 cargo test -p aurora-ipc --test wire_vectors
//! ```
//!
//! 然后在提交里写明为什么变、旧版 DLL 会怎样。红了默认是回归，不是「表过期了」。

use aurora_ipc::codec::*;
use aurora_ipc::protocol::*;
use std::path::PathBuf;

/// 一个用例：输入 → 期望字节。
struct Case {
    /// 表里的键，也是失败信息里指认用的名字。
    name: &'static str,
    /// 这个用例守的是什么。**写清楚漂了会怎样**，否则下一个人只会把表重录一遍。
    why: &'static str,
    /// 帧命令码（`IpcHeader.command`），失败时方便和日志对。
    cmd: u16,
    /// 谁产出这一帧：`client` = 前端发给服务，`service` = 服务发给前端。
    producer: &'static str,
    /// 用真源编码器算出的字节。
    bytes: Vec<u8>,
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();

    // ---- 上行：前端 → 服务 ----
    //
    // KEY_EVENT 在 Rust 侧只有解码器（服务是收方），所以这里用 `KeyPayload::to_bytes`
    // 组出「Rust 期望收到的字节」——那正是 C++ `SendKeyEvent` 必须产出的东西。
    let key = KeyPayload {
        key_code: 0x41,
        scan_code: 0x1E,
        modifiers: 0x0005,
        event_type: 0,
        toggles: 0x03,
        event_seq: 0x1234,
        prev_char: 0x4E2D,
    };
    let mut key_frame = IpcHeader::new(CMD_KEY_EVENT, KeyPayload::SIZE as u32)
        .to_bytes()
        .to_vec();
    key_frame.extend_from_slice(&key.to_bytes());
    out.push(Case {
        name: "key_event_all_fields_distinct",
        why: "上行最高频帧。C++ 直接 memcpy 结构体上送、Rust 逐字段解码，布局靠人肉对齐；\
              toggles 是位域，漂了不报错只静默失效（智能符号在英文半角下删错字）。\
              每个字段取不同的非零值，防止「顺移一位仍然自洽」这类错位。",
        cmd: CMD_KEY_EVENT,
        producer: "client",
        bytes: key_frame,
    });

    // ---- 下行：服务 → 前端 ----

    out.push(Case {
        name: "commit_text_plain",
        why: "上屏主路径。12 字节头 = flags + textLen + compLen，后跟变长 UTF-8。\
              任一段错位都表现为出字错乱或截断，且两端都不报错。",
        cmd: CMD_COMMIT_TEXT,
        producer: "service",
        bytes: encode_commit_text("hi", None, false, true, false),
    });

    out.push(Case {
        name: "commit_text_with_new_composition",
        why: "带后续组合文本的提交：flags bit1 与 compLen 必须同时正确，否则旧 preedit 残留宿主里。",
        cmd: CMD_COMMIT_TEXT,
        producer: "service",
        bytes: encode_commit_text("hi", Some("hao"), true, true, true),
    });

    out.push(Case {
        name: "commit_text_replacing_held",
        why: "智能符号 press2 专用：flags bit3 置位表示**替换**先前 hold 的组合而非追加。\
              置位丢了会让符号重复上屏（中文逗号变两个）。",
        cmd: CMD_COMMIT_TEXT,
        producer: "service",
        bytes: encode_commit_text_replacing_held(",", true),
    });

    out.push(Case {
        name: "update_composition_ascii",
        why: "预编辑主路径：caretPos(u32) + 余下直到帧末全是 UTF-8，**没有**长度前缀。\
              给它加长度前缀是最容易犯的「对称性错误」。",
        cmd: CMD_UPDATE_COMPOSITION,
        producer: "service",
        bytes: encode_update_composition("ni", 2),
    });

    out.push(Case {
        name: "commit_text_with_cursor",
        why: "上屏后把光标左移：textLen + cursorOffset + 文本。偏移单位是 UTF-16 码元不是字节，\
              与 C++/Swift 的换算约定漂了就跳错位置。",
        cmd: CMD_COMMIT_TEXT_WITH_CURSOR,
        producer: "service",
        bytes: encode_commit_text_with_cursor("hi!", 2),
    });

    out.push(Case {
        name: "move_cursor_count",
        why: "ima.pair 多字符右段要越过不止一格。0 视同 1 的兼容语义靠两端一致——\
              旧 DLL 把它当 direction 恒 1，读的正是这 4 字节。",
        cmd: CMD_MOVE_CURSOR,
        producer: "service",
        bytes: encode_move_cursor(3),
    });

    out.push(Case {
        name: "candidate_rects_two",
        why: "候选命中矩形：count 后每条 20 字节（5×i32）。这是 macOS/Linux 鼠标点选的唯一依据，\
              字段顺序错了就点不中或多选一个。",
        cmd: CMD_CANDIDATE_RECTS,
        producer: "service",
        bytes: encode_candidate_rects(&[(0, 0, 10, 20, 24), (1, 10, 46, 20, 24)]),
    });

    out.push(Case {
        name: "mode_push_all_flags",
        why: "焦点进入时同步模式位：只有 4 字节 flags。三位语义各不相同，漏标点位会让\
              中文标点态误用英文集合（`, . '` 直接透传成半角）。",
        cmd: CMD_MODE_PUSH,
        producer: "service",
        bytes: encode_mode_push(true, true, true),
    });

    out.push(Case {
        name: "mode_status_with_label",
        why: "菜单栏指示器：flags + effectiveMode + labelLen + label(UTF-8)。\
              label 有长度前缀（与 updateComposition 不同），这处不对称正是对账要钉的。",
        cmd: CMD_MODE_STATUS,
        producer: "service",
        bytes: encode_mode_status(0x05, 1, "五笔"),
    });

    out.push(Case {
        name: "ext_envelope",
        why: "扩展信封：kindLen + kind + bodyLen + body，两处长度任一错位都会把 kind 读成乱码。\
              body 是不透明的（JSON 或二进制块），本层不解析。",
        cmd: CMD_EXT,
        producer: "service",
        bytes: encode_ext("notify", b"hi"),
    });

    out
}

fn table_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/wire_vectors.json")
}

fn render_json(cases: &[Case]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(
        "  \"_comment\": \"线格式真值表：同一组帧的期望字节。真源是 aurora-ipc 的编解码器，\
         本文件由 tests/wire_vectors.rs 在 AURORA_WIRE_BLESS=1 时重录，三个前端（TSF/Linux/macOS）\
         消费同一份。改线格式 = 有意变更，重录后必须在提交里说明为什么变。\",\n",
    );
    s.push_str(&format!("  \"protocol_version\": {},\n", PROTOCOL_VERSION));
    s.push_str("  \"cases\": [\n");
    for (i, c) in cases.iter().enumerate() {
        s.push_str("    {\n");
        s.push_str(&format!("      \"name\": \"{}\",\n", c.name));
        s.push_str(&format!("      \"why\": \"{}\",\n", c.why));
        s.push_str(&format!("      \"cmd\": {},\n", c.cmd));
        s.push_str(&format!("      \"producer\": \"{}\",\n", c.producer));
        let hex: String = c.bytes.iter().map(|b| format!("{b:02x}")).collect();
        s.push_str(&format!("      \"hex\": \"{hex}\"\n"));
        s.push_str(if i + 1 == cases.len() {
            "    }\n"
        } else {
            "    },\n"
        });
    }
    s.push_str("  ]\n}\n");
    s
}

/// 解析表里的 `hex` 字段。手写解析而不是加 serde_json 依赖：本 crate 的
/// `serde_json` 是运行时依赖，测试里用它只为读一个字段不值当引入类型层。
fn parse_table(json: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    for line in json.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("\"name\":") {
            name = Some(
                v.trim()
                    .trim_start_matches('"')
                    .trim_end_matches(',')
                    .trim_end_matches('"')
                    .to_string(),
            );
        } else if let Some(v) = t.strip_prefix("\"hex\":") {
            let hex = v
                .trim()
                .trim_end_matches(',')
                .trim_start_matches('"')
                .trim_end_matches('"');
            // `name` 只在该用例有 name 时才有；表结构由 render_json 保证，这里防御一下。
            out.push((name.take().unwrap_or_default(), hex.to_string()));
        }
    }
    out
}

#[test]
fn wire_vectors_match_table() {
    let cases = cases();
    let json = render_json(&cases);

    if std::env::var_os("AURORA_WIRE_BLESS").is_some() {
        std::fs::write(table_path(), &json).expect("写入真值表失败");
        return;
    }

    let on_disk = std::fs::read_to_string(table_path()).unwrap_or_else(|e| {
        panic!(
            "读不到线格式真值表 {}：{e}\n\
             首次生成：AURORA_WIRE_BLESS=1 cargo test -p aurora-ipc --test wire_vectors",
            table_path().display()
        )
    });

    let expected = parse_table(&on_disk);
    assert_eq!(
        expected.len(),
        cases.len(),
        "真值表条目数与用例数不一致（表里 {} 条，代码 {} 条）。\
         新增/删除用例后要重录：AURORA_WIRE_BLESS=1 cargo test -p aurora-ipc --test wire_vectors",
        expected.len(),
        cases.len()
    );

    for (case, (name, hex)) in cases.iter().zip(expected.iter()) {
        assert_eq!(
            case.name, name,
            "真值表条目顺序与代码不一致：表里是 {name}，代码里是 {}",
            case.name
        );
        let want: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("表里的 hex 不是合法十六进制"))
            .collect();
        assert_eq!(
            case.bytes,
            want,
            "用例 `{}` 的字节与真值表不一致。\n\
             为什么这条重要：{}\n\
             实际: {}\n\
             期望: {}\n\
             若是有意改线格式，重录 AURORA_WIRE_BLESS=1 并在提交里说明旧版前端会怎样。",
            case.name,
            case.why,
            hex_of(&case.bytes),
            hex
        );
    }
}

fn hex_of(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 表里的每个字节都能被对应的解码器吃回去。
///
/// 没有这条，表可能描述的是一个**连 Rust 自己都不接受**的格式：单测只证明「编码器输出
/// 等于表」，不证明「那个输出是合法帧」。两个方向都过一遍，表才是可信基准。
#[test]
fn every_vector_round_trips_through_its_decoder() {
    for c in cases() {
        let payload = &c.bytes[IpcHeader::SIZE..];
        match c.name {
            "key_event_all_fields_distinct" => {
                let k = decode_key_payload(payload).expect("KEY_EVENT 解不回来");
                // `KeyPayload` 是 `#[repr(packed)]`（线格式要求），直接取字段引用是
                // 未对齐引用（UB）。先拷进局部变量再比。
                let (key_code, scan_code, modifiers) = (k.key_code, k.scan_code, k.modifiers);
                let (event_type, toggles) = (k.event_type, k.toggles);
                let (event_seq, prev_char) = (k.event_seq, k.prev_char);
                assert_eq!(key_code, 0x41);
                assert_eq!(scan_code, 0x1E);
                assert_eq!(modifiers, 0x0005);
                assert_eq!(event_type, 0);
                assert_eq!(toggles, 0x03);
                assert_eq!(event_seq, 0x1234);
                assert_eq!(prev_char, 0x4E2D);
            }
            "ext_envelope" => {
                let (kind, body) = decode_ext(payload).expect("EXT 解不回来");
                assert_eq!(kind, "notify");
                assert_eq!(body, b"hi");
            }
            // 其余用例对应的是前端侧的解码器（C++/Swift），Rust 这一侧只有编码器；
            // 它们的往返由各前端自己的测试保证，真值表负责让双方看同一组字节。
            _ => {}
        }
    }
}

/// 头部自洽：每条真值表的 `length` 字段等于实际载荷长度。
///
/// C++ 侧读帧时先信 `header.length` 再切载荷。表里某条长度写错，前端会去读帧外的内存
/// 或截断一帧——这类问题在真机上表现为随机丢字，几乎无法从日志反推。
#[test]
fn every_vector_header_length_matches_payload() {
    for c in cases() {
        let h = decode_header(&c.bytes).expect("头部不合法");
        // `IpcHeader` 同 `KeyPayload` 是 `#[repr(packed)]`。
        let (cmd, length) = (h.command, h.length);
        assert_eq!(cmd, c.cmd, "用例 {} 的命令码不符", c.name);
        assert_eq!(
            length as usize,
            c.bytes.len() - IpcHeader::SIZE,
            "用例 {} 的 header.length 与实际载荷长度不符",
            c.name
        );
        assert_eq!(h.major_version(), PROTOCOL_VERSION & VERSION_MASK);
    }
}
