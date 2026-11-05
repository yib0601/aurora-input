//! 方案常驻策略（`schema.keep_all_loaded`，设计 `docs/design/memory-footprint.md` §5）端到端：
//! 启动常驻集合、启动后台**只校验盘上已有缓存**的其余方案（缺的不预建，2026-11 起的契约）、
//! 闲置淘汰与保护名单、运行期两个方向的切换。
//!
//! 整棵目录树经**便携标记**重定向到临时目录（`AURORA_INSTALL_ROOT` + `portable_mode`）：
//! 词库缓存落在 `<root>/localdata/cache`（可观测、不碰真实缓存），`reload_user_config` 读
//! `<root>/userdata/config.toml`。一个进程只能重定向一次，故单开测试二进制；用例共用这棵树，
//! 用 `LOCK` 串行、每条开头重写配置。
//!
//! ⚠️ 不依赖 `build_dev/data`：方案全部自造（码表 za / zb / zc / zd + 混输 zm），不会静默跳过。
//!
//! 方案角色（各用例同一套）：
//! - `za`：当前方案；
//! - `zc`：临拼目标（`primary_pinyin = "zc"`——目标只按 id 解析，码表方案也能当靶子）；
//! - `zm`：混输，主码表成员 `zb`（成员不保护：混输自建子引擎）；
//! - `zd`：无任何保护；
//! - `english`：英文方案（只在 `schema.mix.enable_english` 的用例里被混输取走）。

use aurora_bridge::handler::{KeyEventData, MessageHandler};
use aurora_config::Config;
use aurora_coordinator::Coordinator;
use aurora_coordinator::web_host::WebDataHost;
use aurora_ipc::protocol::EVENT_KEY_DOWN;
use aurora_ui_types::MenuCmd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

static LOCK: Mutex<()> = Mutex::new(());

const CODETABLES: [(&str, &str); 4] = [("za", "甲"), ("zb", "乙"), ("zc", "丙"), ("zd", "丁")];

/// 便携根目录（进程内只建一次）。
fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = root_unchecked();
        let schemas = root.join("data/schemas");
        for (id, word) in CODETABLES {
            std::fs::create_dir_all(schemas.join(id)).unwrap();
            std::fs::write(
                schemas.join(format!("{id}.schema.toml")),
                format!(
                    "[schema]\nid = \"{id}\"\nname = \"{id}\"\n\
                     [engine]\ntype = \"codetable\"\n\
                     [engine.codetable]\nmax_code_length = 4\n\
                     [[dictionaries]]\nid = \"main\"\npath = \"{id}/{id}.dict.yaml\"\ndefault = true\n"
                ),
            )
            .unwrap();
            write_dict(id, word);
        }
        std::fs::write(
            schemas.join("zm.schema.toml"),
            "[schema]\nid = \"zm\"\nname = \"zm\"\n\
             [engine]\ntype = \"mixed\"\n\
             [engine.mixed]\nprimary_schema = \"zb\"\nsecondary_schema = \"\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(schemas.join("english")).unwrap();
        std::fs::write(
            schemas.join("english.schema.toml"),
            "[schema]\nid = \"english\"\nname = \"english\"\n\
             [engine]\ntype = \"english\"\n\
             [engine.codetable]\nmax_code_length = 32\n\
             [[dictionaries]]\nid = \"en_main\"\npath = \"english/en.dict.yaml\"\n\
             type = \"english\"\ndefault = true\n",
        )
        .unwrap();
        std::fs::write(
            schemas.join("english/en.dict.yaml"),
            "---\nname: en\nversion: \"1\"\n...\nhello\thello\n",
        )
        .unwrap();
        std::fs::write(root.join(aurora_config::variant::PORTABLE_MARKER_NAME), "").unwrap();
        std::fs::create_dir_all(root.join("userdata")).unwrap();
        // SAFETY: 在任何 OnceLock（variant / 路径缓存 / 引擎缓存根）初始化之前设置；
        // ROOT 的 get_or_init 保证只跑一次，且用例都先经 `LOCK` 串行。
        unsafe {
            std::env::set_var("AURORA_INSTALL_ROOT", &root);
        }
        assert!(aurora_config::variant::is_portable(), "前置条件：便携标记须生效");
        assert_eq!(
            Config::cache_dir(),
            Some(root.join("localdata/cache")),
            "前置条件：缓存根须已重定向，否则本测试会读写真实缓存"
        );
        root
    })
}

