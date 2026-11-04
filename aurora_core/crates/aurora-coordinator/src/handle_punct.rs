//! 标点编排
//!
//! 从 coordinator.rs 拆出（同 crate 内 `impl Coordinator` 块，组织性重构，无逻辑变更）。
//! 纯转换逻辑在 aurora-punct crate；此处是 coordinator 包装（锁转换器/读状态）+
//! 英文模式下的标点吃键路径。

use crate::config_bundle::ConfigBundle;
use crate::coordinator::{Coordinator, State, full_width_source_char, numpad_char, punct_char};
use aurora_bridge::handler::{KeyAction, KeyEventData, MessageHandler};
use aurora_ipc::protocol::MOD_SHIFT;
use aurora_keys::keymap;
use tracing::debug;

impl Coordinator {
    /// 当前输入语境下**生效**的标点配置——`aurora_punct::*` 一切读 `punct.*` 的入口都从这里取。
    ///
    /// # 归属方案：`effective_data_schema`，不是活跃方案
    ///
    /// 自定义映射表回答的是「这个键出什么字」，是**数据**，与 `[phrases]` 同源：临英归
    /// `english` 桶、快符归快符方案，两者的活跃方案都还是主方案（五笔/拼音）。按活跃方案取
    /// 会让快符方案配的符号表永远不生效——而特殊方案本就是「该有自己一套」的那一类。
    ///
    /// ⚠️ 同段的 `[punct] mode` 走的是 `active_behavior()`（活跃方案），**刻意不同**，
    /// 理由见 [`aurora_config::PunctSpec`] 的字段对照表。别看见同一个段就统一成一个判据。
    ///
    /// # 整表替换：连 `custom_enabled` 一起换
    ///
    /// 方案声明了表 ⇒ 全局表整份不参与，全局的总开关也随之失效（`Some({})` = 本方案不用任何
    /// 自定义映射）。`smart_after_digit` / `smart_list` 不在下放范围，故仍是全局值原样带过来。
    ///
    /// 返回 `Cow`：没有方案表时零拷贝借用全局那份（绝大多数用户、每一次标点按键），
    /// 只有真配了方案表才 clone 一次。
    pub(crate) fn effective_punct<'a>(
        &self,
        bundle: &'a ConfigBundle,
        state: &State,
    ) -> std::borrow::Cow<'a, aurora_config::config::PunctConfig> {
        let global = &bundle.config.input.punct;
        let behavior = match self.effective_data_schema(state) {
            Some(id) => self.engine_mgr.behavior_for(&id),
            None => self.engine_mgr.active_behavior(),
        };
        match &behavior.punct_custom_mappings {
            None => std::borrow::Cow::Borrowed(global),
            Some(table) => {
                let mut p = global.clone();
                p.custom_enabled = !table.is_empty();
                p.custom_mappings = table.clone();
                std::borrow::Cow::Owned(p)
            }
        }
    }

    /// 小键盘恒半角（`input.numpad_half_width`）对**本次按键**是否生效。
    ///
    /// 为真时调用方必须**原样输出 ASCII**：跳过整条标点流水线（自定义映射 / 中文标点 /
    /// 全半角三步全不走），而不是只跳过全半角那一步。用户拍板的语义是「小键盘就是数字
    /// 键盘」，`direct` 与 `follow_main` 两档在此统一——此前同一个小键盘 `.`，direct 出
    /// `．`、follow_main 出 `。`。
    ///
    /// ⚠️ **凡是「小键盘字符直接上屏」的路径都必须问它**，漏接一处的症状是「某个模式下
    /// 开关静默无效、零日志」。现有调用点：本文件 `convert_punct`（流水线总口）、
    /// `commit_highlight_then_char`（普通模式 direct）、临拼 numpad 臂、数字键越界
    /// `commit_and_input`。缓冲类模式（临英 / URL / mix）**刻意不接**，见
    /// `InputConfig::numpad_half_width` 的能力边界说明。
    pub(crate) fn numpad_raw_output(&self, state: &State) -> bool {
        state.numpad_origin && self.rt().config.input.numpad_half_width
    }

    /// 按当前中英标点/全半角配置转换一个标点字符为上屏文本（无 prev_char 上下文）。
    /// 用于独占模式（快捷输入/临时英文）等不涉及数字后智能的场景。
    pub(crate) fn convert_punct_char(&self, state: &State, ch: char) -> String {
        self.convert_punct(state, ch, 0)
    }

    /// 空缓冲的空格 DLL 吃没吃：全角态恒吃；中文模式下还看空格行当前列配没配映射
    /// （`CONFIG_KEY_SPACE_CUSTOM_COLS`，与 DLL 同一份掩码）。
    pub(crate) fn idle_space_eaten(&self, state: &State) -> bool {
        let col = aurora_punct::punct_col_idx(state.chinese_punct, state.full_width);
        state.full_width || (state.chinese_mode && self.rt().space_custom_cols & (1 << col) != 0)
    }

    /// 标点转换单点流水线（对齐 Go `convertPunct`，固定优先级）：
    ///   1. 自定义映射（四状态：中半 0 / 英全 1 / 中全 2 / 英半 3，按当前中英标点+全半角选列）
    ///   2. 数字后智能转换（命中则该标点按英文输出，不转中文）
    ///   3. 中文标点转换（引号左右交替状态机）
    ///   4. 全半角转换
    ///
    /// `prev_char` 为光标前一字符的 UTF-16 单元（0=不可用），用于数字后智能判定。
    pub(crate) fn convert_punct(&self, state: &State, ch: char, prev_char: u16) -> String {
        // 小键盘恒半角：整条流水线跳过（不只是第 4 步）。`ch` 进来时已是 ASCII 原形，
        // 故原样返回即可，无须回推字符。这是本开关最主要的一处收口——英文全角、
        // CapsLock 全角、follow_main 归一化后的数字/标点臂、临英的 convert_punct_char
        // 全都汇聚于此。
        if self.numpad_raw_output(state) {
            return ch.to_string();
        }
        let bundle = self.rt();
        let punct = self.effective_punct(&bundle, state);
        let mut conv = self.punct.lock().unwrap_or_else(|e| e.into_inner());
        aurora_punct::convert_punct(
            &mut conv,
            &punct,
            state.chinese_punct,
            state.full_width,
            ch,
            prev_char,
        )
    }

    /// 英文模式 + 全角：按键经完整标点流水线转全角后上屏。
    ///
    /// **必须出字**：这些键已被 TSF 在 `OnTestKeyDown` 的 `english_fullwidth` 分支吃下
    /// （Letter|Number|Punctuation|Space，含小键盘）。此处返回 None → 调用方 PassThrough →
    /// 形成 `OnTestKeyDown(TRUE)+OnKeyDown(FALSE)` 的「吃了再吐」翻转，而 Chrome/Electron 等
    /// 严格 TSF 宿主不会回退合成 WM_CHAR，键会直接丢失（半角宿主则表现为出半角）。
    /// 故此处接住的键集必须覆盖 C++ 的吃键集，二者须同增同减。
    pub(crate) fn handle_english_full_width(
        &self,
        state: &mut State,
        data: &KeyEventData,
    ) -> Option<KeyAction> {
        let shift = data.modifiers & MOD_SHIFT != 0;
        let is_letter = (keymap::VK_A..=keymap::VK_Z).contains(&data.key_code);
        // 键被吃下后系统不再代劳大小写，故由 CapsLock 镜像 XOR Shift 定。镜像每键都用事件
        // 携带的 toggles 快照校准（见 handle_key_event 开头），英文模式下同样可靠。
        let effective_shift = if is_letter {
            shift != state.caps_lock
        } else {
            shift
        };
        // 覆盖面须 ⊇ C++ 全角吃键集（含空格/小键盘），由 full_width_source_char 统一收口。
        let ch = full_width_source_char(data.key_code, effective_shift)?;

        // 「英全」= 英文标点 + 全角（自定义映射四态的列 1）。经完整流水线而非裸 to_full_width，
        // 确保用户自定义中英文符号生效（与 CapsLock+全角 路径同构）。
        let saved_punct = state.chinese_punct;
        state.chinese_punct = false;
        let piece = self.convert_punct_char(state, ch);
        state.chinese_punct = saved_punct;

        // 统计：C++ `OnKeyTraceDown` 在全角态主动跳过计数（注释「will be eaten by
        // OnTestKeyDown for full-width conversion」），把英文统计让给本路径；不记则恒为 0。
        self.record_english_key_stat(data.key_code, shift);

        debug!("English full-width: {:?} -> {:?}", ch, piece);

        Some(Self::commit_action(piece, false))
    }

    /// 英文模式 + 半角：只接手「英半列有自定义映射覆盖」的标点键，按英半列（col 3）出字。
    ///
    /// 为什么需要这条分支：英文模式非全角时 TSF 默认**直接透传**标点键，引擎收不到，四列里的
    /// 「英半」因此是打不到的死格（真机日志指纹：`decision=passthrough_not_handled`）。现由
    /// core 把「哪些源字符配了英半列」推给 DLL，DLL 精确吃下这些键转发过来，此处出字。
    ///
    /// **必须出字**（同 `handle_english_full_width` 的铁律）：接手判据与推给 DLL 的集合同源
    /// （`custom_english_punct_chars`），集合内即必有非空英半列值，故 `convert_punct` 必然命中
    /// 自定义、必然返回该值。返回 None 只发生在「键不在集合里」——那 DLL 本来就没吃它。
    ///
    /// 引号在此照常按左右形交替（`"1` → `"2` → `"1`…），交替态与中文侧共用同一个转换器，
    /// 中英模式切换时由 `punct.reset()` 归零（`coordinator.rs` 的两处 mode_switch）。
    pub(crate) fn handle_english_custom_punct(
        &self,
        state: &mut State,
        data: &KeyEventData,
    ) -> Option<KeyAction> {
        let shift = data.modifiers & MOD_SHIFT != 0;
        let ch = punct_char(data.key_code, shift)?;
        // 判据必须与 DLL 的吃键判据**逐条同源**，漂移即「吃了再吐」丢键。DLL 只吃一类键：
        // 英半列配了自定义映射的源字符（`push_custom_en_punct_config` 推送的集合）；集合内
        // 必有非空英半列值 ⇒ 必然出字，故沿用它同时作接手判据。
        let rt = self.rt();
        if !rt.custom_en_punct_chars.contains(&ch) {
            return None; // DLL 未吃此键，透传给宿主
        }

        // 「英半」= 英文标点 + 半角（列 3）。英文模式恒按英文标点列输出，与工具栏的
        // 中/英标点开关无关——这与改造前「DLL 透传、宿主出 ASCII」的既有语义一致。
        let saved_punct = state.chinese_punct;
        state.chinese_punct = false;
        let piece = self.convert_punct_char(state, ch);
        state.chinese_punct = saved_punct;

        // 统计：与全角路径同理，键被吃下后 C++ 的英文计数不再经过原路径，此处补记。
        self.record_english_key_stat(data.key_code, shift);
        debug!("English custom punct: {:?} -> {:?}", ch, piece);

        Some(Self::commit_action(piece, false))
    }

    /// 英文模式按键的统计归类：**镜像** C++ `_RecordEnglishKeyTrace` 的分桶（Shift+数字算标点、
    /// 小键盘数字算数字…），保证全角/半角两条路径统计口径一致。
    fn record_english_key_stat(&self, key_code: u32, shift: bool) {
        let (chars, digits, puncts, spaces) = if (keymap::VK_A..=keymap::VK_Z).contains(&key_code) {
            (1, 0, 0, 0)
        } else if (keymap::VK_0..=keymap::VK_9).contains(&key_code) {
            if shift { (0, 0, 1, 0) } else { (0, 1, 0, 0) }
        } else if key_code == keymap::VK_SPACE {
            (0, 0, 0, 1)
        } else if numpad_char(key_code).is_some_and(|c| c.is_ascii_digit()) {
            (0, 1, 0, 0)
        } else {
            (0, 0, 1, 0)
        };
        self.handle_english_stats(chars, digits, puncts, spaces);
    }

    /// `ime.pair(left, right, jump=N)`：上屏 `left+right`。
    ///
    /// # 现在为什么只剩「纯上屏」
    ///
    /// 自动配对功能已整块删除（配对栈、跳出键、`_pairPendingDepth` 全没了）。本命令是命令
    /// 直通车里**用户可见的公开动作**（`ime.pair`），删它会破坏已写好的词条，故保留入口，
    /// 但语义退化为「把两段文本原样上屏」——**刻意不发带光标偏移的提交**：那条推送会让 C++
    /// 递增它的 `_pairPendingDepth`，而 core 已不再弹栈，那个计数会永久 >0，把中文模式下的
    /// Tab/Enter 从协调器手里截走（症状是「上屏后回车没反应」）。宁可光标落在末尾。
    pub(crate) fn cmd_pair_commit(&self, left: &str, right: &str, _jump_steps: u32) {
        if left.is_empty() && right.is_empty() {
            return;
        }
        self.push_commit_text(&format!("{left}{right}"));
    }
}

