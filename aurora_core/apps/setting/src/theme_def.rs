//! 设置窗口的亮/暗主题定义（v2 设计规范见 `docs/design/settings-redesign-v2.md`）。
//!
//! 通过 `App::theme_handle()` 在运行期切换：底栏的明暗控件调
//! `handle.set(theme_for(dark))` 即可整窗换肤（所有 `*_role` 节点自动跟随）。
//!
//! # 这一层的职责
//!
//! 只有三件事：**调色板**（谁能当底、谁能当字）、**表单排版**（行高/字号/间距/限行）、
//! **卡片与控件的尺幅**（圆角、描边、内边距）。视觉层级靠「底色分三档 + 描边一色」表达，
//! 不靠阴影、渐变、多重强调——每多一层手段，下一处新增控件就多一次选错的机会。

use auroraui::prelude::*;

/// 设置行 hint 悬浮提示的换行宽度（逻辑 px）。设置窗口比库默认场景更宽，
/// 稍微放宽避免过短说明也被强制拆成两三行。
pub const TOOLTIP_MAX_W: f32 = 320.0;

/// 卡片内水平内边距（逻辑 px）。行间分割线两端内缩到与它对齐，卡片标题行也用它。
/// 一处定义，`widgets` 里卡片 / 列表 / 对话框三套写法共用。
pub const CARD_PAD_X: i32 = 18;

/// 卡片圆角。
pub const CARD_CORNER: f32 = 12.0;

/// 页内边距：左右 / 上 / 下。
pub const PAGE_PAD_X: i32 = 28;
pub const PAGE_PAD_TOP: i32 = 20;
pub const PAGE_PAD_BOTTOM: i32 = 24;

/// 卡片之间的纵向间距。
pub const SECTION_GAP: i32 = 16;

/// 表单行排版（`Element::setting_row` / `setting_row_desc` / `field` 一并适用）。
///
/// 这些值原是 `widgets.rs` 顶部的一组私有常量，本仓自建的行构造器逐个读它们；行的实现
/// 收敛到库之后，它们的去处只能是主题——库刻意不让尺寸进构造器签名（一个应用里的表单行
/// 必须整齐划一，逐行传尺寸只会让每处各写一个近似值）。
///
/// 层次策略仍是**字号分层**（标题 13.5 / 说明 11.5）而非字重：DirectWrite 下雅黑 300–500
/// 均落 Regular（细分字重无效），600 起才是 Bold，而整页行标题全加粗过重、显乱。
///
/// 限行取 1：说明文字来自 core 的 manifest，长度不可控（真实数据里四分之一超出左列单行
/// 宽度）。限行后库会自动加末尾省略并挂上悬浮看全文的提示，没真截断时不弹。
fn apply_form_theme(t: &mut Theme) {
    t.form.label_size = Some(13.5);
    t.form.label_weight = Some(500);
    t.form.desc_size = Some(11.5);
    // 行高 44：右侧开关/下拉是 32 高，上下各留 6 才不显得贴边。
    t.form.row_height = Some(44);
    t.form.gap = Some(16);
    // 带说明的行不定高，靠内边距撑：12 + 标题 18 + 间距 2 + 说明 15 + 12 ≈ 59。
    t.form.row_pad_y = Some(12);
    t.form.label_max_lines = Some(1);
    t.form.desc_max_lines = Some(1);
}

/// 卡片与控件的尺幅。`Element::card` 与库自带控件（输入框、下拉、分段、列表）都读它。
fn apply_metrics(t: &mut Theme) {
    let m = &mut t.metrics;
    m.corner_sm = 5.0;
    m.corner_md = 8.0;
    m.corner_lg = CARD_CORNER;
    m.space_xs = 4;
    m.space_sm = 8;
    m.space_md = 12;
    m.space_lg = CARD_PAD_X;
    m.space_xl = 24;
    m.control_h = 32;
    m.control_h_sm = 28;
    m.row_h = 38;
    m.font_xs = 11.5;
    m.font_sm = 12.5;
    m.font_md = 13.5;
    m.font_lg = 15.0;
    m.font_xl = 18.0;
    // 卡片：描边 + 底色，无阴影（阴影在深色底上几乎不可见，在浅色底上又是一层多余的灰边）。
    t.card.corner = Some(CARD_CORNER);
    t.card.pad = Some(CARD_PAD_X);
    t.card.gap = Some(12);
    t.card.title_size = Some(13.5);
    t.card.title_weight = Some(600);
    t.card.bordered = Some(true);
}