fn write_dict(id: &str, word: &str) {
    std::fs::write(
        root_unchecked()
            .join("data/schemas")
            .join(format!("{id}/{id}.dict.yaml")),
        format!("---\nname: {id}\nversion: \"1\"\n...\n{word}\ta\n"),
    )
    .unwrap();
}

/// `root()` 初始化期间也要写词库，那时 ROOT 还没落值；两边共用同一个进程级目录。
///
/// 进程级托管（unix 由 atexit 删）：不能换 `scratch()`——它归首个 test 线程，
/// 线程一结束就删，而 `ROOT` 还缓存着这个路径。
fn root_unchecked() -> PathBuf {
    static ROOT_PATH: OnceLock<PathBuf> = OnceLock::new();
    ROOT_PATH
        .get_or_init(|| aurora_utils::testing::process_scratch("residency"))
        .clone()
}

fn user() -> PathBuf {
    root().join("userdata")
}

fn cache_root() -> PathBuf {
    root().join("localdata/cache")
}

/// `<cache>/<id>/` 下的 `.wdat`（码表词库缓存）。
/// 该方案目录下**最新**的 wdat。缓存按内容命名、重建写新文件且旧份保留到被 prune
/// （`cache_fp` S2），同一时刻目录里可能有新旧两份，取 mtime 最新的才是当前这份。
fn wdat_of(id: &str) -> Option<PathBuf> {
    std::fs::read_dir(cache_root().join(id))
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "wdat"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// 每条用例的起点。`keep_all` 为 `schema.keep_all_loaded`。
fn reset(keep_all: bool) {
    reset_with(keep_all, "");
}

/// 同 [`reset`]，`extra` 原样追加到 config.toml 末尾（整张表，如 `[schema.mix]`）。
fn reset_with(keep_all: bool, extra: &str) {
    root();
    std::fs::write(
        user().join("config.toml"),
        format!(
            "[schema]\nactive = \"za\"\navailable = [\"za\", \"zb\", \"zc\", \"zd\", \"zm\"]\n\
             primary_codetable = \"zb\"\nprimary_pinyin = \"zc\"\nkeep_all_loaded = {keep_all}\n\
             [input.temp_pinyin]\nenabled = true\n\
             [input.temp_english]\nshow_candidates = false\n{extra}"
        ),
    )
    .unwrap();
}

fn set_keep_all(keep_all: bool) {
    let p = user().join("config.toml");
    let text = std::fs::read_to_string(&p).unwrap();
    let text = text.replace(
        &format!("keep_all_loaded = {}", !keep_all),
        &format!("keep_all_loaded = {keep_all}"),
    );
    std::fs::write(&p, text).unwrap();
}

/// 设置页保存了一个会把 schema 段标脏的键（引擎整体重建）。
fn dirty_schema_save() {
    let p = user().join("config.toml");
    let mut text = std::fs::read_to_string(&p).unwrap();
    // 夹具没有 data/config.toml，出厂值取 L1（false），故写 true 才是一次变更。
    text.push_str("[schema.codetable]\ntop_code_commit = true\n");
    std::fs::write(&p, text).unwrap();
}

/// 等后台线程把已加载集合推到 `want`（最多 20 秒）。
fn wait_loaded(c: &Coordinator, want: &[String]) {
    let t0 = Instant::now();
    while loaded(c) != want && t0.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn make() -> Arc<Coordinator> {
    let data = Config::data_dir();
    Coordinator::new_headless_with_ui_at(
        Config::load(data.as_deref()).unwrap(),
        data.as_deref(),
        Some(&user()),
    )
    .0
}

fn loaded(c: &Coordinator) -> Vec<String> {
    c.engine_mgr().loaded_schemas()
}

fn sorted(v: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = v.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

fn key(c: &Coordinator, vk: u32) {
    c.handle_key_event(&KeyEventData {
        key_code: vk,
        scan_code: 0,
        modifiers: 0,
        event_type: EVENT_KEY_DOWN,
        toggles: 0,
        event_seq: 0,
        prev_char: 0,
    });
}

/// 菜单切到 available 第 i 项后敲 `a`，返回首页候选；敲 Esc 收尾。
fn switch_and_type(c: &Coordinator, i: usize) -> Vec<String> {
    c.debug_run_menu_cmd(MenuCmd::SchemaSelect(i));
    key(c, 0x41);
    let texts = c.debug_page_texts();
    key(c, 0x1B);
    texts
}

/// 菜单切到 `available` 第 `menu_index` 项一次，让那个方案（及它的词库缓存）**落到盘上**。
///
/// 新契约（2026-11）下缓存只在**真正用到**时产生，于是「启动后应该常驻 / 应该被校验重建」
/// 这类断言必须先有缓存。此前这些用例靠同进程其它用例留下的残留蒙对——顺序一变就飘，
/// 所以这里显式造，不依赖别人。
///
/// ⚠️ 手切方案会把 `schema.active` **写进 config.toml**（`app_schema` 的既定行为），
/// 故调用方必须在后面**重跑一次 `reset*()`** 把夹具配置写回去，否则本用例后续 `make()`
/// 的当前方案就不是 `za` 了。
fn prime_cache(menu_index: usize) {
    let c = make();
    c.debug_run_menu_cmd(MenuCmd::SchemaSelect(menu_index));
}

/// `<cache>/<id>/` 下的缓存产物计数：`(.wdat 本体, .wdat 的 .fp/.st sidecar)`。
///
/// **分开数，别用「目录非空 / 有文件」这种粗判据**：内容寻址缓存会在 `.wdat` 旁边留
/// `.wdat.fp`（摘要）与 `.wdat.st`（源状态快照），于是「建了又删」（拼音 merged 那条
/// 中间段回收）与「压根没建」在只看目录条目时长得一样——前者会留下 sidecar。本用例要
/// 断言的是后者（**没建**），所以两个数都要为 0。
fn cache_artifacts(id: &str) -> (usize, usize) {
    let (mut bodies, mut sidecars) = (0usize, 0usize);
    let Ok(rd) = std::fs::read_dir(cache_root().join(id)) else {
        return (0, 0);
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".wdat") {
            bodies += 1;
        } else if name.ends_with(".wdat.fp") || name.ends_with(".wdat.st") {
            sidecars += 1;
        }
    }
    (bodies, sidecars)
}

/// `false`（2026-11 起的契约）：启动只常驻**当前方案**；临拼目标与其余可用方案只**校验
/// 盘上已有缓存**的方案，缓存不在的一律不预建，留到首次切换 / 首次按引导键。
///
/// ★ 契约为什么改了（本用例随之改名）：旧断言是「启动后台把**全部**可用方案的缓存都建
/// 出来（含临拼目标 zc）」。那条契约的代价是每个新装用户都替可能永不使用的方案编译并
/// 落盘词库——出厂默认方案是五笔，而 `available` 里的混输成员会把拼音词库带出来，实测
/// 141 MB 盘 + 3.8 s CPU（runtime 审计 E3/E4/E10）。新契约把「建」这半个动作推给真实
/// 使用，留着「校验 / 重建已有缓存」那半个。
///
/// 旧用例守的三件事一件没丢，第二条按新契约收窄到它**唯一还存在的形态**：
/// ① `keep_all_loaded = false` 时只建常驻集合（当前方案；临拼目标缓存不在就不预热）；
/// ② 缓存**存在但陈旧**（升级包刷新了词库）⇒ 启动后台仍然重建——现在是唯一还需要启动期
///    重建的路径，强度不降，只是前置条件变成「先切过去一次让它有缓存」；
/// ③ 闲置清扫 / 常驻集合语义由本文件其余用例守。
#[test]
fn startup_without_keep_all_defers_missing_caches_and_repairs_existing() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset(false);
    let _ = std::fs::remove_dir_all(cache_root());
    let c = make();
    c.debug_desktop_startup_and_prewarm();
    assert_eq!(
        loaded(&c),
        sorted(&["za"]),
        "只常驻当前方案：临拼目标 zc 的缓存不在 ⇒ 也不预热（旧契约是 [za, zc]）"
    );
    assert_eq!(cache_artifacts("za").0, 1, "当前方案的缓存照建");
    // 新契约的核心断言：缓存不在的方案一条都不预建。两个数都为 0 才排除「建了又删」。
    for id in ["zb", "zc", "zd", "zm"] {
        assert_eq!(
            cache_artifacts(id),
            (0, 0),
            "{id} 的缓存在启动时不该被建出来（缺的不预建，留到首次切换 / 首次按引导键）"
        );
    }

    // 切到 zd 一次：新契约下这是它第一次拿到缓存的唯一途径（旧契约里启动就建好了）。
    assert_eq!(
        switch_and_type(&c, 3).first().map(String::as_str),
        Some("丁")
    );
    assert_eq!(cache_artifacts("zd").0, 1, "切过去之后 zd 才有缓存");
    drop(c);

    // ② 缓存**已存在但陈旧**：改 zd 的词库后重启，启动后台必须重建它。这条路径不能丢，
    // 否则升级包刷新词库后，用户首次切到该方案要现场等几秒。
    write_dict("zd", "戊");
    let before = std::fs::read(wdat_of("zd").unwrap()).unwrap();
    let c = make();
    c.debug_desktop_startup_and_prewarm();
    let rebuilt = wdat_of("zd").unwrap();
    assert_ne!(
        std::fs::read(&rebuilt).unwrap(),
        before,
        "缓存陈旧 ⇒ 启动后台应重建（新增的那把尺子只拦「缓存不在」，不拦「陈旧」）"
    );
    let stamp = std::fs::metadata(&rebuilt).unwrap().modified().unwrap();
    assert_eq!(
        switch_and_type(&c, 3).first().map(String::as_str),
        Some("戊")
    );
    assert_eq!(
        std::fs::metadata(&rebuilt).unwrap().modified().unwrap(),
        stamp,
        "缓存已新鲜，切换不该再重建"
    );
    write_dict("zd", "丁");
}

/// `true`：与现状一致——启动预热全部可用方案 + 临拼目标，且清扫拍子不摘任何引擎。
#[test]
fn keep_all_loaded_prewarms_everything_and_never_evicts() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset(true);
    let c = make();
    c.debug_desktop_startup_and_prewarm();
    let all = sorted(&["za", "zb", "zc", "zd", "zm"]);
    assert_eq!(loaded(&c), all);
    std::thread::sleep(Duration::from_millis(50));
    assert!(c.debug_idle_sweep(Duration::ZERO).is_empty());
    assert_eq!(loaded(&c), all, "常驻开着时一个都不摘");
}

