// 与 C++ 原实现逐条比对的等价性测试（oracle 见 /tmp/skipverify/oracle.cpp 的输出）
use aurora_tsf_win::SkipKeyTable;

#[test]
fn equivalence_with_cpp_impl() {
    // A：残条 + 批量注入混合
    {
        let mut t = SkipKeyTable::new();
        t.push(0x08, false, 1000);
        t.push(0x41, false, 1001);
        t.push(0x41, true, 1001);
        t.push(0x42, false, 1002);
        t.push(0x42, true, 1002);
        assert_eq!(t.count(), 5, "A count");
        // C++: A c1=1 c2=1 c3=1 c4=1
        assert!(t.try_consume(0x41, false, 1005), "A c1");
        assert!(t.try_consume(0x41, true, 1005), "A c2");
        assert!(t.try_consume(0x42, false, 1006), "A c3");
        assert!(t.try_consume(0x42, true, 1006), "A c4");
        assert_eq!(t.count(), 1, "A left");
        assert_eq!(t.dropped_total(), 0, "A dropped");
        assert_eq!(t.expired_total(), 0, "A expired");
    }
    // B：TTL 边界（恰好 500 不过期，501 过期）
    {
        let mut t = SkipKeyTable::new();
        t.push(0x08, false, 1000);
        t.purge(1500);
        assert_eq!(t.count(), 1, "B at500");
        t.purge(1501);
        assert_eq!(t.count(), 0, "B at501");
        assert_eq!(t.expired_total(), 1, "B expired");
    }
    // C：溢出
    {
        let mut t = SkipKeyTable::new();
        for i in 0..138u16 {
            t.push(i, false, 1000);
        }
        assert_eq!(t.count(), 128, "C count");
        assert_eq!(t.dropped_total(), 10, "C dropped");
    }
    // D：通道隔离
    {
        let mut t = SkipKeyTable::new();
        t.push(0x25, false, 1000);
        assert!(!t.try_consume(0x25, true, 1001), "D up");
        assert!(t.try_consume(0x25, false, 1002), "D down");
    }
}

// ── PassthroughNote 的等价性（oracle: /tmp/skipverify/pt.cpp, pt2.cpp）──
use aurora_tsf_win::PassthroughState;

#[test]
fn passthrough_equivalence_with_cpp_impl() {
    // 单次判定的完整真值表（C++ 输出 A/B/C/D/E 与 M 系列）
    let cases: &[(u32, bool, bool, bool)] = &[
        (0x41, false, false, true),  // A: 普通透传 → 记
        (0x41, true, false, false),  // B: 吃了 → 不记
        (0x41, false, true, false),  // C: 抑制 → 不记
        (0xE7, false, false, false), // D: VK_PACKET → 不记
        (0xE8, false, false, false), // E: VK_ASYNC_COMMIT_TRIGGER → 不记
        (0x10, false, false, false), // M: Shift
        (0x11, false, false, false), // Control
        (0x12, false, false, false), // Menu
        (0x5B, false, false, false), // LWin
        (0x5C, false, false, false), // RWin
        (0xA0, false, false, false), // LShift
        (0xA1, false, false, false), // RShift
        (0xA2, false, false, false), // LControl
        (0xA3, false, false, false), // RControl
        (0xA4, false, false, false), // LMenu
        (0xA5, false, false, false), // RMenu
    ];
    for &(vk, eaten, sup, want) in cases {
        let mut s = PassthroughState::new();
        s.note_key_down(vk, eaten, sup);
        assert_eq!(
            s.pending(),
            want,
            "vk=0x{vk:02X} eaten={eaten} suppressed={sup}"
        );
    }

    // take / restore / reset（pt2.cpp: first=1 second=0）
    let mut s = PassthroughState::new();
    s.note_key_down(0x41, false, false);
    assert!(s.take_on_key_down_send(), "first");
    assert!(!s.take_on_key_down_send(), "second");

    s.note_key_down(0x41, false, false);
    assert!(s.take_on_key_down_send());
    s.restore_on_send_failure();
    assert!(s.pending(), "afterRestore");
    s.reset();
    assert!(!s.pending(), "afterReset");
}