/// 方案级自定义标点表（`[punct.custom_mappings]`）：整表替换 + 按数据归属方案取。
#[cfg(test)]
mod schema_scoped_punct_tests {
    use super::*;
    use crate::pipeline::ModeKind;
    use aurora_config::config::Config;
    use std::sync::Arc;

    /// 三个方案的临时 data 目录：
    /// - `zzp_own`：自带表（`.`→`·`；`;` 的**英半列**→`#`）
    /// - `zzp_follow`：不写 `[punct]` ⇒ 跟随全局
    /// - `english`：临英的数据归属方案，自带**另一张**表（`.`→`!`）
    ///
    /// 三张表在同一个源字符 `.` 上取值**互不相同**——这是本组用例能测出差别的前提。
    /// 若让它们碰巧同值，「取错了方案」和「取对了」会给出一样的断言结果。
    fn data_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wind_punct_scope_{}_{tag}", std::process::id()));
        let schemas = dir.join("schemas");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&schemas).unwrap();
        let write = |id: &str, punct: &str| {
            std::fs::write(
                schemas.join(format!("{id}.schema.toml")),
                format!("[schema]\nid = \"{id}\"\n[engine]\ntype = \"codetable\"\n{punct}"),
            )
            .unwrap();
        };
        write(
            "zzp_own",
            "[punct.custom_mappings]\n\".\" = [\"·\", \"\", \"\", \"\"]\n\
             \";\" = [\"；\", \"\", \"\", \"#\"]\n",
        );
        write("zzp_follow", "");
        write(
            "english",
            "[punct.custom_mappings]\n\".\" = [\"!\", \"\", \"\", \"\"]\n",
        );
        dir
    }

    /// 全局表里有一行 `,`→`G`，且与三个方案表的键**不重叠**——于是「全局那行还在不在」
    /// 就直接回答了「整表替换有没有真的整表」。
    fn coord_at(dir: &std::path::Path, active: &str) -> Arc<Coordinator> {
        let mut cfg = Config::default();
        cfg.schema.active = active.to_string();
        cfg.schema.available = vec!["zzp_own".into(), "zzp_follow".into()];
        cfg.input.punct.custom_enabled = true;
        cfg.input.punct.custom_mappings.insert(
            ",".into(),
            vec!["G".into(), "".into(), "".into(), "".into()],
        );
        Coordinator::new_headless(cfg, Some(dir))
    }

    /// 取当前语境下生效表里某个源字符的中半列（col 0）值。
    fn cn_half(c: &Coordinator, mode: Option<ModeKind>, ch: char) -> Option<String> {
        {
            let mut st = c.state.lock().unwrap();
            st.active = mode;
        }
        let bundle = c.rt();
        let state = c.state.lock().unwrap();
        let punct = c.effective_punct(&bundle, &state);
        let conv = c.punct.lock().unwrap();
        conv.peek_custom(&punct, ch, 0)
    }

    /// ★ 方案声明了表 ⇒ 全局表**整份**不参与。
    ///
    /// 两条断言缺一不可：只断言「方案的行生效」的话，逐键合并的实现照样通过——
    /// 真正区分两种模型的是**全局那行还在不在**。
    #[test]
    fn schema_table_replaces_global_table_wholesale() {
        let dir = data_dir("own");
        let c = coord_at(&dir, "zzp_own");

        assert_eq!(
            cn_half(&c, None, '.'),
            Some("·".into()),
            "方案自己的行要生效"
        );
        assert_eq!(
            cn_half(&c, None, ','),
            None,
            "★ 全局的 `,` 那行必须整份不参与——逐键合并时它会残留"
        );

        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 方案不写这一段 ⇒ 全走全局（含全局的 `custom_enabled`）。
    #[test]
    fn schema_without_table_follows_global() {
        let dir = data_dir("follow");
        let c = coord_at(&dir, "zzp_follow");

        assert_eq!(cn_half(&c, None, ','), Some("G".into()), "跟随全局那张表");
        assert_eq!(cn_half(&c, None, '.'), None, "别的方案的表不该漏过来");

        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★★ 归属方案是 `effective_data_schema` 而**不是**活跃方案。
    ///
    /// 活跃方案是 `zzp_own`（`.`→`·`），进临英后必须换成 `english` 方案那张（`.`→`!`）。
    /// 三张表在 `.` 上各不相同，故「取错方案」与「取对」给出的值互不相同——若让 english
    /// 也写 `·`，把实现改成按活跃方案取，这条照样绿。
    #[test]
    fn temp_english_takes_english_schema_table() {
        let dir = data_dir("temp");
        let c = coord_at(&dir, "zzp_own");

        assert_eq!(
            cn_half(&c, None, '.'),
            Some("·".into()),
            "主输入路取活跃方案"
        );
        assert_eq!(
            cn_half(&c, Some(ModeKind::TempEnglish), '.'),
            Some("!".into()),
            "★ 临英必须取 english 方案的表——按活跃方案取会拿到 `·`"
        );

        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★★ 推给 DLL 的吃键集是**跨方案并集**，不随活跃方案变。
    ///
    /// `;` 的英半列只有 `zzp_own` 配了，而这里的活跃方案是 `zzp_follow`。按活跃方案裁剪
    /// 的话它不在集合里 ⇒ 切到 `zzp_own` 后 DLL 手里还是旧表（`CONFIG_KEY_CUSTOM_EN_PUNCT`
    /// 只在握手与热重载时推），表现为「刚切完方案这个键不灵」。
    #[test]
    fn en_punct_eat_set_is_cross_schema_union() {
        let dir = data_dir("union");
        let c = coord_at(&dir, "zzp_follow");

        assert!(
            c.rt().custom_en_punct_chars.contains(&';'),
            "★ 别的方案配的英半列源字符也必须在吃键集里（并集），实际: {:?}",
            c.rt().custom_en_punct_chars
        );

        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