/// 懒建缓存的闲置清扫（设计 §7）走同一个清扫拍子，且**不看** `keep_all_loaded`：常驻开着
/// 时方案一个不摘，单字全码表与按词查编码用户层照样到期释放。
#[test]
fn idle_sweep_releases_caches_even_when_keep_all() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset(true);
    // 用户库单开一份：别的用例的协调器可能还被后台线程握着，redb 不能同时开两次（开不了
    // 时 store 为 None，用户层根本建不出来）。
    let own = root().join("userdata-sweep");
    std::fs::create_dir_all(&own).unwrap();
    let data = Config::data_dir();
    let c = Coordinator::new_headless_with_ui_at(
        Config::load(data.as_deref()).unwrap(),
        data.as_deref(),
        Some(&own),
    )
    .0;
    c.debug_desktop_startup_and_prewarm();
    let em = c.engine_mgr();
    assert!(em.prewarm_single_char_codes("za"));
    em.prewarm_text_codes("za");
    assert_eq!(em.user_text_loaded(), 1, "前提：用户层已建");
    std::thread::sleep(Duration::from_millis(80));
    assert!(
        c.debug_idle_sweep(Duration::from_millis(40)).is_empty(),
        "常驻开着时方案一个不摘"
    );
    assert!(!em.single_char_codes_ready("za"), "单字全码表到期释放");
    assert_eq!(em.user_text_loaded(), 0, "用户层到期释放");
    assert_eq!(loaded(&c), sorted(&["za", "zb", "zc", "zd", "zm"]));
}