// ── EditableContextPolicy 的等价性（oracle: /tmp/skipverify/ec.cpp）──
//
// 这条是**实测纠正**的产物：移植时凭直觉写了注释「其余失败一律不可编辑」，
// 单测也跟着写反；用 g++ 编译原头文件跑出来才发现语义正好相反——
// 除「空上下文」外，失败一律**算可编辑**。语义本身就是反直觉的，
// 所以必须有 oracle 对照，不能靠读代码推断。
#[test]
fn editable_context_equivalence_with_cpp_impl() {
    use aurora_tsf_win::policy::editable_context::context_is_editable;

    // C++ 实测输出：
    //   S_OK,0            -> 1
    //   S_OK,READONLY     -> 0
    //   EMPTY_CONTEXT_HR  -> 0
    //   E_FAIL            -> 1
    //   E_INVALIDARG      -> 1
    //   TS_E_NOOBJECT     -> 1
    let cases: &[(u32, u32, bool, &str)] = &[
        (0x0000_0000, 0x0, true, "S_OK 且非只读"),
        (0x0000_0000, 0x1, false, "S_OK 但只读"),
        (0x8004_0509, 0x0, false, "空上下文"),
        (0x8000_4005, 0x0, true, "E_FAIL"),
        (0x8007_0057, 0x0, true, "E_INVALIDARG"),
        (0x8004_0506, 0x0, true, "TS_E_NOOBJECT"),
    ];
    for &(hr, flags, want, why) in cases {
        assert_eq!(
            context_is_editable(hr, flags),
            want,
            "hr=0x{hr:08X} flags=0x{flags:X}（{why}）"
        );
    }
}

// ── 光标 / 候选窗几何的等价性（oracle: /tmp/skipverify/geo.cpp）──
#[test]
fn caret_geometry_equivalence_with_cpp_impl() {
    use aurora_tsf_win::policy::caret_geometry::*;

    let empty = Rect::new(0, 0, 0, 0);
    let caret = Rect::new(100, 50, 102, 70);
    let comp = Rect::new(100, 50, 300, 70);
    let outside = Rect::new(400, 50, 402, 70);

    // C++ 实测：
    //   defaultPos(empty,empty,empty) = 1
    //   defaultPos(caret,caret,caret) = 0
    //   defaultPos(empty,caret,empty) = 0
    assert!(is_host_default_position(&empty, true, &empty, true, &empty));
    assert!(!is_host_default_position(
        &caret, true, &caret, true, &caret
    ));
    assert!(!is_host_default_position(
        &empty, true, &caret, true, &empty
    ));

    //   laidOut(caret,comp)   = 1
    //   laidOut(caret,caret)  = 0
    //   laidOut(outside,comp) = 0
    //   laidOut(empty,comp)   = 0
    assert!(shows_laid_out_composition(true, &caret, true, &comp));
    assert!(!shows_laid_out_composition(true, &caret, true, &caret));
    assert!(!shows_laid_out_composition(true, &outside, true, &comp));
    assert!(!shows_laid_out_composition(true, &empty, true, &comp));

    //   fallback(F,T,T,200,T) = 1 / (T,...) = 0 / (F,T,T,0,T) = 0
    assert!(allow_composition_rect_fallback(
        false, true, true, 200, true
    ));
    assert!(!allow_composition_rect_fallback(
        true, true, true, 200, true
    ));
    assert!(!allow_composition_rect_fallback(false, true, true, 0, true));

    //   candForm(T,T,POINT,100,200) = 1 / (-10,200) = 0 / (F,...) = 0
    let screen = Rect::new(0, 0, 1920, 1080);
    assert!(accept_candidate_form_point(
        true, true, CFS_POINT, 100, 200, true, &screen
    ));
    assert!(!accept_candidate_form_point(
        true, true, CFS_POINT, -10, 200, true, &screen
    ));
    assert!(!accept_candidate_form_point(
        false, true, CFS_POINT, 100, 200, true, &screen
    ));
}

// ── InstallerGuard 的等价性（oracle: /tmp/skipverify/ig.cpp）──
#[test]
fn installer_guard_equivalence_with_cpp_impl() {
    use aurora_tsf_win::policy::installer_guard::*;

    // ParseOwner（C++ 输出 A..G）
    let ok_cases = [
        (
            "1234|133000000000000000",
            Some((1234u32, 133000000000000000u64)),
        ),
        ("4294967295|123", Some((4294967295u32, 123u64))),
    ];
    for (input, want) in ok_cases {
        let got = parse_owner(input).map(|o| (o.pid, o.create_time));
        assert_eq!(got, want, "parse_owner({input:?})");
    }
    // B/D/E/F/G 都应拒绝
    for bad in [
        "0|133000000000000000",
        "4294967296|123",
        "1234|",
        "|1",
        "1234",
    ] {
        assert!(parse_owner(bad).is_none(), "应拒绝 {bad:?}");
    }

    // ShouldBlockStart 全 32 组合（C++ 输出在上面）
    // 顺序: has_flag, has_owner, owner_alive, stale_by_age, owner_hard_stale
    let oracle: [(bool, bool, bool, bool, bool, bool); 32] = [
        (false, false, false, false, false, false),
        (false, false, false, false, true, false),
        (false, false, false, true, false, false),
        (false, false, false, true, true, false),
        (false, false, true, false, false, false),
        (false, false, true, false, true, false),
        (false, false, true, true, false, false),
        (false, false, true, true, true, false),
        (false, true, false, false, false, false),
        (false, true, false, false, true, false),
        (false, true, false, true, false, false),
        (false, true, false, true, true, false),
        (false, true, true, false, false, false),
        (false, true, true, false, true, false),
        (false, true, true, true, false, false),
        (false, true, true, true, true, false),
        (true, false, false, false, false, true),
        (true, false, false, false, true, true),
        (true, false, false, true, false, false),
        (true, false, false, true, true, false),
        (true, false, true, false, false, true),
        (true, false, true, false, true, true),
        (true, false, true, true, false, false),
        (true, false, true, true, true, false),
        (true, true, false, false, false, false),
        (true, true, false, false, true, false),
        (true, true, false, true, false, false),
        (true, true, false, true, true, false),
        (true, true, true, false, false, true),
        (true, true, true, false, true, false),
        (true, true, true, true, false, true),
        (true, true, true, true, true, false),
    ];
    for (i, &(f, ho, oa, sa, hs, want)) in oracle.iter().enumerate() {
        assert_eq!(
            should_block_start(f, ho, oa, sa, hs),
            want,
            "组合 #{i}: flag={f} owner={ho} alive={oa} staleAge={sa} hardStale={hs}"
        );
    }
}

