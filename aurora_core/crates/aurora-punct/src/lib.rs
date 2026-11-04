//! aurora-punct: 标点转换纯逻辑（从 aurora-coordinator 抽出，可原生测试）。
//!
//! 与 Go `aurora_core/internal/coordinator/handle_punctuation.go` 对齐。所有函数无副作用
//! （除经 `&mut PunctuationConverter` 推进引号状态机外），输入为配置 + 当前模式布尔
//! （chinese_punct / full_width），便于单测。
//!
//! 转换优先级（对齐 Go `convertPunct`）：自定义映射 → 数字后智能 → 中文标点 → 全半角。
//!
//! # ★★ 参数是 `&PunctConfig` 而不是 `&InputConfig`（自定义标点方案级化的关键）
//!
//! 自定义映射表可以由方案整表替换（[`aurora_config::PunctSpec::custom_mappings`]），于是
//! 「该用哪张表」不再是全局唯一答案——临英查 `english` 方案的表、快符查快符方案的表、
//! 主输入路查活跃方案的表。调用方必须显式说明用的是**哪一份**。
//!
//! ⇒ 凡读 `punct.*` 的函数一律收窄到 `&PunctConfig`，由 `Coordinator::effective_punct`
//! 供给；**经本 crate 的消费点漏接一个就是编译失败**，而不是「这条路上方案表静默不生效」。
//! 同型见 `[phrases]` 六闸门把 `scope` 做成必填参数（漏接表现是另一个功能停止工作，零日志）。
//!
//! ⚠️ **这道防线有一个缺口**：`PunctuationConverter::peek_custom`（aurora-transform）可以被
//! 直接调用，绕过本 crate（消费方是引号左右形与英文配对——后者已随自动配对功能删除，
//! 现只剩本 crate 自己的 `quote_forms`）。**再出现这种直调时，要么收编进本 crate，
//! 要么给它补一条守门测试**——签名收窄管不着它。
//!
//! 另两个吃 `&InputConfig` 的是透传集判定（[`chinese_passthrough_punct_chars`] /
//! [`english_passthrough_punct_chars`]）——它们还要看按键占用层之外的东西，故仍需整份配置。

use aurora_config::config::{InputConfig, PunctConfig};
use aurora_transform::fullwidth::to_full_width;
use aurora_transform::punctuation::PunctuationConverter;

/// 数字后智能标点：中文标点模式下，若 ch 在智能标点列表且光标前一字符为数字，
/// 则该标点按英文（半角）输出（如 "3." 不转 "3。"）。`prev_char` 为 UTF-16 单元（0=不可用）。
pub fn is_smart_punct_after_digit(punct: &PunctConfig, ch: char, prev_char: u16) -> bool {
    if !punct.smart_after_digit {
        return false;
    }
    let list = &punct.smart_list;
    let in_list = if list.is_empty() {
        ch == '.' || ch == ','
    } else {
        list.contains(ch)
    };
    if !in_list {
        return false;
    }
    // 数字 '0'..='9' = 0x30..=0x39
    (0x30..=0x39).contains(&prev_char)
}

/// 自定义标点映射的列号：中半 0 / 英全 1 / 中全 2 / 英半 3。
/// `chinese_punct` 须是**已扣除数字后智能**的有效值（见 `convert_punct`）。
pub fn punct_col_idx(chinese_punct: bool, full_width: bool) -> usize {
    match (chinese_punct, full_width) {
        (true, true) => 2,
        (true, false) => 0,
        (false, true) => 1,
        (false, false) => 3,
    }
}