/// 闲置淘汰：未用的摘掉；当前方案、临拼目标不摘；混输成员不保护（混输自建子引擎）；
/// 摘掉后再切回能正常出字；摘除不清掉仍在用的反查索引。
#[test]
fn idle_eviction_spares_protected_and_in_use_indexes() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset(false);
    // 临拼目标 zc 要「启动即常驻」，新契约下得先让它的缓存存在（见 `prime_cache`），
    // 再重写一遍夹具配置把手切写盘的 schema.active 恢复成 za。
    prime_cache(2);
    reset(false);
    assert!(wdat_of("zc").is_some(), "夹具前置：zc 的缓存应已落盘");
    let c = make();
    c.debug_desktop_startup_and_prewarm();
    let em = c.engine_mgr();
    for id in ["zb", "zd", "zm"] {
        assert!(em.prewarm_schema(id));
    }
    // zb 是主码表：它的反查索引给编码提示用，与 zb 的引擎在不在内存无关。zd 的不在用。
    assert!(em.prewarm_reverse_index("zb"));
    assert!(em.prewarm_reverse_index("zd"));
    assert_eq!(loaded(&c), sorted(&["za", "zb", "zc", "zd", "zm"]));

    // 第一拍只推进时钟（阈值很大，谁都不摘），随后用一下 zm：它的「最后使用」= 这一拍。
    std::thread::sleep(Duration::from_secs(2));
    assert!(c.debug_idle_sweep(Duration::from_secs(3600)).is_empty());
    assert!(!em.convert_with("zm", "a", 5).candidates.is_empty());
    // 第二拍：zd、zb 闲了 ≥2s 被摘（zb 虽是 zm 的成员，混输用的是自建的子引擎）；
    // zm 刚用过；za 当前；zc 临拼目标。
    let mut evicted = c.debug_idle_sweep(Duration::from_millis(1500));
    evicted.sort();
    assert_eq!(evicted, sorted(&["zb", "zd"]));
    assert_eq!(loaded(&c), sorted(&["za", "zc", "zm"]));
    assert!(
        !em.convert_with("zm", "a", 5).candidates.is_empty(),
        "成员的独立引擎被摘，混输照常出字"
    );
    assert!(
        em.reverse_index_if_ready("zd").is_none(),
        "只属于被摘方案的反查索引随之释放"
    );
    assert!(
        em.reverse_index_if_ready("zb").is_some(),
        "主码表的反查索引仍在用，不能清"
    );

    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(c.debug_idle_sweep(Duration::from_millis(1500)), vec!["zm"]);
    assert_eq!(
        loaded(&c),
        sorted(&["za", "zc"]),
        "当前方案与临拼目标始终不摘"
    );
    assert!(
        em.reverse_index_if_ready("zb").is_some(),
        "主码表引擎被摘，它的反查索引仍留着"
    );

    // 摘掉后再切回：现建、能出字。
    assert_eq!(
        switch_and_type(&c, 3).first().map(String::as_str),
        Some("丁")
    );
    assert_eq!(
        switch_and_type(&c, 4).first().map(String::as_str),
        Some("乙")
    );
}