/// 亮色主题。
pub fn light_theme() -> Theme {
    let mut t = Theme::default();
    let p = &mut t.palette;
    p.accent = Color::hex(0x2F6BED);
    p.accent_hover = Color::hex(0x1558EB);
    p.accent_active = Color::hex(0x114BC9);
    p.on_accent = Color::WHITE;
    // 三档底色，每档只差一点明度：侧栏/窗口(Bg) → 内容区(SurfaceAlt) → 卡片(Surface)。
    // 卡片最白、内容区略灰，靠这一档差把卡片「托起来」，不用阴影。
    p.bg = Color::hex(0xFAFBFC);
    p.surface = Color::hex(0xFFFFFF);
    p.surface_alt = Color::hex(0xF2F4F7);
    p.text = Color::hex(0x1B1F27);
    p.text_muted = Color::hex(0x5A6474);
    p.text_disabled = Color::hex(0xB3BAC6);
    p.border = Color::hex(0xE5E8ED);
    p.divider = Color::hex(0xEEF0F4);
    // track 兼作 outline_soft 按钮静默边（须深于 border，禁用边才有层级）与开关关闭轨道。
    p.track = Color::hex(0xC7CEDA);
    // 输入框提示（"待填写"语义）。
    p.placeholder = Color::hex(0x9AA3B2);
    // 设置行描述这类三级弱化正文（text_muted 留给次级正文）。
    // 取值与 placeholder 接近是设计稿如此，语义则分家：auroraui 0.12 起有正牌的
    // `Role::TextSubtle`，此前本仓借 `Role::Placeholder` 上色，把可读说明说成了待填写。
    p.text_subtle = Color::hex(0x8A93A3);
    // 反色表面按 auroraui 的约定取正文色 / 窗口底：亮档 tooltip、toast 随之用本仓的蓝灰，
    // 暗档实底按钮换深字时的深字候选也是 `on_surface_inverse`。
    p.surface_inverse = p.text;
    p.on_surface_inverse = p.bg;
    t.metrics.border_width = Len::Physical { px: 1.0 };
    t.metrics.border_width_focus = Len::Physical { px: 2.0 };
    t.tooltip.max_width = Some(TOOLTIP_MAX_W);
    apply_metrics(&mut t);
    apply_form_theme(&mut t);
    t
}

/// 暗色主题。
pub fn dark_theme() -> Theme {
    let mut t = Theme::dark();
    let p = &mut t.palette;
    // 取 `accent::derive` 的不动点：卡片上 4.62:1、压白字 3.41:1，悬停变亮后白字仍 ≥ 3:1。
    p.accent = Color::hex(0x5487F5);
    p.accent_hover = Color::hex(0x6492F6);
    p.accent_active = Color::hex(0x3571F3);
    p.on_accent = Color::WHITE;
    // 强调色当前景就用它本身：本仓的暗档强调色（含主题色派生的，见 `accent::derive`）已保证
    // 在卡片上 ≥ 4.5:1。auroraui 默认再提亮 35%，库画的焦点环、描边按钮会与本仓按
    // `Role::Accent` 画的选中字、图标分成两种蓝。
    p.accent_text = Some(p.accent);
    // 三档底色同亮档思路，但暗色下要**逐档提亮**：卡片比内容区亮一档才浮得起来。
    p.bg = Color::hex(0x111419);
    p.surface = Color::hex(0x1D222B);
    p.surface_alt = Color::hex(0x16191F);
    p.text = Color::hex(0xE9ECF2);
    p.text_muted = Color::hex(0x9BA4B4);
    p.text_disabled = Color::hex(0x606A79);
    p.border = Color::hex(0x2A3038);
    // 分割线须在暗色卡片(0x1D222B)上可辨（历史教训：太接近会不可见）。
    p.divider = Color::hex(0x262C35);
    // track 兼作 outline_soft 按钮静默边（须深/亮于 border）与开关关闭轨道。
    p.track = Color::hex(0x39414F);
    // 输入框提示（"待填写"语义）。
    p.placeholder = Color::hex(0x6E7887);
    // 设置行描述这类三级弱化正文。选型同亮色主题。
    p.text_subtle = Color::hex(0x8D97A6);
    // 反色表面同亮档。
    p.surface_inverse = p.text;
    p.on_surface_inverse = p.bg;
    t.metrics.border_width = Len::Physical { px: 1.0 };
    t.metrics.border_width_focus = Len::Physical { px: 2.0 };
    t.tooltip.max_width = Some(TOOLTIP_MAX_W);
    apply_metrics(&mut t);
    apply_form_theme(&mut t);
    t
}

