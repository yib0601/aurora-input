//! 「设置项精简」的界面部分 —— 开关 / 逃生口 / 恢复默认 / 当前条数。
//!
//! 数据与落盘都在 [`crate::user_hide`]（它自己读写 `wind_setting_hide.toml`），本模块只负责
//! 把那份状态呈现成一张卡片，并接住三个动作。
//!
//! 它此前挂在「界面细节」试验窗里——那个窗是**过渡工具**（比较几种界面写法，选定后删除）。
//! 试验层退场时，这一节是**正式功能**，故整体搬到独立模块，入口改成「外观」页的一张卡片：
//! 它调节的正是设置界面本身呈现什么，与外观页其余内容同一类事。
//!
//! 隐藏只影响显示，被藏的设置照常生效（值仍来自配置文件，本程序不改写它），故这里所有
//! 文案都强调「只影响显示」。

use auroraui::prelude::*;

use crate::state::AppState;

/// 一整个「设置项精简」卡片。
pub fn build_section(app: &std::rc::Rc<std::cell::RefCell<AppState>>) -> Element {
    let store = app.borrow().user_hide_store.clone();
    let version = store.version();

    let enabled_on = signal(store.enabled());
    let show_on = signal(store.show_hidden());
    let summary = signal(hide_summary(&store));
    let errors = signal(hide_errors_text(&store));

    // 开关的初值与 store 保持同步：写盘失败时 store 会回滚，界面得跟着退回去。
    let (st_eff, ver_eff) = (store.clone(), version);
    let sync_effect = crate::pages::dict::reactive::reactive_effect(
        move || ver_eff.get(),
        move || {
            enabled_on.set(st_eff.enabled());
            show_on.set(st_eff.show_hidden());
        },
    );

    let (st_ver, sum_sig, err_sig) = (store.clone(), summary, errors);
    let refresh_effect = crate::pages::dict::reactive::reactive_effect(
        move || version.get(),
        move || {
            sum_sig.set(hide_summary(&st_ver));
            err_sig.set(hide_errors_text(&st_ver));
        },
    );

    let st_en = store.clone();
    let enabled_row = crate::widgets::setting_row(
        "开启设置项隐藏功能",
        Some(
            "开启后，在任意设置项或卡片上按 Ctrl+右键，可复制其配置 key、隐藏这一项 / 这一节。\
             隐藏只影响显示，设置照常生效；改动即时生效。",
        ),
        Element::switch(enabled_on).on_switch_change(move |ctx, v| {
            if let Err(e) = st_en.set_enabled(v) {
                ctx.toast_err(e);
            } else if v {
                ctx.toast("可在设置项上按 Ctrl+右键隐藏；改动即时生效");
            }
        }),
    );

    let st_show = store.clone();
    let show_row = crate::widgets::setting_row(
        "显示已隐藏的设置",
        Some(
            "临时把已隐藏的设置全部放回来，并用警示色描边标出，便于按 Ctrl+右键取消隐藏。\
             与定制版的隐藏互不影响——定制版藏掉的设置仍要用高级页的开关找回。",
        ),
        Element::switch(show_on)
            .enabled_signal(enabled_on)
            .on_switch_change(move |ctx, v| {
                if let Err(e) = st_show.set_show_hidden(v) {
                    ctx.toast_err(e);
                } else if v {
                    ctx.toast("已显示隐藏的设置，它们带警示色描边");
                }
            }),
    );

    let st_reset = store.clone();
    let reset_row = crate::widgets::setting_row(
        "恢复默认",
        Some("清空你自己隐藏的全部设置项与分区。定制版的隐藏不受影响。"),
        Element::button("全部恢复")
            .small()
            .outline_soft()
            .enabled_signal(enabled_on)
            .on_click(move |ctx| match st_reset.reset() {
                Ok(0) => ctx.toast("当前没有你自己隐藏的设置"),
                Ok(n) => ctx.toast(format!("已恢复 {n} 项")),
                Err(e) => ctx.toast_err(e),
            }),
    );

    let summary_row = crate::widgets::setting_row(
        "当前隐藏",
        None,
        Element::label_signal(summary)
            .font_size(12.0)
            .fg_role(Role::TextMuted)
            .max_lines(1),
    );

    let err_row = Element::label_signal(errors)
        .font_size(11.5)
        .fg_role(Role::Warning)
        .width_match()
        .max_lines(3)
        .visible_when(move || !errors.get().is_empty());

    Element::col()
        .width_match()
        .child(crate::widgets::card_items(
            None,
            None,
            vec![
                crate::widgets::CardItem::Row(enabled_row),
                crate::widgets::CardItem::Row(show_row),
                crate::widgets::CardItem::Row(summary_row),
                crate::widgets::CardItem::Row(reset_row),
                crate::widgets::CardItem::Row(err_row),
            ],
            None,
        ))
        .child(sync_effect)
        .child(refresh_effect)
}

fn hide_summary(store: &crate::user_hide::UserHideStore) -> String {
    let n = store.count();
    if n == 0 {
        "尚未隐藏任何设置".to_string()
    } else if !store.enabled() {
        format!("已隐藏 {n} 项（功能已关闭，不生效）")
    } else if store.show_hidden() {
        format!("已隐藏 {n} 项（当前正在显示）")
    } else {
        format!("已隐藏 {n} 项")
    }
}

fn hide_errors_text(store: &crate::user_hide::UserHideStore) -> String {
    let errors = store.errors();
    if errors.is_empty() {
        String::new()
    } else {
        format!(
            "有 {} 条规则无法解析，已忽略：{}",
            errors.len(),
            errors.join("；")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_reflects_store_state() {
        let s =
            crate::user_hide::UserHideStore::in_memory(crate::user_hide::UserHideConfig::default());
        assert_eq!(hide_summary(&s), "尚未隐藏任何设置");
        s.add("key:ui.font.family").unwrap();
        s.set_enabled(true).unwrap();
        assert_eq!(hide_summary(&s), "已隐藏 1 项");
        s.set_show_hidden(true).unwrap();
        assert_eq!(hide_summary(&s), "已隐藏 1 项（当前正在显示）");
        s.set_show_hidden(false).unwrap();
        s.set_enabled(false).unwrap();
        assert_eq!(hide_summary(&s), "已隐藏 1 项（功能已关闭，不生效）");
    }

    #[test]
    fn errors_text_is_empty_without_bad_rules() {
        let s =
            crate::user_hide::UserHideStore::in_memory(crate::user_hide::UserHideConfig::default());
        assert!(hide_errors_text(&s).is_empty());
    }
}