/// 引号键在当前模式下的 **(左形, 右形)**：自定义映射的 `"1`/`"2`（`'1`/`'2`）两行即左形与
/// 右形，任一行缺值/空串则该侧回落内置中文引号。非引号键返回 None。
///
/// 这是「左右形」的**唯一真相源**——需要知道某个引号会出什么形态的调用方都必须问它，
/// 不能自己去查 `quote_pair`（内置形）：用户把引号自定义成 `「」` 后，判定按 `“”`、
/// 实际产出却按 `「」`，两处立刻错位。
/// 无状态：不看也不动引号交替态，因为左右是**按行**取的，不靠「第几次」推导。
pub fn quote_forms(
    punct: &PunctConfig,
    chinese_punct: bool,
    full_width: bool,
    c: char,
) -> Option<(String, String)> {
    let (def_left, def_right) = aurora_transform::punctuation::quote_pair(c)?;
    let (left_key, right_key) = aurora_transform::punctuation::quote_custom_keys(c)?;
    let col = punct_col_idx(chinese_punct, full_width);
    let pick = |key: &str, def: char| -> String {
        if !punct.custom_enabled {
            return def.to_string();
        }
        punct
            .custom_mappings
            .get(key)
            .and_then(|vals| vals.get(col))
            .filter(|v| !v.is_empty())
            .cloned()
            .unwrap_or_else(|| def.to_string())
    };
    Some((pick(left_key, def_left), pick(right_key, def_right)))
}

/// 「英文半角列有自定义覆盖」的源字符集合（去重、升序）。
///
/// 存在理由：英文输入模式（非全角）下 TSF 默认**直接透传**标点键，引擎根本收不到，于是四列
/// 里的「英半」成了打不到的死格（英全列有 `english_fullwidth` 分支吃键才得以生效）。core 把
/// 这个集合推给 DLL，DLL 只吃集合内的键并转发——**门控精确到字符**：用户没配的标点键行为
/// 完全不变（仍走 DLL 本地英文配对 + 透传）。
///
/// 与之配对的铁律：**C++ 吃键集必须 ⊆ Rust 出字集**。本函数同时是 DLL 吃键判据和
/// `Coordinator::handle_english_custom_punct` 的接手判据，同源即不会漂移；空串列（= 回落
/// 默认转换）不算覆盖，否则会吃下一个自己不出字的键。
///
/// 引号两行（`"1`/`"2`）折回同一个源字符 `"`，任一行有值即视为该键有覆盖。
pub fn custom_english_punct_chars(punct: &PunctConfig) -> Vec<char> {
    if !punct.custom_enabled {
        return Vec::new();
    }
    let mut out: Vec<char> = Vec::new();
    for (key, vals) in &punct.custom_mappings {
        if vals.get(3).is_none_or(|v| v.is_empty()) {
            continue; // 英半列无值 → 回落默认转换，不必吃键
        }
        let Some(src) = aurora_transform::punctuation::custom_key_source_char(key) else {
            continue;
        };
        if !out.contains(&src) {
            out.push(src);
        }
    }
    out.sort_unstable(); // HashMap 迭代序不稳定，排序保证推送字节可复现
    out
}

/// 纯查表读自定义标点映射的指定列（不碰转换器引号状态），供无副作用计算用。
/// 四状态列：中半 0 / 英全 1 / 中全 2 / 英半 3。
///
/// 键的生成一律走 [`PunctuationConverter::custom_key`]——此处曾自己按 `ch.to_string()` 拼键，
/// 于是引号（存储键是 `"1`/`"2`）在这条路上永远查不到自定义。
pub fn custom_lookup(
    conv: &PunctuationConverter,
    punct: &PunctConfig,
    ch: char,
    col_idx: usize,
) -> Option<String> {
    conv.peek_custom(punct, ch, col_idx)
}

/// 标点转换单点流水线（对齐 Go `convertPunct`）。`conv` 推进引号状态机故取 `&mut`。
pub fn convert_punct(
    conv: &mut PunctuationConverter,
    punct: &PunctConfig,
    chinese_punct: bool,
    full_width: bool,
    ch: char,
    prev_char: u16,
) -> String {
    let smart_en = chinese_punct && is_smart_punct_after_digit(punct, ch, prev_char);
    let is_chinese_punct = chinese_punct && !smart_en;

    // 1. 自定义映射优先（四状态均可配置）。开关与映射表同取自传入的**生效** `PunctConfig`
    //    （`lookup_custom` 内部判 `custom_enabled`，故此处不再重复一道开关）。
    let col_idx = punct_col_idx(is_chinese_punct, full_width);
    if let Some(text) = conv.lookup_custom(punct, ch, col_idx) {
        return text;
    }

    // 2~4. 默认转换：中文标点（含引号状态机）→ 全半角。
    let mut piece = ch.to_string();
    if is_chinese_punct && let Some(c) = conv.to_chinese(ch) {
        piece = c;
    }
    if full_width {
        piece = to_full_width(&piece);
    }
    piece
}