/// 按明暗标志取主题。
pub fn theme_for(dark: bool) -> Theme {
    if dark { dark_theme() } else { light_theme() }
}

/// 按明暗取主题，并换上给定的强调色组（`None` = 设置窗默认蓝）。
///
/// 只动强调色五个槽位（含 `accent_text`）：底色、卡片、文字等仍是设置窗自己的（见 [`crate::accent`] 的取舍）。
/// 用到强调色的静态控件一律走角色色（`bg_role_alpha` / `bg_gradient_roles` / `tint_role`），
/// 换这五个槽位即整窗跟随。
pub fn theme_with_accent(dark: bool, accent: Option<crate::accent::AccentSet>) -> Theme {
    let mut t = theme_for(dark);
    if let Some(a) = accent {
        let p = &mut t.palette;
        p.accent = a.accent;
        p.accent_hover = a.hover;
        p.accent_active = a.active;
        p.on_accent = a.on_accent;
        p.accent_text = Some(a.accent);
    }
    t
}

/// 检测系统是否为暗色（窗口默认主题随系统，不持久化用户切换）。
///
/// Windows：读注册表 `AppsUseLightTheme`（0=暗 / 1=亮）。
/// 失败或非 Windows：回退暗色（与 ime.rs 一致，暗色效果最佳）。
pub fn system_dark() -> bool {
    #[cfg(windows)]
    {
        // 直接调用 winreg API，避免原先启动 reg.exe 子进程带来的 50-200ms 开销。
        use winreg::{RegKey, enums::HKEY_CURRENT_USER};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) =
            hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            && let Ok(v) = key.get_value::<u32, _>("AppsUseLightTheme")
        {
            return v == 0; // 0 = 暗色
        }
        true
    }
    #[cfg(not(windows))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 守护：强调色只能经调色板（角色色）取用。把默认蓝写成静态色的控件不会跟随
    /// 「主题色：跟随输入法」——换了主题它还是蓝的，而且截图上要切到非默认主题才看得出来。
    #[test]
    fn accent_hex_only_lives_in_theme_def() {
        let palette_hex = ["2F6BED", "1558EB", "114BC9", "5487F5", "6492F6", "3571F3"];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root];
        let mut hits = Vec::new();
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs")
                    && !p.ends_with("theme_def.rs")
                    && !p.ends_with("accent.rs")
                {
                    let text = std::fs::read_to_string(&p).unwrap().to_uppercase();
                    for h in palette_hex {
                        if text.contains(&format!("0X{h}")) {
                            hits.push(format!("{} 含 0x{h}", p.display()));
                        }
                    }
                }
            }
        }
        assert!(
            hits.is_empty(),
            "强调色要用 Role::Accent* 取，别写静态色：\n{}",
            hits.join("\n")
        );
    }

    /// 默认暗档的强调前景就是强调色本身，不经 auroraui 的 35% 提亮（理由见 `dark_theme`）。
    #[test]
    fn dark_accent_text_is_accent() {
        let p = dark_theme().palette;
        assert_eq!(p.accent_text(), p.accent);
    }

    /// 反色表面取本仓自己的正文色 / 窗口底，不落到 auroraui 默认的 slate。
    #[test]
    fn surface_inverse_follows_own_palette() {
        for dark in [false, true] {
            let p = theme_for(dark).palette;
            assert_eq!(p.surface_inverse, p.text, "dark={dark}");
            assert_eq!(p.on_surface_inverse, p.bg, "dark={dark}");
        }
    }

    #[test]
    fn theme_with_accent_only_touches_accent_slots() {
        let set = crate::accent::AccentSet {
            accent: Color::hex(0x05875F),
            hover: Color::hex(0x047A56),
            active: Color::hex(0x046849),
            on_accent: Color::WHITE,
        };
        let base = theme_for(false);
        let t = theme_with_accent(false, Some(set));
        assert_eq!(t.palette.accent, set.accent);
        assert_eq!(t.palette.accent_hover, set.hover);
        assert_eq!(t.palette.accent_active, set.active);
        assert_eq!(t.palette.accent_text(), set.accent);
        assert_eq!(
            theme_with_accent(true, Some(set)).palette.accent_text(),
            set.accent,
            "暗档强调前景不再另行提亮"
        );
        assert_eq!(t.palette.surface, base.palette.surface);
        assert_eq!(t.palette.text, base.palette.text);
        assert_eq!(
            theme_with_accent(true, None).palette.accent,
            theme_for(true).palette.accent
        );
    }
}