// ── UiElementPolicy 的等价性（oracle: /tmp/skipverify/ue.cpp）──
#[test]
fn ui_element_equivalence_with_cpp_impl() {
    use aurora_tsf_win::policy::ui_element::*;

    // UseSnapshot：16 组合，C++ 输出「只有 0000 与 0100/0101/0110/0111、1xxx 为真」
    // 顺序: declared, read_candidates, snapshot_empty, dirty
    let use_snapshot_oracle = [
        // 0000..1111
        true, false, false, false, // 0000=true, 0001=false, 0010=false, 0011=false
        true, true, true, true, // 01xx 全 true（read_candidates）
        true, true, true, true, // 10xx 全 true（declared）
        true, true, true, true, // 11xx 全 true
    ];
    for (i, &want) in use_snapshot_oracle.iter().enumerate() {
        let d = i & 8 != 0;
        let r = i & 4 != 0;
        let e = i & 2 != 0;
        let di = i & 1 != 0;
        assert_eq!(use_snapshot(d, r, e, di), want, "UseSnapshot {d}{r}{e}{di}");
    }

    // ShouldRefreshOnGet：8 组合，只有 dirty && (contentRead||readCandidates) 为真
    let refresh_oracle = [false, false, false, false, false, true, true, true];
    for (i, &want) in refresh_oracle.iter().enumerate() {
        let di = i & 4 != 0;
        let c = i & 2 != 0;
        let r = i & 1 != 0;
        assert_eq!(
            should_refresh_on_get(di, c, r),
            want,
            "ShouldRefreshOnGet {di}{c}{r}"
        );
    }

    // HostDraws / Eager / Invariant
    // C++: H 00 -> 000, H 01/10/11 -> 111
    let h_oracle = [
        (false, false, false),
        (true, true, true),
        (true, true, true),
        (true, true, true),
    ];
    for (i, &(wh, we, wi)) in h_oracle.iter().enumerate() {
        let d = i & 2 != 0;
        let r = i & 1 != 0;
        assert_eq!(host_draws(d, r), wh, "HostDraws {d}{r}");
        assert_eq!(should_refresh_eagerly(d, r), we, "Eager {d}{r}");
        assert_eq!(invariant_broken(true, d, r), wi, "Invariant(true,{d}{r})");
    }
}

// ── mode_status 载荷格式与 aurora-ipc 编码器对账 ──
//
// 这是跨语言边界的字节布局，必须用**真编码器**验证而不是手写的近似：
// 手写近似只能证明「我照自己想的写对了」。
#[test]
fn mode_status_matches_aurora_ipc_encoder() {
    use aurora_tsf_win::policy::mode_status::{FLAG_CHINESE_PUNCT, FLAG_FULL_WIDTH, parse};

    for (flags, mode, label) in [
        (FLAG_CHINESE_PUNCT, 0u32, "拼音"),
        (0u32, 1u32, ""),
        (FLAG_FULL_WIDTH | FLAG_CHINESE_PUNCT, 0, "全角"),
    ] {
        // 用 aurora-ipc 的真编码器产出字节
        let frame = aurora_ipc::codec::encode_mode_status(flags, mode, label);
        // 跳过 8 字节帧头，载荷才是 parse 的输入
        let payload = &frame[8..];

        let st = parse(payload).unwrap_or_else(|| {
            panic!("真编码器产的载荷必须能解析: flags={flags} mode={mode} label={label:?}")
        });
        assert_eq!(st.is_chinese(), mode == 0, "mode=0 应为中文");
        assert_eq!(st.is_full_width(), flags & FLAG_FULL_WIDTH != 0);
        assert_eq!(st.is_chinese_punct(), flags & FLAG_CHINESE_PUNCT != 0);
        assert_eq!(st.label, label);
    }
}
