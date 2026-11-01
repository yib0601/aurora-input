//! 「设置项精简」独立窗 —— 同进程非模态子窗，做法同 [`crate::compat_window`] / [`crate::dict_window`]。
//!
//! 为什么是独立窗而不是主窗里的一张卡：
//!
//! 它管的是「设置界面本身显现什么」，与任何一页的设置项都不是一类。放进「外观」页会让
//! 那一页在清单被定制版整组藏掉时**仍有内容**，从而破坏「清单藏空了 → 侧栏不给入口」这条
//! 既有语义（`custom_hide` 的 `emptied_card_pages_are_flagged` 就是钉这条的）。而放进
//! 高级页又会让它和「导入 / 备份 / 兼容性」这些一次性动作混在同一列。
//!
//! 独立窗另有一个实际好处：主窗重建设置树时它不受影响，用户能一边开着它一边在主窗里翻页
//! 看效果——这正是这个功能唯一的使用方式。
//!
//! 入口两处：底栏右键菜单「设置项精简…」、高级页「定制版」卡里的按钮。

use std::cell::RefCell;
use std::rc::Rc;

use auroraui::core::EventCtx;
use auroraui::prelude::*;

use crate::state::AppState;

/// 单例键：再点一次入口只把已开着的那个激活到前台。
pub(crate) const SINGLE_KEY: &str = "user_hide";

/// 开窗（已开着则激活）。
pub fn open(ctx: &mut EventCtx, app: &Rc<RefCell<AppState>>) {
    let app_c = app.clone();
    ctx.open_window(
        Window::new("极光输入法 · 设置项精简", 560, 560)
            .single(SINGLE_KEY)
            .frameless(true)
            .centered(true)
            .min_size(480, 400)
            // Ctrl+W 关窗：`App::on_shortcut` 只归主窗，子窗要自己接。
            .on_shortcut(|ctx, ev| {
                if crate::shortcuts::is_close_window(&ev) {
                    ctx.request_close();
                    true
                } else {
                    false
                }
            })
            .content(move || build_content(&app_c)),
    );
}

fn build_content(app: &Rc<RefCell<AppState>>) -> Element {
    Element::col()
        .fill()
        .bg_role(Role::SurfaceAlt)
        .child(crate::shell::build_titlebar("设置项精简"))
        .child(
            Element::scroll().width_match().weight(1.0).child(
                Element::col()
                    .width_match()
                    .padding_edges(Insets::new(
                        crate::theme_def::PAGE_PAD_X,
                        crate::theme_def::PAGE_PAD_TOP,
                        crate::theme_def::PAGE_PAD_X,
                        crate::theme_def::PAGE_PAD_BOTTOM,
                    ))
                    .spacing(crate::theme_def::SECTION_GAP)
                    .child(crate::user_hide_ui::build_section(app))
                    .child(
                        Element::label(
                            "这里只想让界面短一点，不影响任何设置的实际取值——隐藏只影响显示。",
                        )
                        .font_size(11.5)
                        .fg_role(Role::TextSubtle)
                        .width_match()
                        .max_lines(2),
                    ),
            ),
        )
}