/// 中文输入模式下**产物就是原样半角 ASCII**、因而该让 DLL 直接透传（不吃键）的标点集合，
/// 去重升序。覆盖 Shift+数字的上挡符号与 OEM 标点键的两态。
///
/// # 为什么不能「吃下来再原样吐回去」
///
/// 吃了再吐要绕一圈经 CUAS 回到宿主，而非 TSF-aware 宿主普遍把这条路上送达的**字符码当
/// 虚拟键码**解释。Tkinter 实测（B-9，2026-09-18）：`#`(0x23)→`VK_END`、`%`(0x25)→`VK_LEFT`、
/// `&`(0x26)→`VK_UP`，字不上屏、反倒执行了一次光标移动；同批的 `@`(0x40)、`*`(0x2A) 只因撞上
/// 的 VK 在宿主里没有默认绑定才侥幸正常。微软拼音对这批符号根本不吃键，宿主拿到的 keycode
/// 仍是真实 VK（`VK_3`/`VK_5`/`VK_7`），因而不受影响 —— 本函数就是对齐那个行为。
///
/// # 判据与出字侧同源
///
/// 铁律同 [`custom_english_punct_chars`]：**吃键集必须 ⊆ 出字集**。这里反过来用：凡
/// [`convert_punct`] 的三步（自定义映射 → 中文标点表 → 全半角）里**任何一步会改写该字符**，
/// 就不能透传。第三步（全半角）由 DLL 侧判 `IsFullWidth()`，故本函数只管前两步。
///
/// # ⚠️ 本函数只管「标点转换层面」，按键占用由调用方再滤一道
///
/// 覆盖范围含 OEM 标点键，而那批键可能被配成**引导键**（`special:*` / `mix` / 临拼 / 临英）
/// 或**方案码元首码**——这两类在**空缓冲时也生效**，透传掉就是「那个模式再也进不去」
/// / 「那个方案再也打不出字」，且**不报错**。但它们属于按键绑定语义、数据在
/// `keys.key_actions` 与各方案码元集里，本 crate（只吃 `InputConfig`）看不见。
///
/// ⇒ 调用方**必须**在本函数结果上再减去那两类，见 `ConfigBundle` 里
/// `cn_passthrough_punct_chars` 的组装。翻页键 / 次选键 / 以词定字这些**只在有会话时**
/// 生效的绑定不必在此排除——两侧的透传闸门本就带 `!hasInputSession`。
/// 主键盘能打出的全部 ASCII 标点，与 `key_convert::punct_char` 的两列**逐字对应**
/// （那边答 VK+shift→字符，这边只要字符本身）。
/// 首行 = Shift+数字的上挡符号，后两行 = OEM 键的无 Shift / 有 Shift 两态。
///
/// 手工排版即文档：压成两行就再也对不上「首行数字、后两行 OEM」这句话了。
#[rustfmt::skip]
const PUNCT_SOURCES: [char; 32] = [
    ')', '!', '@', '#', '$', '%', '^', '&', '*', '(',
    '-', '_', '=', '+', '[', '{', ']', '}', '\\', '|',
    ';', ':', '\'', '"', ',', '<', '.', '>', '/', '?', '`', '~',
];