/// 运行期改键：true→false 立即按保护名单摘除；false→true 立即后台预热全部。不需要重启，
/// 也不触发整套引擎重建（当前方案的引擎不换）。
#[test]
fn runtime_toggle_applies_both_directions_without_restart() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset(true);
    let c = make();
    c.debug_desktop_startup_and_prewarm();
    assert_eq!(loaded(&c), sorted(&["za", "zb", "zc", "zd", "zm"]));

    set_keep_all(false);
    c.reload_user_config();
    assert_eq!(
        loaded(&c),
        sorted(&["za", "zc"]),
        "关掉常驻：保护名单外的立即摘"
    );

    set_keep_all(true);
    c.reload_user_config();
    let all = sorted(&["za", "zb", "zc", "zd", "zm"]);
    let t0 = Instant::now();
    while loaded(&c) != all && t0.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(loaded(&c), all, "打开常驻：后台预热全部可用方案");
}

/// 设置页保存了标脏 schema 段的键：引擎整体重建后，常驻集合要回来——`true` 是全部方案，
/// `false` 是保护名单（当前方案 + 临拼目标），而不是只剩当前方案直到重启。
#[test]
fn dirty_reload_restores_resident_set() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for (keep_all, want) in [
        (true, sorted(&["za", "zb", "zc", "zd", "zm"])),
        (false, sorted(&["za", "zc"])),
    ] {
        reset(keep_all);
        // keep_all=false 那一支要断言「保护名单（当前方案 + 临拼目标）回来了」，而新契约下
        // 临拼目标只在缓存已存在时才预热——先造出 zc 的缓存，断言才守得住这条语义。
        // 手切会写盘 schema.active，故造完再 reset 一次恢复夹具配置。
        if !keep_all {
            prime_cache(2);
            reset(keep_all);
        }
        let c = make();
        c.debug_desktop_startup_and_prewarm();
        assert_eq!(loaded(&c), want);
        dirty_schema_save();
        c.reload_user_config();
        wait_loaded(&c, &want);
        assert_eq!(
            loaded(&c),
            want,
            "keep_all_loaded={keep_all}：重建后常驻集合应回来"
        );
    }
}