/// 某份自定义标点表**覆盖**了哪些标点源字符（任一列非空即算覆盖，去重升序）。
///
/// 「覆盖」= 该键在这份表下会被改写成别的东西 ⇒ 必须经引擎出字，不能透传。
/// 空串列是「回落默认转换」，不算覆盖（同 [`custom_english_punct_chars`] 的口径）。
///
/// # 为什么要单独成函数
///
/// 有两个消费者，判据必须同源：
/// 1. [`chinese_passthrough_punct_chars`] 判**全局**表；
/// 2. `ConfigBundle` 判**每个方案**的表——`effective_punct` 在方案声明了 `[punct]
///    custom_mappings` 时是**整表替换**（连 `custom_enabled` 一起换），全局那份完全不参与。
///    方案配的键漏减就是「被透传掉、方案里配的那一格静默失效」，零日志。
///
/// ⚠️ 查表走 [`custom_lookup`]（内部按 `PunctuationConverter::custom_key` 生成键），
/// **不要**自己遍历 `custom_mappings` 的键：引号的存储键是 `"1`/`"2`、`'1`/`'2`，
/// 手写遍历拿到的是存储键、折不回源字符，方案自定义的引号就会被漏减。
pub fn custom_covered_punct_chars(conv: &PunctuationConverter, punct: &PunctConfig) -> Vec<char> {
    PUNCT_SOURCES
        .iter()
        .copied()
        .filter(|&ch| {
            (0..4).any(|col| custom_lookup(conv, punct, ch, col).is_some_and(|v| !v.is_empty()))
        })
        .collect()
}

/// 空格行哪几列配了自定义映射：第 i 位 = 列 i（[`punct_col_idx`] 的列号）非空。
///
/// 中文模式空缓冲时 DLL 默认透传空格（保留网页滚动等宿主语义），只在全角态吃；半角列配了
/// 映射就得据此多吃，否则按 `OnTestKeyDown` 结论走的宿主（Notepad++、WPS）直接收到半角
/// 空格，而照调 `OnKeyDown` 的 Chrome 类宿主却正常（t337）。查表同走 [`custom_lookup`]。
pub fn custom_space_cols(conv: &PunctuationConverter, punct: &PunctConfig) -> u8 {
    (0..4)
        .filter(|&col| custom_lookup(conv, punct, ' ', col).is_some())
        .fold(0, |mask, col| mask | (1 << col))
}

/// # 只管**中文标点态**
///
/// 判据第 1 步问的是中文标点表，故结论只在中文标点态下成立。英文标点态（中文输入模式下
/// 也能切）另有一份更大的集合，见 [`english_passthrough_punct_chars`]——那个态不走中文
/// 标点表，`,` `.` `;` 这些的产物同样是原样 ASCII，撞码还更凶（`.`→`VK_DELETE` 吞字符、
/// `[`→`VK_LWIN` 弹开始菜单）。两个集合都要推给宿主，由宿主按当下标点态选用。
pub fn chinese_passthrough_punct_chars(
    conv: &PunctuationConverter,
    cfg: &InputConfig,
) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for ch in PUNCT_SOURCES {
        // 1. 中文标点表有映射（`!`→！、`$`→￥、`^`→……、`(`→（、`)`→））⇒ 要转换，必须吃。
        if conv.peek_chinese_str(ch).is_some() {
            continue;
        }
        // 2. 自定义映射**任一列**有值 ⇒ 要改写，必须吃。判据抽在
        //    [`custom_covered_punct_chars`]，与 `ConfigBundle` 判**方案级**表的那份同源
        //    ——方案声明了 `[punct] custom_mappings` 时是整表替换，全局这份完全不参与。
        //    只看中半列不够：中文输入模式下还能切到英文标点态（走英半列），那时这个键若已被
        //    透传，用户配的那一列就成了打不到的死格。空串列 = 回落默认转换，不算覆盖。
        if custom_covered_punct_chars(conv, &cfg.punct).contains(&ch) {
            continue;
        }
        out.push(ch);
    }
    out.sort_unstable(); // 推送字节可复现（同 custom_english_punct_chars）
    out
}

/// **英文标点态**（中文输入模式 + 标点切英文）下产物就是原样半角 ASCII、因而该透传的标点
/// 集合，去重升序。[`chinese_passthrough_punct_chars`] 的姊妹，成因与失效方向完全相同，
/// 差别只在「什么算会被改写」。
///
/// # 为什么必须单独一份，不能给中文那份加个条件
///
/// 英文标点态下 [`convert_punct`] 根本不走中文标点表（`is_chinese_punct == false`），
/// 于是 `,` `.` `;` `'` `[` `]` `\` 这些**在中文态必须吃**的键，在这个态下产物就是原样
/// ASCII。所以这份集合**通常**比中文那份更大 —— 给现有集合叠 `&&` 只会让它更小，
/// 方向正好反了。
///
/// ⚠️ 「超集」是出厂配置下的观察，**不是不变量**：两态的排除项来源不同（中文态看中文
/// 标点表与自定义覆盖，英文态只看英半列覆盖），用户把某个无中文映射、却有英半列自定义的
/// 字符（如把 `@` 的英半列配成别的形态）时，它会变成「中文能透、英文不能」——那是
/// **正确**行为，不是判据写反。
///
/// 撞码在这个态下同样成立，而且比中文态那批更凶：`.`(0x2E)→`VK_DELETE`（**吞掉光标后
/// 一个字符**）、`[`(0x5B)→`VK_LWIN`（弹开始菜单）、`'`(0x27)→`VK_RIGHT`、`;`(0x3B)→`VK_F1`。
///
/// # 判据
///
/// 英文半角列（col 3）是这个态唯一会改写字符的来源，故复用
/// [`custom_english_punct_chars`]——那正是「英半列有覆盖」的判据，与 DLL 英文模式吃键
/// 共用同一份，天然同源。全角由调用方的动态闸门挡（全角走 col 1，不在本函数职责内）。
///
/// ⚠️ 与姊妹函数同样的职责边界：**按键占用层（引导键 / 方案码元首码）不在此排除**，
/// 由调用方再滤一道，见 `ConfigBundle` 里的组装。
pub fn english_passthrough_punct_chars(cfg: &InputConfig) -> Vec<char> {
    let en_custom = custom_english_punct_chars(&cfg.punct);
    let mut out: Vec<char> = Vec::new();
    for ch in PUNCT_SOURCES {
        // 1. 英半列配了自定义 ⇒ 要出用户配的值，必须吃。
        if en_custom.contains(&ch) {
            continue;
        }
        out.push(ch);
    }
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> InputConfig {
        InputConfig::default()
    }

    #[test]
    fn smart_punct_after_digit_default_list() {
        let c = cfg(); // 默认 smart_punct_after_digit=true, list=".,:"
        // '.' 在列表 + 前字符是数字 '5'(0x35) → true
        assert!(is_smart_punct_after_digit(&c.punct, '.', 0x35));
        // 前字符非数字 → false
        assert!(!is_smart_punct_after_digit(&c.punct, '.', b'a' as u16));
        // 不在列表的标点 → false
        assert!(!is_smart_punct_after_digit(&c.punct, '!', 0x35));
    }

    #[test]
    fn convert_punct_chinese_and_fullwidth() {
        let mut conv = PunctuationConverter::new();
        let c = cfg();
        // 中文标点模式：'.' → '。'
        assert_eq!(
            convert_punct(&mut conv, &c.punct, true, false, '.', 0),
            "。"
        );
        // 英文标点模式 + 全角：'.' 走全半角 → '．'
        let out = convert_punct(&mut conv, &c.punct, false, true, '.', 0);
        assert_ne!(out, "."); // 全角化
    }

    #[test]
    fn convert_punct_smart_digit_forces_english() {
        let mut conv = PunctuationConverter::new();
        let c = cfg();
        // 中文模式但前字符是数字 → '.' 按英文输出（不转 '。'）。
        assert_eq!(
            convert_punct(&mut conv, &c.punct, true, false, '.', 0x33),
            "."
        );
    }

    #[test]
    fn custom_space_cols_marks_mapped_columns() {
        let conv = PunctuationConverter::new();
        let mut c = cfg();
        assert_eq!(custom_space_cols(&conv, &c.punct), 0, "出厂无映射");
        c.punct.custom_enabled = true;
        // 列序：0=中半 1=英全 2=中全 3=英半；空串 = 回落默认，不算配了。
        c.punct.custom_mappings.insert(
            " ".into(),
            vec!["\u{3000}".into(), "".into(), "\u{3000}".into(), "".into()],
        );
        assert_eq!(custom_space_cols(&conv, &c.punct), 0b0101);
        c.punct.custom_enabled = false;
        assert_eq!(custom_space_cols(&conv, &c.punct), 0, "总开关关着不算");
    }

    #[test]
    fn custom_lookup_empty_is_none() {
        let conv = PunctuationConverter::new();
        let c = cfg(); // 默认无自定义映射
        assert_eq!(custom_lookup(&conv, &c.punct, '.', 0), None);
    }

    /// 回归锁（根因）：自定义映射来自**实时配置**，不是转换器里的启动快照。
    /// 同一个 conv 实例，配置里加上映射后下一次转换即生效——这正是「设置页改自定义标点
    /// 必须重启服务才生效」的病灶：曾把表存进转换器且只在 `Coordinator::new` 注入一次。
    #[test]
    fn convert_punct_follows_live_custom_mappings() {
        let mut conv = PunctuationConverter::new();
        let mut c = cfg();
        // 出厂：中文标点模式下 '"' 走内置引号交替 → 左引号。
        assert_eq!(
            convert_punct(&mut conv, &c.punct, true, false, '"', 0),
            "\u{201C}"
        );
        conv.reset();

        // 用户在设置页配了双引号第一次/第二次（中文半角列）。
        c.punct.custom_enabled = true;
        c.punct
            .custom_mappings
            .insert("\"1".into(), vec!["「".into()]);
        c.punct
            .custom_mappings
            .insert("\"2".into(), vec!["」".into()]);
        assert_eq!(
            convert_punct(&mut conv, &c.punct, true, false, '"', 0),
            "「",
            "热重载后第一次应立即出自定义值"
        );
        assert_eq!(
            convert_punct(&mut conv, &c.punct, true, false, '"', 0),
            "」",
            "第二次应出「第二次」那一行"
        );
    }

    /// 推给 DLL 的吃键集合：只含「英半列非空」的行的源字符，且引号两行折回同一字符。
    /// 这个集合同时是 DLL 的吃键判据和 core 的出字判据，多一个字符就是一次丢键。
    #[test]
    fn custom_english_punct_chars_only_covered_keys() {
        let mut c = cfg();
        c.punct.custom_enabled = true;
        // 引号两行都配了英半列 → 折回一个 '"'
        c.punct.custom_mappings.insert(
            "\"1".into(),
            vec!["E".into(), "".into(), "".into(), "#".into()],
        );
        c.punct.custom_mappings.insert(
            "\"2".into(),
            vec!["￥".into(), "".into(), "".into(), "$".into()],
        );
        // 只配了中半列 → 英文模式无需吃键
        c.punct
            .custom_mappings
            .insert("/".into(), vec!["、".into()]);
        // 英半列是空串（回落默认）→ 同样不该吃
        c.punct.custom_mappings.insert(
            ";".into(),
            vec!["；".into(), "".into(), "".into(), "".into()],
        );
        // 单引号只配英半列
        c.punct.custom_mappings.insert(
            "'1".into(),
            vec!["".into(), "".into(), "".into(), "@".into()],
        );
        assert_eq!(custom_english_punct_chars(&c.punct), vec!['"', '\'']);

        // 总开关关掉 → 空集合（DLL 恢复历史行为）
        c.punct.custom_enabled = false;
        assert!(custom_english_punct_chars(&c.punct).is_empty());
    }

    /// `"1`/`"2` 两行 = 左形/右形：配对判定与插入都从这里取，两行都用得上
    /// （曾只按「第几次」取用，配对钉左后第二行永远取不到）。
    #[test]
    fn quote_forms_maps_two_rows_to_left_and_right() {
        let mut c = cfg();
        // 未自定义：回落内置中文引号。
        assert_eq!(
            quote_forms(&c.punct, true, false, '"'),
            Some(("\u{201C}".into(), "\u{201D}".into()))
        );
        // 两行齐：左形取 "1、右形取 "2。
        c.punct.custom_enabled = true;
        c.punct
            .custom_mappings
            .insert("\"1".into(), vec!["「".into()]);
        c.punct
            .custom_mappings
            .insert("\"2".into(), vec!["」".into()]);
        assert_eq!(
            quote_forms(&c.punct, true, false, '"'),
            Some(("「".into(), "」".into()))
        );
        // 只配左形：右侧回落内置（不会跟着变）。
        c.punct.custom_mappings.remove("\"2");
        assert_eq!(
            quote_forms(&c.punct, true, false, '"'),
            Some(("「".into(), "\u{201D}".into()))
        );
        // 非引号键无左右形。
        assert_eq!(quote_forms(&c.punct, true, false, ','), None);
        // 列随模式走：中文全角取第 2 列。
        c.punct
            .custom_mappings
            .insert("\"1".into(), vec!["「".into(), "x".into(), "『".into()]);
        assert_eq!(
            quote_forms(&c.punct, true, true, '"').map(|(l, _)| l),
            Some("『".into())
        );
    }

    // ── chinese_passthrough_punct_chars ────────────────────────────────────────
    //
    // 判据的真值表锁在这里。漏掉任一条排除项的现场表现都是「某个键忽然不经引擎了」，
    // 且不报错 —— 靠真机复现的成本远高于这几条断言。

    #[test]
    fn passthrough_default_is_the_unmapped_punct_set() {
        let conv = PunctuationConverter::new();
        let c = cfg();
        // 默认配置下「标点层面产物不变」的全集，恰是中文标点表没有映射的那十个。
        // 这张表同时是撞码风险的全集（ASCII 码点 ≤0xFF 才可能撞 VK）：
        //   `#`→VK_END  `%`→VK_LEFT  `&`→VK_UP  `-`→VK_INSERT  `/`→VK_HELP  `|`→VK_F13
        //   `@` `=` 落在未分配 VK 上，`*`→VK_PRINT、`+`→VK_EXECUTE 无默认绑定，侥幸无害。
        //
        // ⚠️ 这不是最终放行集：`-` `=` `/` 这些还可能被配成引导键或方案码元首码，
        // 由 `ConfigBundle` 再滤一道（见本函数文档的职责边界说明）。
        assert_eq!(
            chinese_passthrough_punct_chars(&conv, &c),
            vec!['#', '%', '&', '*', '+', '-', '/', '=', '@', '|']
        );
    }

    #[test]
    fn english_passthrough_is_superset_of_chinese() {
        // 出厂配置下英文态是中文态的超集：英文态不走中文标点表，中文态能透传的它也能。
        // 反过来不成立——`,` `.` `;` 这些在中文态要转、在英文态却是原样。
        //
        // ⚠️ 只在**出厂配置**下成立，**不是不变量**：两态的排除项来源不同（中文态看中文
        // 标点表与自定义覆盖，英文态只看英半列覆盖），给某个无中文映射的字符配英半列
        // 自定义后就会出现「中文能透、英文不能」——别把本条误当铁律去「修」判据。
        let conv = PunctuationConverter::new();
        let c = cfg();
        let cn = chinese_passthrough_punct_chars(&conv, &c);
        let en = english_passthrough_punct_chars(&c);
        for ch in &cn {
            assert!(en.contains(ch), "`{ch}` 中文态能透传，英文态没有理由不能");
        }
        assert!(
            en.len() > cn.len(),
            "出厂配置下英文态该是真超集，不该只是相等"
        );
    }

    #[test]
    fn english_passthrough_default_set() {
        let en = english_passthrough_punct_chars(&cfg());
        // ★ `.` 是这批里最危险的一个（撞 `VK_DELETE`，会吞掉光标后一个字符）。智能符号
        //   智能符号功能在时它一度被排除在透传之外；功能删除后
        //   这条判据彻底不在了，这批键必须全部照常透传。
        for ch in ['.', ',', '?', '!', ':', ';'] {
            assert!(en.contains(&ch), "`{ch}` 产物即原样，应透传");
        }
        // `(`(0x28) 撞 `VK_DOWN`、`)`(0x29) 撞 `VK_SELECT`：没有自动配对之后它们同样
        // 产物即原样，必须透传。
        for ch in ['(', ')', '[', ']', '{', '}'] {
            assert!(en.contains(&ch), "`{ch}` 产物即原样，应透传");
        }
        // 中文态里被中文标点表拦下、英文态却该放行的那批，逐个点名。
        for ch in ['\'', '"', '\\', '`', '~', '$', '^', '_'] {
            assert!(en.contains(&ch), "`{ch}` 在英文半角态产物即原样，应透传");
        }
    }

    #[test]
    fn english_passthrough_excludes_en_half_custom() {
        // 英半列配了自定义 ⇒ 要出用户配的值，必须吃。判据复用
        // `custom_english_punct_chars`，与 DLL 英文模式吃键同源。
        let mut c = cfg();
        c.punct.custom_enabled = true;
        c.punct.custom_mappings.insert(
            "/".into(),
            vec![String::new(), String::new(), String::new(), "÷".into()],
        );
        assert!(!english_passthrough_punct_chars(&c).contains(&'/'));
    }

    #[test]
    fn passthrough_excludes_quotes_via_state_machine() {
        // 引号不在 `static_chinese` 表里、走交替状态机，但 `peek_chinese_str` 覆盖了它们
        // （返回 `‘` / `“`），故判据天然把它们排除。漏掉这条的后果是引号被透传、
        // 中文引号再也打不出来。
        let conv = PunctuationConverter::new();
        let got = chinese_passthrough_punct_chars(&conv, &cfg());
        assert!(!got.contains(&'\''), "单引号要转中文引号，不得透传");
        assert!(!got.contains(&'"'), "双引号要转中文引号，不得透传");
    }

    #[test]
    fn passthrough_excludes_chinese_mapped_symbols() {
        let conv = PunctuationConverter::new();
        let c = cfg();
        let got = chinese_passthrough_punct_chars(&conv, &c);
        for ch in ['!', '$', '^', '(', ')'] {
            assert!(!got.contains(&ch), "{ch} 有中文标点映射，不得透传");
        }
    }

    #[test]
    fn passthrough_excludes_any_custom_mapping_column() {
        // 中半列配了 → 必须吃（要出用户配的值）。
        let conv = PunctuationConverter::new();
        let mut c = cfg();
        c.punct.custom_enabled = true;
        c.punct.custom_mappings.insert(
            "#".into(),
            vec!["井".into(), String::new(), String::new(), String::new()],
        );
        assert!(!chinese_passthrough_punct_chars(&conv, &c).contains(&'#'));

        // ★ 只配了**英半列**也必须吃：中文输入模式下还能切到英文标点态走那一列，
        //   透传掉的话用户配的那格就永远打不出来。这条是「只看中半列」会漏掉的。
        let mut c2 = cfg();
        c2.punct.custom_enabled = true;
        c2.punct.custom_mappings.insert(
            "%".into(),
            vec![String::new(), String::new(), String::new(), "pct".into()],
        );
        assert!(!chinese_passthrough_punct_chars(&conv, &c2).contains(&'%'));
    }

    #[test]
    fn passthrough_empty_custom_column_is_not_coverage() {
        // 四列全是空串 = 回落默认转换，不算覆盖，仍可透传。
        let conv = PunctuationConverter::new();
        let mut c = cfg();
        c.punct.custom_enabled = true;
        c.punct.custom_mappings.insert(
            "#".into(),
            vec![String::new(), String::new(), String::new(), String::new()],
        );
        assert!(chinese_passthrough_punct_chars(&conv, &c).contains(&'#'));
    }
}