/// 启动校验让混输取走了共享英文引擎、而眼下没有混输在内存、临英又关着：共享缓存要放手，
/// 英文不再受保护，闲置清扫能摘掉它。
#[test]
fn validation_releases_shared_english_when_no_consumer() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_with(false, "[schema.mix]\nenable_english = true\n");
    // 新契约下「启动校验」只对**已有缓存**的方案跑；本用例要的正是那一步把共享英文取走，
    // 故先让 zm 有缓存——它自己是混输、不带词库，缓存落在主码表成员 zb 上。
    prime_cache(4);
    reset_with(false, "[schema.mix]\nenable_english = true\n");
    assert!(
        wdat_of("zb").is_some(),
        "夹具前置：zm 的成员 zb 缓存应已落盘"
    );
    let c = make();
    c.debug_desktop_startup_and_prewarm();
    assert!(!loaded(&c).contains(&"zm".to_string()), "zm 校验完即释放");
    let evicted = c.debug_idle_sweep(Duration::ZERO);
    assert!(
        evicted.contains(&"english".to_string()),
        "没有消费者的英文引擎应能被摘：摘了 {evicted:?}，剩 {:?}",
        loaded(&c)
    );
}

/// 宿主声明按需加载（移动端 `set_eager_prewarm(false)`）时，把常驻打开也不在后台建全部方案。
#[test]
fn turning_keep_all_on_respects_eager_prewarm_off() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset(false);
    // 断言里的 zc 是「常驻的临拼目标」：新契约下它得先有缓存（见 `prime_cache`）。
    prime_cache(2);
    reset(false);
    let c = make();
    c.set_eager_prewarm(false);
    c.debug_desktop_startup_and_prewarm();
    set_keep_all(true);
    c.reload_user_config();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(loaded(&c), sorted(&["za", "zc"]));
}

/// 「全部常驻」的预热循环逐个复核开关：中途被关掉就停（此处开关一开始就是关的）。
#[test]
fn prewarm_all_stops_when_keep_all_is_off() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset(false);
    let c = make();
    c.debug_prewarm_all_schemas();
    assert_eq!(loaded(&c), sorted(&["za", "zc"]));
}

/// 快捷输入成员的取数上限按成员**引擎类型**分级：成员没加载时也要按它的真实类型算
/// （码表单码 100），不能当成「类型未知」落到 300。
#[test]
fn mix_member_limit_uses_real_type_of_unloaded_member() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset(false);
    let c = make();
    assert!(!loaded(&c).contains(&"zd".to_string()));
    assert_eq!(c.debug_mix_member_fetch_limit("zd", "a"), 100);
}
