//! 应用外壳 — 无边框窗口 + 标题栏 + 单栏侧边导航 + 页内分节标签 + 底部操作栏 + 运行时明暗换肤。
//!
//! 标题栏：Logo + 标题（可拖动）+ 窗口按钮。
//! 侧边栏：搜索框 + 单栏导航（选中 = 淡底 + 强调色文字 + 左缘竖条）。
//! 内容区顶部：**横向分节标签条**（本页有哪些卡片、正看着哪一张），随卡片滚动高亮。
//! 底部操作栏：左=状态点 + 明暗 + 主题色；右=恢复本页 / 重新加载 / 应用设置 / 保存并退出。
//!
//! 布局从 v1 的「一级导航 140 + 二级导航 132 + 内容」三栏改为「侧栏 + 内容」两栏：
//! 三栏在 950 宽窗口下只剩约 640 内容区，设置行的控件与说明挤成一列窄条。
//! 分节导航改成横向标签条后，同样的信息密度不再吃掉一整列竖向空间。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use auroraui::core::EventCtx;
use auroraui::prelude::*;

use crate::dialogs::schema_manager::build_schema_manager_dialog;
use crate::dialogs::theme_import_dialog::build_theme_import_dialog;
use crate::dialogs::theme_import_picker::build_theme_import_picker;
use crate::dialogs::update_dialog::build_update_dialog;
use crate::pages;
use crate::state::{AppState, ConnState};

/// 构建整窗 UI。`app` 为共享状态（换肤句柄也在里面，见 [`AppState::apply_ui_theme`]）。
pub fn build(app: &Rc<RefCell<AppState>>) -> Element {
    crate::memstat::mark("[build 入口]");
    // 借用一次构建页面与对话框，随后释放，便于保存闭包再借 mut。
    let (
        tabs_vec,
        tab_sig,
        dark,
        conn,
        feedback,
        dialog,
        theme_import_dialog,
        theme_import_picker,
        backup_dialog,
        config_import_dialog,
        stats_confirm_dialog,
        reload_confirm,
        reset_confirm,
        conn_failed,
        field_dialogs,
    ) = {
        // 「显示定制版隐藏的设置」可能在上一个主窗里切过：建树前按当前值重折清单。
        app.borrow_mut().refold_manifest_if_needed();
        let st = app.borrow();
        // 先清空上一棵设置树的登记项，再构建新的。主窗可以关掉再开（见
        // `dict_window` 与 `main::open_main_window`），本函数因此不再是"一辈子只跑一次"
        // ——不清的话，旧树那批 Signal 句柄会连同新的一起留在注册表里，而它们的槽位
        // 早已随上一个窗口的 `SignalScope` 回收、并被新信号复用。详见 `FieldRegistry::reset`。
        st.fields.reset();
        // reset 把**所有**登记项都清了，包括不在本函数里注册的那批（方案页的三个写回器
        // 与 SideCommitter，原先只在 `AppState::empty()` 里登记一次）。不在这里补回来，
        // 设置树一建好它们就永远缺席——症状是「别的设置能存、只有方案页存不了」：热键 /
        // 引导键改动进不了 diff，保存时提示「没有变更」。
        crate::state::register_schema_registrations(&st.fields, &st.schema_mgr);
        crate::state::register_system_registrations(&st.fields);
        // 搜索态也要复位。它建在 `AppState` 上（进程级，早于任何窗口），所以句柄不会
        // 悬垂——但**值会整份带到新窗**：带着查询词、开着的浮层、或顶着一条方案级指示条
        // 关掉主窗，再开时新窗一亮就弹出上一次的结果、内容区顶上挂着上一次的指路；
        // 连未被认领的 `jump` 都会再滚一次、闪一次。关窗重开该是干净的开始。
        st.search.clear();
        st.search.hint.set(String::new());
        st.jump.set(None);
        // 注意：tabs_vec 必须先求值——all_tabs 构建各 dialog_button 时把对话框登记进
        // st.fields.dialogs，随后才能在本元组里 take 出来挂到根窗口（元组按从左到右求值）。
        let tabs_vec = pages::all_tabs(app, &st);
        // all_tabs 后各「支持全局」的功能键已登记 global_flags，汇总为 keys.global_hotkeys 写回/回填。
        crate::manifest::register_global_hotkeys_collector(&st);
        let field_dialogs = std::mem::take(&mut *st.fields.dialogs.borrow_mut());
        (
            tabs_vec,
            st.tab,
            st.dark,
            st.conn,
            st.feedback,
            build_schema_manager_dialog(app, &st.schema_mgr),
            build_theme_import_dialog(app),
            build_theme_import_picker(app),
            pages::backup::build_backup_restore_dialog(&st.backup_mgr),
            pages::config_import::build_config_import_dialog(
                app,
                &st.config_import,
                &st.schema_mgr,
            ),
            pages::stats::build_stats_confirm_dialog(&st.stats_mgr),
            st.reload_confirm,
            st.reset_confirm,
            st.conn_failed,
            field_dialogs,
        )
    };
    crate::memstat::mark("[全部对话框]");

    // 静态色控件（图标 tint/渐变/淡底）用它做亮暗双态，须在页面构建前注册。
    crate::widgets::init_dark_sig(dark);

    let titlebar = build_titlebar("设置");

    // 快速导入右键菜单：挂在左侧栏与底栏的空白处（真机反馈：挂满整个内容区太泛，
    // 所有条目上都会弹）。内容区不再挂，页内元素的自有菜单不受影响。
    let (ci_menu, mgr_menu, ov_menu) = {
        let st = app.borrow();
        (
            st.config_import.clone(),
            st.schema_mgr.clone(),
            crate::pages::config_import::ThemePkgConfirmSignals::of(&st),
        )
    };
    let ci_menu_footer = ci_menu.clone();
    let mgr_menu_footer = mgr_menu.clone();

    // 左侧栏：搜索框 + 图标导航（选中 = 淡蓝底 + 渐变图标块 + 左缘指示条）。
    let mut names: Vec<String> = Vec::with_capacity(tabs_vec.len());
    let mut pages: Vec<Element> = Vec::with_capacity(tabs_vec.len());
    let mut subnavs: Vec<crate::subnav::SubNav> = Vec::with_capacity(tabs_vec.len());
    let mut empty: Vec<bool> = Vec::with_capacity(tabs_vec.len());
    for t in tabs_vec {
        names.push(t.name);
        pages.push(t.page);
        subnavs.push(t.subnav);
        empty.push(t.empty);
    }
    let nav = build_nav(&names, &empty, tab_sig, app);
    let empty_redirect = redirect_off_empty_tabs(empty, tab_sig);
    let (search_state, jump_sig) = {
        let st = app.borrow();
        // 用户精简清单不折索引（见 `crate::user_hide`），改由这一层按当前开关过滤。
        // 逃生口打开时 `hides_*` 返回 false，被临时放出来的项照常搜得到。
        let store = st.user_hide_store.clone();
        let manifest = std::rc::Rc::new(st.manifest.clone());
        st.search.set_filter(std::rc::Rc::new(
            move |e: &crate::search::index::SearchEntry| {
                // 手写条目：`feature` 就是它的归属卡。
                if let Some(f) = e.target_feature() {
                    return !store.hides_feature(f);
                }
                // 清单项：按 key / 所在整节判，与页面那一行同一套判据。
                match manifest.get(&e.id) {
                    Some(item) => {
                        !store.hides_key(&item.key)
                            && !store.hides_section(&item.group, &item.section)
                    }
                    // 方案级条目（`schema-level:*`）：没有对应的用户清单选择器，照列。
                    None => true,
                }
            },
        ));
        (st.search.clone(), st.jump)
    };
    let search_box = crate::search::ui::build_search_box(&search_state, tab_sig, jump_sig);
    // 侧栏：Bg 角色 = 左栏色（与内容区 SurfaceAlt 不同档，靠底色差分开，不用分隔线）。
    //
    // 宽度 176：单栏导航要能放下「图标 + 两字标题 + 动作项的外开角标」。v1 的 140 是
    // 被右侧二级导航挤出来的，那一栏已经取消。
    // 定制版用 `feature:wizard` 关掉向导时侧栏不放入口；用户打开「显示定制版隐藏的设置」
    // 后才找回来（见 `wizard::should_auto_open` 对自动弹出的另一种处置）。
    let show_wizard_entry = !crate::custom_hide::wizard_hidden(crate::custom_hide::installed())
        || app.borrow().setting_config.ui.show_custom_hidden_settings;
    let mut sidebar = Element::col()
        .width(NAV_W)
        .height_match()
        .bg_role(Role::Bg)
        .padding_edges(Insets::new(10, 12, 10, 12))
        // ⚠ 搜索框与导航之间只要这一档间距。**别往这个 col 里塞 0×0 哨兵**：
        // `spacing` 是加在**每对**相邻子节点之间的，塞两个不可见节点就等于把间距
        // 加了三次——真机上看着就是搜索框与导航离得老远。哨兵一律挂到根 stack。
        .spacing(6)
        .child(search_box)
        .child(nav.weight(1.0));
    if show_wizard_entry {
        sidebar = sidebar.child(wizard_entry(app));
    }
    // 定制版 `feature:config_import` 藏的是整条导入流程：「导入」卡、这里、底栏、词库窗的
    // 右键菜单与 `--page import` 深链一起藏。空菜单 auroraui 不弹。
    let import_shown = !app.borrow().manifest.feature_hidden("config_import");
    let sidebar = sidebar.on_context_menu(move || {
        if !import_shown {
            return Vec::new();
        }
        crate::pages::config_import::import_menu_items(&ci_menu, &mgr_menu, ov_menu)
    });
    let vdivider = Element::leaf()
        .width(1)
        .height_match()
        .bg_role(Role::Divider);

    // 内容区：各分区页叠放，按选中索引切换显示。
    let mut pages_stack = Element::stack().fill().weight(1.0);
    for (i, page) in pages.into_iter().enumerate() {
        pages_stack = pages_stack.child(page.fill().visible_when(move || tab_sig.get() == i));
    }
    // 固定在滚动区之外的三条提示条（降级 / 冲突 / 搜索指路），无内容时各自隐藏不占位。
    //
    // **排序即优先级**，越靠上的越该先被看见：
    // 降级条说「你的配置没生效」，冲突条说「这两个键会互相影响」——两条同时出现时，
    // 降级往往正是冲突判定失真的原因，故降级在前；指示条排末位，它是用户自己搜出来的
    // 一句指路，读完即关，不该跟前两条的「出事了」抢第一眼。
    let bars = Element::col()
        .fill()
        .weight(0.0)
        .child(crate::pages::degradation_bar(app.borrow().degradation))
        .child(crate::pages::conflict_bar(app.borrow().conflict))
        .child(crate::search::hint::build_hint_bar(search_state.hint));
    // 分节标签条：横向，置于页面滚动区之上并常驻——它是「本页有什么、我在哪」的地图，
    // 跟着内容一起滚走就失去意义了。分节不足两块时整条收起（判据见 [`subnav_available`]）。
    let subnav_hidden = app.borrow().manifest.feature_hidden("subnav");
    let show_subnav = subnav_available(&subnavs, tab_sig, subnav_hidden);
    let subnav_bar = build_subnav_bar(&subnavs, tab_sig, show_subnav);
    let content = Element::col()
        .fill()
        .weight(1.0)
        .bg_role(Role::SurfaceAlt)
        .child(bars)
        .child(subnav_bar)
        .child(pages_stack);
    let body = Element::row()
        .fill()
        .weight(1.0)
        .child(sidebar)
        .child(vdivider)
        .child(content);

    // 底栏右键 = 侧栏那份导入菜单 + 「设置项精简…」。
    let app_hide = app.clone();
    let footer = build_footer(app, dark, conn).on_context_menu(move || {
        let mut items = Vec::new();
        if import_shown {
            items = crate::pages::config_import::import_menu_items(
                &ci_menu_footer,
                &mgr_menu_footer,
                ov_menu,
            );
            items.push(MenuItem::separator());
        }
        items.push(hide_menu_item(&app_hide));
        items
    });

    let main_col = Element::col()
        .fill()
        .child(titlebar)
        .child(Element::divider())
        .child(body)
        .child(Element::divider())
        .child(footer);

    let (protocol_overwrite_dlg, protocol_remove_dlg) = build_protocol_confirm_dialogs(app);
    let (assoc_overwrite_dlg, assoc_remove_dlg) = build_assoc_confirm_dialogs(app);
    let theme_pkg_confirm_dlg = build_theme_package_confirm_dialog(app);

    let mut root = Element::stack()
        .fill()
        .bg_role(Role::Bg)
        .child(main_col)
        .child(protocol_overwrite_dlg)
        .child(protocol_remove_dlg)
        .child(assoc_overwrite_dlg)
        .child(assoc_remove_dlg)
        .child(theme_pkg_confirm_dlg)
        .child(dialog)
        // 词库对话框已随词库页迁进独立窗（见 `dict_window::build_content`），不在这里挂。
        .child(theme_import_dialog)
        .child(theme_import_picker)
        .child(backup_dialog)
        .child(config_import_dialog)
        .child(stats_confirm_dialog)
        .child(build_exit_confirm_dialog(app))
        .child(build_reload_dialog(app, reload_confirm))
        .child(build_reset_dialog(app, reset_confirm))
        .child(build_theme_delete_dialog(app))
        .child(build_theme_overwrite_dialog(app))
        .child(build_update_dialog(app))
        .child(crate::dialogs::schema_download_dialog::build_schema_download_dialog(app))
        .child(crate::wizard::view::build_wizard_dialog(app))
        .child(build_conn_failed_dialog(app, conn_failed))
        .child(empty_redirect);
    // dialog_button 的对话框挂到根：全窗模态遮罩 + 居中（否则内联在设置行格内会偏下）。
    for dlg in field_dialogs {
        root = root.child(dlg);
    }
    // 搜索结果浮层挂根：它要盖到内容区上，挂在侧栏里就只有 184 宽。
    // 捕获层必须在浮层**之前**（在其下层），否则点浮层里的项会先被它吃掉。
    let (search_scrim, search_overlay) =
        crate::search::ui::build_results_overlay(&search_state, tab_sig, jump_sig);
    root = root.child(search_scrim).child(search_overlay);
    // 搜索的三枚 0×0 哨兵：查询变化即重算、词库条目开独立窗、跳转副作用汇。
    //
    // 三枚都挂**根**而不是侧栏：侧栏那个 col 有 `spacing`，而 spacing 是加在每对相邻
    // 子节点之间的，塞进去等于把搜索框与导航的间距加了三遍（见上面 sidebar 处的说明）。
    root = root.child(crate::search::ui::build_query_watcher(&search_state));
    root = root.child(crate::search::ui::build_dict_request_watcher(
        &search_state,
        app,
    ));
    // ⚠ jump_sink 必须排在**锚点之后**（锚点都在 main_col 里，而它挂在根的末尾）。
    // 响应式哨兵按树序派发：排到内容区之前的话，这一帧它读到的还是锚点写入之前的值，
    // 开框要白等一帧。同 conflict_watcher 与 toast_sink 之间的先后讲究。
    //
    // 它管的是锚点管不了的两件事：跳转目标在对话框里时把框打开（第二跳才落到框内那
    // 一行），以及方案级目标写出指示条文案。
    root = root.child(crate::search::ui::build_jump_sink(
        jump_sig,
        search_state.hint,
        app.borrow().fields.clone(),
    ));
    // 按键冲突实时监听：每帧刷新 state.conflict（驱动常驻警告条），并对新出现的
    // 「可共存」组合写 feedback 弹一次说明 toast。
    //
    // ⚠ 必须挂在 toast_sink **之前**：两者都在 on_update 阶段按树序派发，watcher 在后就要
    // 等下一帧才被 toast_sink 读到——而渲染由交互驱动、空闲零帧，那一帧未必会来。
    root = root.child(crate::pages::conflict_watcher(app));
    root = root.child(accent_watcher(app));
    // 全局 toast 反馈汇：保存/重载/恢复/协议路由等瞬时结果统一在此弹出（底栏只剩状态点）。
    let root = root.child(crate::pages::dict::reactive::toast_sink(feedback));
    crate::memstat::mark("[build 出口]");
    root
}

/// 一级导航栏宽度（逻辑 px）。搜索框与搜索浮层的捕获层都从它推出来，见 `search::ui`。
///
/// 176：图标 18 + 间距 10 + 两字标题约 30 + 右侧余量，恰好放得下「设置向导」这种五字动作项
/// 而不省略。v1 的 140 是被右边那条二级导航挤出来的宽度，那一栏已取消。
pub(crate) const NAV_W: i32 = 176;

/// 当前页**有没有**分节标签可显示。
///
/// 词库页是个空占位、关于页只有一整块，给它们顶一条空白横条只会让人以为界面坏了。
/// 判据取「可跳转的分块 ≥ 2」——只有一块时点它等于没点。
///
/// 定制版 `feature:subnav` 整条不要（`hidden`），各页都按「没有」处理。
///
/// 计数**每帧现求**（`section_count` 会调各分块的 `visible` 闭包）：用户精简清单藏掉
/// 一张卡后，那一条该当场少掉；只剩一条时整条收起。建树时固化成 `Vec<bool>` 的话，
/// 藏完得重开窗口才变，与「即时生效」的承诺不符。
fn subnav_available(
    navs: &[crate::subnav::SubNav],
    tab: Signal<usize>,
    hidden: bool,
) -> impl Fn() -> bool + 'static {
    let navs: Rc<Vec<crate::subnav::SubNav>> = Rc::new(navs.to_vec());
    move || {
        if hidden {
            return false;
        }
        navs.get(tab.get()).is_some_and(|n| n.section_count() >= 2)
    }
}

/// 分节标签条：各页的标签清单叠在一个 stack 里按 Tab 切换，外加每页一枚高亮解析器。
///
/// 标签本身横向滚动（见 [`crate::subnav::build_list`]）：输入页 13 块，窄窗口下装不下。
///
/// 解析器**故意挂在条内**而不是根上：`on_update` 本就无视可见性（见 `subnav` 模块文档
/// 第 1 条），条隐着它照样每帧跑——于是切到有标签的页时，高亮已经是对的，
/// 不会先闪一下第一项。
fn build_subnav_bar(
    navs: &[crate::subnav::SubNav],
    tab: Signal<usize>,
    visible: impl Fn() -> bool + 'static,
) -> Element {
    let mut stack = Element::stack()
        .width_match()
        .height(crate::subnav::SUBNAV_H);
    for (i, nav) in navs.iter().enumerate() {
        let list = Element::row()
            .fill()
            .cross(Align::Center)
            .padding_xy(crate::theme_def::PAGE_PAD_X, 0)
            .child(crate::subnav::build_list(nav));
        stack = stack.child(list.fill().visible_when(move || tab.get() == i));
        stack = stack.child(crate::subnav::resolver(nav, tab));
    }
    stack.visible_when(visible)
}

/// 侧栏导航：图标+文字项；选中态 = accent 淡底 + 渐变图标块（白图标）+ 左缘指示条。
/// 选中/未选中为两套元素 visible_when 切换（auroraui 无 hover-only/响应式样式）；
/// 渐变图标块与淡底走 accent 角色色，换明暗 / 换强调色自动跟随。
/// 「高级」项前插分隔线（参考稿 nav-sep）。
///
/// 「词库」是**动作项**而非页项：点它开独立窗（[`crate::dict_window::open`]），主窗停在
/// 原页不动。故它没有选中态、没有指示条，改在右侧挂一枚「外开」角标把这件事说出来——
/// 不给提示的话，点一下侧栏而当前页纹丝不动，看着就是按钮坏了。
/// 定制版藏空了的页没有侧栏入口，可深链（`--page`、命令直通车）仍会切过去：落到这种页
/// 就改回首页，免得停在一张白板上。建窗时先校一次（冷启动的深链早于建窗），之后跟着切页信号。
fn redirect_off_empty_tabs(empty: Vec<bool>, tab: Signal<usize>) -> Element {
    let fix = move || {
        if empty.get(tab.get()).copied().unwrap_or(false) {
            tab.set(0);
        }
    };
    fix();
    crate::pages::dict::reactive::reactive_effect(move || tab.version(), fix)
}

fn build_nav(
    names: &[String],
    empty: &[bool],
    sel: Signal<usize>,
    app: &Rc<RefCell<AppState>>,
) -> Element {
    use crate::icons::{EXTERNAL, ICON_PX, nav_icon, tinted_role};

    // 动作项下标经 `page_to_index` 取，不写字面量 4：PAGES 与 `all_tabs` 的顺序本来就
    // 靠约定对齐，这里再手抄一个下标，将来插一页就会指到隔壁那项上。
    let dict_idx = crate::cli::page_to_index("dict");

    // 顶上那行「设置」小标题已撤：窗口标题栏就写着「设置」，侧栏里再说一遍是重复，
    // 白占 26px 还把导航整体往下推。二级导航加进来之后左边有两栏清单，少一层无谓的
    // 标题反而更容易一眼扫完。
    let mut col = Element::col().width_match().spacing(2);

    for (i, name) in names.iter().enumerate() {
        if empty[i] {
            continue;
        }
        if name == "高级" {
            col = col.child(
                Element::col().width_match().padding_xy(10, 8).child(
                    Element::leaf()
                        .width_match()
                        .height(1)
                        .bg_role(Role::Divider),
                ),
            );
        }
        let svg = nav_icon(name);
        let is_action = dict_idx == Some(i);

        // ── 选中态：淡底 + 强调色图标与文字 + 左缘竖条 ──
        //
        // 这三重里只有「淡底 + 强调色文字」是通用选中语言（与分节标签、列表选中一致）；
        // 竖条是一级导航独有的**位置**提示——它表示「你在这整摞页里的哪一页」，
        // 而分节标签表示「在这页的哪一块」，两者不该长得一样。
        //
        // 图标不再垫渐变块：渐变是纯装饰，与「这是当前页」毫无关系，却让选中态比
        // 未选中态亮一整个数量级。
        let indicator = Element::row()
            .width_match()
            .height(38)
            .cross(Align::Center)
            .child(
                Element::leaf()
                    .width(2)
                    .height(16)
                    .corner(1.0)
                    .bg_role(Role::Accent),
            )
            .visible_when(move || sel.get() == i);
        let on = Element::row()
            .width_match()
            .height(38)
            .corner(8.0)
            .cross(Align::Center)
            .spacing(10)
            .padding_xy(12, 0)
            .bg_role_alpha(Role::Accent, auroraui::theme::SELECTED_ALPHA)
            .child(tinted_role(svg, ICON_PX, Role::Accent))
            .child(
                Element::label(name)
                    .font_size(13.0)
                    .font_weight(500)
                    .fg_role(Role::Accent)
                    .weight(1.0)
                    .max_lines(1)
                    .height(18),
            )
            .visible_when(move || sel.get() == i);

        // ── 未选中态（可点击；图标与文字取弱化色，随主题）──
        let app_for_click = app.clone();
        let mut off = Element::row()
            .clickable()
            .on_click(move |ctx| {
                if is_action {
                    crate::dict_window::open(ctx, &app_for_click);
                } else {
                    sel.set(i);
                }
            })
            .width_match()
            .height(38)
            .corner(8.0)
            .cross(Align::Center)
            .spacing(10)
            .padding_xy(12, 0)
            .child(tinted_role(svg, ICON_PX, Role::TextSubtle))
            .child(
                Element::label(name)
                    .font_size(13.0)
                    .font_weight(500)
                    .fg_role(Role::TextMuted)
                    .weight(1.0)
                    .max_lines(1)
                    .height(18),
            );
        if is_action {
            // 角标比正文更淡：它是对「点了会发生什么」的注解，不该与条目本身争视线。
            off = off.child(tinted_role(EXTERNAL, 13, Role::TextSubtle).align(Align::Center));
        }
        // 动作项永不选中，`visible_when` 恒真即可 —— 但仍写成同一个判据，免得哪天
        // `--tab <词库下标>` 之类的路径真把 sel 设成了它，整行凭空消失。
        // （不写死那个数字：`cli::PAGES` 的顺序改过，词库现在是 5 不是 4。）
        let off = off.visible_when(move || sel.get() != i);

        // 层叠顺序 on → off → indicator：on/off 互斥显示，指示条压在最上。
        // 动作项不生成选中态与指示条：它们的 `visible_when` 恒假，留着只是两棵永不显示
        // 的子树，白占布局签名。
        let mut item = Element::stack().width_match().height(38);
        if !is_action {
            item = item.child(on);
        }
        item = item.child(off);
        if !is_action {
            item = item.child(indicator);
        }
        col = col.child(item);
    }
    Element::scroll().fill().child(col)
}

/// 侧栏最底部的「设置向导」动作项：样式同导航未选中态，点它弹向导、不切页。
/// 输入法服务没连上时向导无从预填，只给提示。
fn wizard_entry(app: &Rc<RefCell<AppState>>) -> Element {
    use crate::icons::{ICON_PX, nav_icon, tinted_role};
    let app = app.clone();
    Element::row()
        .clickable()
        .on_click(move |_| {
            let connected = app.borrow().connected;
            if connected {
                crate::wizard::open_wizard(&app);
            } else {
                app.borrow().feedback.set(Some((
                    ToastKind::Info,
                    "输入法未在运行，暂时无法使用设置向导".to_string(),
                )));
            }
        })
        .width_match()
        .height(38)
        .corner(8.0)
        .cross(Align::Center)
        .spacing(10)
        .padding_xy(12, 0)
        .child(tinted_role(nav_icon("设置向导"), ICON_PX, Role::TextSubtle))
        .child(
            Element::label("设置向导")
                .font_size(13.0)
                .font_weight(500)
                .fg_role(Role::TextMuted)
                .weight(1.0)
                .max_lines(1)
                .height(18),
        )
}

/// 自定义标题栏（无边框窗口的可拖动区 + 窗口按钮）。状态与操作在底部操作栏。
///
/// `subtitle` 是三段式标题的末段（主窗「设置」、词库窗「词库管理」）。参数化而非各窗
/// 各写一份：Logo 尺寸、三段式的字重/弱化色分层、版本号基线对齐这些都是调过的，
/// 复制一份出去就会在下次调整时漏改一边，而两个窗口并排摆着，差异一眼可见。
pub(crate) fn build_titlebar(subtitle: &str) -> Element {
    // 应用图标（与 exe/任务栏同一张图，PNG 由 build 期从 .ico 提取的 256px 帧）。
    // 传 28 是**占位**边长：`app_icon` 内缩 2px 后实绘仍是 24px（图标视觉大小不变），
    // 多出的那 2px 是安全边距——小尺寸下按比例算只有 1px，不足以吸收落点吸附的
    // 半物理像素位移，150% 下会把右侧削掉一列（见 `app_icon` 文档）。
    let logo = crate::widgets::app_icon(28);

    // 窗口按钮走全包式（同 aurora-ui-rust 的 frameless / light_titlebar 示例）：按钮吃满
    // 整条标题栏高度、关闭键贴死窗口右缘。靠两处布局，不改控件：
    // - 外层 `cross(Align::Stretch)` 覆盖 `WindowButton` 自报的 32 高，hover 底色铺满
    //   44 高，而不是浮在条中间的一枚小方块；
    // - 内边距只挂在左侧品牌区上——挂整条 row 会把右侧按钮推离窗口边 14px。
    // 品牌区里层重新 `cross(Center)`，免得 Stretch 把 Logo 与文字拉变形。
    // 关闭键 hover 红底仍是方角：Win11 下由 DWM 按系统半径裁出窗口圆角。
    Element::row()
        .width_match()
        .height(44)
        .cross(Align::Stretch)
        .bg_role(Role::SurfaceAlt)
        .window_drag()
        .child(
            Element::row()
                .cross(Align::Center)
                .padding_xy(14, 0)
                .spacing(10)
                .child(logo)
                // 三段式标题：产品名加粗、间隔点与「设置」弱化分层（参考稿 titlebar）。
                .child(
                    Element::row()
                        .cross(Align::Center)
                        .spacing(4)
                        .child(
                            Element::label("极光输入法")
                                .font_size(13.0)
                                .font_weight(600)
                                .fg_role(Role::Text),
                        )
                        .child(
                            Element::label("·")
                                .font_size(13.0)
                                .fg_role(Role::TextDisabled),
                        )
                        .child(
                            Element::label(subtitle)
                                .font_size(13.0)
                                .fg_role(Role::TextMuted),
                        ),
                )
                // 版本号（build 期取自 git tag/hash，弱化色小字，与标题基线对齐）；dev 变体追加「(开发版)」。
                .child(
                    Element::label(format!(
                        "{}{}",
                        env!("AURORA_SETTING_VERSION"),
                        crate::mode::version_suffix()
                    ))
                    .font_size(11.0)
                    .fg_role(Role::TextMuted),
                ),
        )
        .child(Element::leaf().weight(1.0))
        // 图标随主题明暗（否则默认深色字在暗色标题栏上不可见）。
        .child(Element::window_button(WindowButtonKind::Minimize).fg_role(Role::Text))
        .child(Element::window_button(WindowButtonKind::Maximize).fg_role(Role::Text))
        .child(Element::window_button(WindowButtonKind::Close).fg_role(Role::Text))
}

/// 主题色监视：底栏「主题色」下拉、外观页主题下拉（含未保存的选择）、明暗任一变化，就
/// 持久化主题色来源并 [`AppState::sync_accent`]（按需取色 + 换肤）。
///
/// 下拉只写回信号、没有变更回调，故用响应式哨兵（同 `conflict_watcher`）。外观页的主题
/// 下拉随页面重建会换一枚索引信号，所以每帧经注册表现查，而不是建树时捕获一次。
fn accent_watcher(app: &Rc<RefCell<AppState>>) -> Element {
    let (accent_idx, dark, theme_values) = {
        let st = app.borrow();
        (st.accent_idx, st.dark, st.theme_value_sig)
    };
    let ver_app = app.clone();
    // 借不到 AppState 的那一帧沿用上次的版本，免得凭空「变化」一次。
    let theme_sel_ver = Cell::new(0u64);
    let eff_app = app.clone();
    crate::pages::dict::reactive::reactive_effect(
        move || {
            if let Ok(st) = ver_app.try_borrow() {
                let v = st
                    .fields
                    .select_indices
                    .borrow()
                    .get("ui.theme.name")
                    .map(|s| s.version())
                    .unwrap_or(0);
                theme_sel_ver.set(v);
            }
            [
                accent_idx.version(),
                dark.version(),
                theme_values.version(),
                theme_sel_ver.get(),
            ]
            .iter()
            .fold(0u64, |h, v| h.wrapping_mul(1_000_003).wrapping_add(*v))
        },
        move || {
            let Ok(mut st) = eff_app.try_borrow_mut() else {
                return;
            };
            let mode = st.accent_mode();
            st.save_accent_mode(mode);
            st.sync_accent();
        },
    )
}

/// 底栏右键菜单里的「设置项精简…」：开那张独立窗（单例，已开则激活）。
///
/// 它调的是**设置界面自己**显现什么，与任何一页的设置项都不是一类，故不占主窗的一格
/// （理由见 [`crate::user_hide_window`]）。入口只此一处加高级页那个快捷按钮。
fn hide_menu_item(app: &Rc<RefCell<AppState>>) -> MenuItem {
    let app = app.clone();
    MenuItem::run(
        "设置项精简…",
        move |ctx| crate::user_hide_window::open(ctx, &app),
        false,
    )
}

/// 底部操作栏（搜狗式）：左=状态点（悬停出说明）+ 明暗切换 + 主题色；右=恢复本页/重新加载/应用设置/保存并退出。
fn build_footer(
    app: &Rc<RefCell<AppState>>,
    dark: Signal<bool>,
    conn: Signal<ConnState>,
) -> Element {
    // 状态点：绿=服务就绪，琥珀=其余（加载中/离线预览/服务未运行）；外圈 22% alpha 光环。
    // 说明文字只在悬停时出现——常驻的「已连接」被不少用户读成"连上了云端服务器"，
    // 而这里连的是本机输入法进程；文案因此也避开"连接"一词（详见 [`ConnState`]）。
    //
    // 两处实现约束：
    // - `tooltip` 收的是静态文本，给不了 Signal，故四态各建一个点用 visible_when 互斥切换
    //   （四种状态是固定的，这样最省；文案本身要随运行期变化时走 `widgets::info_badge`
    //   那条 `Widget::tooltip()` 的路）；
    // - 提示取命中节点自身（`Tree::node_tooltip` 不回溯父链），所以命中区/光环/实心点
    //   三层都得挂同一句，否则指针停在最上层的实心点上反而不弹。
    let dot = |c: u32, tip: &'static str, when: ConnState| {
        Element::stack()
            .size(22, 22) // 透明命中区：14px 的点太小，悬停不好瞄
            .tooltip(tip)
            .child(
                Element::stack()
                    .size(14, 14)
                    .corner(7.0)
                    .bg(Color::hex(c).scale_alpha(0.22))
                    .align(Align::Center)
                    .tooltip(tip)
                    .child(
                        Element::leaf()
                            .size(8, 8)
                            .corner(4.0)
                            .bg(Color::hex(c))
                            .align(Align::Center)
                            .tooltip(tip),
                    ),
            )
            .visible_when(move || conn.get() == when)
    };
    const DOT_OK: u32 = 0x22C55E;
    const DOT_WARN: u32 = 0xF59E0B;
    let status_dot = Element::stack()
        .child(dot(
            DOT_OK,
            "输入法服务就绪，设置可正常保存",
            ConnState::Ready,
        ))
        .child(dot(DOT_WARN, "正在读取输入法设置…", ConnState::Loading))
        .child(dot(
            DOT_WARN,
            "离线预览（示例数据），修改不会保存",
            ConnState::Mock,
        ))
        .child(dot(
            DOT_WARN,
            "输入法未在运行，设置无法保存",
            ConnState::Down,
        ));

    // 明暗切换：两段胶囊（☀ | 🌙），高亮的那一段就是**当前**状态。
    // 旧版是单个按钮、显示「点了会变成什么」——浅色下写着「深色」，常被读成当前是深色。
    // 切换即持久化——选择写进 aurora_setting.toml 的 ui.dark，下次启动不再跟随系统。
    // 换肤走 `sync_accent`：跟随输入法时，强调色要按新的明暗档重新修正。
    let seg = |glyph: &'static str, tip: &'static str, want_dark: bool| {
        let app_c = app.clone();
        let cell = |fg: Role| {
            Element::stack()
                .size(30, 22)
                .corner(5.0)
                .tooltip(tip)
                .child(
                    Element::label(glyph)
                        .font_size(12.0)
                        .fg_role(fg)
                        .align(Align::Center),
                )
        };
        let on = cell(Role::Accent)
            .bg_role_alpha(Role::Accent, auroraui::theme::SELECTED_ALPHA)
            .visible_when(move || dark.get() == want_dark);
        let off = cell(Role::TextMuted)
            .clickable()
            .on_click(move |_| {
                dark.set(want_dark);
                let mut st = app_c.borrow_mut();
                st.save_dark_mode(want_dark);
                st.sync_accent();
            })
            .visible_when(move || dark.get() != want_dark);
        Element::stack().child(on).child(off)
    };
    let theme_btn = Element::row()
        .cross(Align::Center)
        .spacing(2)
        .padding(2)
        .corner(7.0)
        .border_role(Role::Border, 1)
        .child(seg("☀", "浅色", false))
        .child(seg("🌙", "深色", true));

    // 主题色：默认 / 跟随输入法。选中写回信号，持久化与换肤由 `accent_watcher` 接手。
    // 角标只在跟随模式出现：悬停说明用了什么、为什么没用；点击重新读取（输入法后来才
    // 起来、或在别处改了主题时用）。
    let (accent_idx, accent_note) = {
        let st = app.borrow();
        (st.accent_idx, st.accent_note)
    };
    let accent_pick =
        Element::dropdown(crate::accent::AccentMode::LABELS.to_vec(), accent_idx).width(184);
    let app_reread = app.clone();
    let accent_badge = crate::widgets::info_badge(accent_note, move |_| {
        if let Ok(st) = app_reread.try_borrow() {
            st.invalidate_accent();
            st.sync_accent();
        }
    })
    .visible_when(move || accent_note.with(|n| !n.is_empty()));

    // 刷新数据：只在统计页出现——其余页没有需要重拉的只读数据，常驻会让人误以为
    // 是「重新加载配置」。放底栏而非页内浮层：浮层要么挡内容、要么得在页尾留白让位。
    let (stats_mgr, tab_sig) = {
        let s = app.borrow();
        (s.stats_mgr.clone(), s.tab)
    };
    let stats_tab = crate::cli::page_to_index("stats");
    let refresh_btn = Element::button("刷新数据")
        .small()
        .outline_soft()
        .on_click(move |_| stats_mgr.reload())
        .visible_when(move || stats_tab == Some(tab_sig.get()));

    // 恢复本页：本页改默认值；有未保存改动先弹确认。
    let app_reset = app.clone();
    let reset_btn = Element::button("恢复本页")
        .small()
        .outline_soft()
        .on_click(move |_| {
            if let Ok(s) = app_reset.try_borrow() {
                if s.is_dirty() {
                    s.reset_confirm.set(true);
                } else {
                    s.reset_current_page();
                }
            }
        });

    // 重新加载：从服务端重读，丢弃改动——二次确认。
    let app_reload = app.clone();
    let reload_btn = Element::button("重新加载")
        .small()
        .outline_soft()
        .on_click(move |_| {
            if let Ok(s) = app_reload.try_borrow() {
                s.reload_confirm.set(true);
            }
        });

    // 应用设置：保存当前改动（不退出）。
    let app_apply = app.clone();
    let apply_btn = Element::button("应用设置")
        .small()
        .outline_soft()
        .on_click(move |_| {
            if let Ok(mut s) = app_apply.try_borrow_mut() {
                s.save();
            }
        });

    // 保存并退出：保存成功（无报错）才退出。
    //
    // 它是底栏唯一的**主操作**（实底强调色），其余一律 outline_soft。此前四个按钮同权重
    // 并排，用户得读完四个词才能决定点哪个，而四者里只有它会让窗口消失——误点的代价最大，
    // 越该一眼可辨。
    let app_save_exit = app.clone();
    let save_exit_btn = Element::button("保存并退出").small().on_click(move |ctx| {
        if let Ok(mut s) = app_save_exit.try_borrow_mut()
            && s.save()
        {
            // 走 close_window 而非裸 request_close：保存若有被服务端跳过的项，配置仍判脏，
            // 关窗守卫会再弹一次确认——而用户刚刚已经明确表达过"保存并退出"，跳过项的
            // 提示 save 自己弹的 toast 里有。
            s.close_window(ctx);
        }
    });

    // 明暗 / 主题色各有一个 `feature:` id（`dark_mode` / `accent`），被定制版或用户藏掉的
    // 不放。角标是主题色下拉的附属，随它走。
    //
    // 这两块也能用 Ctrl+右键藏（`wrap_feature_menu`）：它们是底栏控件、不在清单里，
    // 但对用户来说同样是「一个不太用的设置」。判据走 `feature_hidden`，故两套配置都认。
    let (show_dark, show_accent, hide_store) = {
        let st = app.borrow();
        (
            !st.manifest.feature_hidden("dark_mode"),
            !st.manifest.feature_hidden("accent"),
            st.user_hide_store.clone(),
        )
    };
    let feature_menu = |id: &'static str, el: Element| {
        el.on_context_menu(crate::hide_menu::menu_for(
            hide_store.clone(),
            crate::hide_menu::Target::Feature(id),
        ))
    };
    // 用户隐藏走运行期判据（藏了立刻消失，逃生口打开时放回）；定制版隐藏仍是建树期跳过。
    let (s1, s2, s3) = (hide_store.clone(), hide_store.clone(), hide_store.clone());
    let theme_btn =
        feature_menu("dark_mode", theme_btn).visible_when(move || !s1.hides_feature("dark_mode"));
    let accent_pick =
        feature_menu("accent", accent_pick).visible_when(move || !s2.hides_feature("accent"));
    let accent_badge = accent_badge.visible_when(move || !s3.hides_feature("accent"));
    // 底栏与标题栏同色(SurfaceAlt)，与内容区(SurfaceAlt)之间由 main_col 的分割线分开。
    //
    // 主按钮「保存并退出」放最右，符合各平台「主操作在右」的肌肉记忆；「应用设置」用
    // outline_soft 与它拉开权重——两个都是实底蓝时用户得读字才知道哪个是主操作。
    let mut bar = Element::row()
        .width_match()
        .height(52)
        .cross(Align::Center)
        .padding_xy(16, 0)
        .spacing(8)
        .bg_role(Role::SurfaceAlt)
        .child(status_dot);
    if show_dark {
        bar = bar.child(theme_btn);
    }
    if show_accent {
        bar = bar.child(accent_pick).child(accent_badge);
    }
    bar.child(Element::leaf().weight(1.0))
        .child(refresh_btn)
        .child(reset_btn)
        // 「恢复本页」会丢掉本页改动，与保存类按钮并排同权容易误点；隔一条竖线分开。
        .child(
            Element::leaf()
                .size(1, 18)
                .margin_xy(6, 0)
                .bg_role(Role::Divider),
        )
        .child(reload_btn)
        .child(apply_btn)
        .child(save_exit_btn)
}

/// 关窗前的「有未保存的更改」确认框。三个出口：保存并退出 / 直接退出 / 取消。
///
/// 三按钮而非「确定/取消」：两按钮版本逼用户在"丢弃"和"留下"之间二选一，而绝大多数人
/// 此刻想要的是第三件事——存了再走。这也是各家编辑器关窗确认的通行形状。
///
/// 「直接退出」与「保存并退出」都走 `force_close`（绕过守卫）：守卫的判据是"有没有未保存
/// 改动"，而丢弃改动并不会消掉它们——走普通 `request_close` 会被自己的守卫再拦一次、
/// 永远关不掉。
fn build_exit_confirm_dialog(app: &Rc<RefCell<AppState>>) -> Element {
    let (visible, note, tip) = {
        let s = app.borrow();
        (s.exit_confirm, s.unsaved_note, s.unsaved_tip)
    };
    let app_save = app.clone();
    let app_quit = app.clone();
    // 复制走 `unsaved_items()` 重算而不是读 `tip` 信号：那份是给人看的（截断过、
    // 末尾还带一句提示），而贴给维护者的必须是**全量原始键名**。
    let app_copy = app.clone();
    let copy_keys = move |ctx: &mut EventCtx| {
        let Ok(s) = app_copy.try_borrow() else {
            return;
        };
        let items = s.unsaved_items();
        if items.is_empty() {
            return;
        }
        let n = items.len();
        ctx.clipboard_set(&items.join("\n"));
        // 只弹 toast，**不动**那行「共 N 项未保存」：它说的是对话框此刻的事实（还有几项
        // 没保存），复制这个动作不该把它改掉——改了反而丢掉用户正需要看的那个数。
        ctx.toast_ok(format!("已复制 {n} 项未保存的配置键"));
    };
    let frame = Element::col()
        .width(380)
        .bg_role(Role::Surface)
        .border_role(Role::Border, 1)
        .corner(12.0)
        .shadow(Shadow::new(0.0, 16.0, 40.0, Color::rgba(0, 0, 0, 130)))
        .child(
            Element::label("有未保存的更改")
                .font_size(14.0)
                .font_weight(600)
                .fg_role(Role::Text)
                .width_match()
                .max_lines(1)
                .padding_xy(18, 14),
        )
        .child(Element::divider())
        .child(
            Element::col()
                .width_match()
                .padding_xy(18, 16)
                .spacing(8)
                .child(
                    Element::label("关闭设置窗口前是否保存这些更改？未保存的更改将会丢失。")
                        .font_size(13.0)
                        .fg_role(Role::TextMuted)
                        .width_match()
                        .max_lines(3),
                )
                // 「共 N 项未保存 ⓘ」：正文不列具体项——普通用户不关心哪几个配置键在
                // 动，一串 `input.auto_pair.english_pairs` 只会喧宾夺主。清单收进角标，
                // 悬停能看、点击能复制，排查「什么都没动就说有未保存」时一步到位。
                .child(
                    Element::row()
                        .cross(Align::Center)
                        .spacing(6)
                        .child(
                            Element::label_signal(note)
                                .font_size(12.0)
                                .fg_role(Role::TextMuted),
                        )
                        .child(crate::widgets::info_badge(tip, copy_keys)),
                ),
        )
        .child(Element::divider())
        .child(
            Element::row()
                .width_match()
                .cross(Align::Center)
                .padding_xy(16, 12)
                .spacing(8)
                .child(Element::leaf().weight(1.0))
                .child(
                    Element::button("取消")
                        .small()
                        .outline_soft()
                        .on_click(move |_| visible.set(false)),
                )
                .child(
                    Element::button("直接退出")
                        .small()
                        .outline_soft()
                        .danger()
                        .on_click(move |ctx| {
                            visible.set(false);
                            // 走 close_window 而非裸 force_close：退出置位收在那里，
                            // 漏掉这一条，二次实例会误判主窗还在（见 mark_main_window_closed）。
                            app_quit.borrow().close_window(ctx);
                        }),
                )
                .child(Element::button("保存并退出").small().on_click(move |ctx| {
                    // 保存失败（如 core 未运行）就留在原地：此时退出等于丢数据，
                    // 而失败原因已由 save 自己弹的 toast 说明。
                    if let Ok(mut s) = app_save.try_borrow_mut()
                        && s.save()
                    {
                        visible.set(false);
                        s.close_window(ctx);
                    }
                })),
        );
    Element::dialog(visible, frame)
}

/// 「重新加载」二次确认对话框。
fn build_reload_dialog(app: &Rc<RefCell<AppState>>, visible: Signal<bool>) -> Element {
    let app_c = app.clone();
    confirm_dialog(
        visible,
        "重新加载",
        "将丢弃所有未保存的修改，从服务端重新读取配置。确定继续？",
        "重新加载",
        move |_| {
            if let Ok(mut s) = app_c.try_borrow_mut() {
                s.reload();
            }
            visible.set(false);
        },
    )
}

/// 协议关联的两个确认框：覆盖他人注册 / 清除他人注册。
///
/// 只有「当前指向别的程序」时才会被打开（见 `IntegrationState::on_*_clicked`）——
/// 指向本程序或未注册时按钮直接执行，不该为无风险的操作加一道多余的确认。
fn build_protocol_confirm_dialogs(app: &Rc<RefCell<AppState>>) -> (Element, Element) {
    let (overwrite, remove, msg) = {
        let st = app.borrow();
        (
            st.integration.overwrite_confirm,
            st.integration.remove_confirm,
            st.integration.confirm_msg,
        )
    };

    let app_ow = app.clone();
    let overwrite_dlg = confirm_dialog_msg(
        overwrite,
        "改为由本程序处理",
        msg,
        "覆盖注册",
        move |_| {
            if let Ok(mut s) = app_ow.try_borrow_mut() {
                s.register_protocol();
            }
            overwrite.set(false);
        },
    );

    let app_rm = app.clone();
    let remove_dlg = confirm_dialog_msg(
        remove,
        "解除协议关联",
        msg,
        "仍然删除",
        move |_| {
            if let Ok(mut s) = app_rm.try_borrow_mut() {
                s.unregister_protocol();
            }
            remove.set(false);
        },
    );

    (overwrite_dlg, remove_dlg)
}

/// 主题包「要装这个吗」确认框（双击 `.wtheme` / 从导入对话框选到主题包都落在这里）。
///
/// 方案包双击同样是先预览再确认，两种包的双击体验不该一个有一个没有；而主题包多半
/// 是从网上下回来的，双击就直接改掉当前外观并不合适。
///
/// 内容一句话说得完（名字/作者/版本/几个资源），用不上方案包那个带逐键 diff 的大
/// 对话框，复用通用确认框即可。撞名会在这一步之后再弹覆盖确认——两组 signal 分开，
/// 后一个不会把前一个的内容冲掉。
fn build_theme_package_confirm_dialog(app: &Rc<RefCell<AppState>>) -> Element {
    let (visible, msg) = {
        let st = app.borrow();
        (st.theme_pkg_confirm_visible, st.theme_pkg_confirm_msg)
    };
    let app_im = app.clone();
    confirm_dialog_msg(visible, "导入主题包", msg, "导入", move |_| {
        if let Ok(mut s) = app_im.try_borrow_mut() {
            s.confirm_theme_package_import();
        }
        visible.set(false);
    })
}

/// `.wpkg` 文件关联的两个确认框，形态与协议的完全一致（共用 `confirm_msg`，
/// 同刻只开一个框），只是落点换成 `register_file_assoc` / `unregister_file_assoc`。
fn build_assoc_confirm_dialogs(app: &Rc<RefCell<AppState>>) -> (Element, Element) {
    let (overwrite, remove, msg, pending) = {
        let st = app.borrow();
        (
            st.integration.assoc_overwrite_confirm,
            st.integration.assoc_remove_confirm,
            st.integration.confirm_msg,
            // 两个扩展名共用这一对确认框，点下去时靠它认出问的是哪一个。
            st.integration.assoc_pending_idx,
        )
    };

    let app_ow = app.clone();
    let overwrite_dlg = confirm_dialog_msg(
        overwrite,
        "改为由本程序打开",
        msg,
        "覆盖注册",
        move |_| {
            if let Ok(mut s) = app_ow.try_borrow_mut() {
                s.register_file_assoc(pending.get());
            }
            overwrite.set(false);
        },
    );

    let app_rm = app.clone();
    let remove_dlg = confirm_dialog_msg(
        remove,
        "解除 .wpkg 文件关联",
        msg,
        "仍然解除",
        move |_| {
            if let Ok(mut s) = app_rm.try_borrow_mut() {
                s.unregister_file_assoc(pending.get());
            }
            remove.set(false);
        },
    );

    (overwrite_dlg, remove_dlg)
}

/// 「恢复本页」确认对话框（仅在有未保存改动时弹出）。
fn build_reset_dialog(app: &Rc<RefCell<AppState>>, visible: Signal<bool>) -> Element {
    let app_c = app.clone();
    confirm_dialog(
        visible,
        "恢复本页为默认值",
        "本页存在未保存的修改，恢复默认会丢弃这些修改。确定恢复为系统默认值？",
        "恢复默认",
        move |_| {
            if let Ok(s) = app_c.try_borrow() {
                s.reset_current_page();
            }
            visible.set(false);
        },
    )
}

/// 「删除主题」确认对话框：正文含主题名（`theme_delete_msg` 动态填充），确认按钮为危险色。
/// 由主题下拉里外部主题的删除图标触发显示，确认后调 `theme.delete` 并刷新列表。
fn build_theme_delete_dialog(app: &Rc<RefCell<AppState>>) -> Element {
    let st = app.borrow();
    let visible = st.theme_delete_visible;
    let msg = st.theme_delete_msg;
    drop(st);
    let app_c = app.clone();
    let frame = Element::col()
        .width(360)
        .bg_role(Role::Surface)
        .border_role(Role::Border, 1)
        .corner(12.0)
        .shadow(Shadow::new(0.0, 16.0, 40.0, Color::rgba(0, 0, 0, 130)))
        .child(
            Element::label("删除主题")
                .font_size(14.0)
                .font_weight(600)
                .fg_role(Role::Text)
                .width_match()
                .max_lines(1)
                .padding_xy(18, 14),
        )
        .child(Element::divider())
        .child(
            Element::label_signal(msg)
                .font_size(13.0)
                .fg_role(Role::TextMuted)
                .width_match()
                .max_lines(4)
                .padding_xy(18, 16),
        )
        .child(Element::divider())
        .child(
            Element::row()
                .width_match()
                .cross(Align::Center)
                .padding_xy(16, 12)
                .spacing(8)
                .child(Element::leaf().weight(1.0))
                .child(
                    Element::button("取消")
                        .small()
                        .outline_soft()
                        .on_click(move |_| visible.set(false)),
                )
                .child(Element::button("删除").small().danger().on_click(move |_| {
                    if let Ok(mut s) = app_c.try_borrow_mut() {
                        s.confirm_delete_theme();
                    }
                })),
        );
    Element::dialog(visible, frame)
}

/// 「覆盖同名主题」确认对话框：导入撞上已存在的同名主题时弹出（两个导入入口共用），
/// 确认后以 `force=true` 重推 `theme_overwrite_text` 里暂存的原文。
///
/// 没有它的话，同名主题只会得到一条「主题已存在」的 toast——core 一直支持 `force`，
/// 但界面上没有任何触发它的路，用户等于无法更新已装过的主题。
fn build_theme_overwrite_dialog(app: &Rc<RefCell<AppState>>) -> Element {
    let st = app.borrow();
    let visible = st.theme_overwrite_visible;
    let msg = st.theme_overwrite_msg;
    let source = st.theme_overwrite_source;
    drop(st);
    let app_c = app.clone();
    let frame = Element::col()
        .width(360)
        .bg_role(Role::Surface)
        .border_role(Role::Border, 1)
        .corner(12.0)
        .shadow(Shadow::new(0.0, 16.0, 40.0, Color::rgba(0, 0, 0, 130)))
        .child(
            Element::label("覆盖同名主题")
                .font_size(14.0)
                .font_weight(600)
                .fg_role(Role::Text)
                .width_match()
                .max_lines(1)
                .padding_xy(18, 14),
        )
        .child(Element::divider())
        .child(
            Element::label_signal(msg)
                .font_size(13.0)
                .fg_role(Role::TextMuted)
                .width_match()
                .max_lines(4)
                .padding_xy(18, 16),
        )
        .child(Element::divider())
        .child(
            Element::row()
                .width_match()
                .cross(Align::Center)
                .padding_xy(16, 12)
                .spacing(8)
                .child(Element::leaf().weight(1.0))
                .child(
                    Element::button("取消")
                        .small()
                        .outline_soft()
                        // 取消也要清掉暂存原文：一份 TOML 最大 2 MiB，留着白占内存，
                        // 更要紧的是别让下一次「覆盖」误推上一次放弃掉的那份。
                        .on_click(move |_| {
                            source.set(None);
                            visible.set(false);
                        }),
                )
                .child(Element::button("覆盖").small().danger().on_click(move |_| {
                    if let Ok(mut s) = app_c.try_borrow_mut() {
                        s.confirm_overwrite_theme();
                    }
                })),
        );
    Element::dialog(visible, frame)
}

/// 连接失败阻断对话框：[重连] 重跑后台加载；[退出] 关闭应用。无取消项（阻断）。
fn build_conn_failed_dialog(app: &Rc<RefCell<AppState>>, visible: Signal<bool>) -> Element {
    let app_retry = app.clone();
    let app_exit = app.clone();
    let frame = Element::col()
        .width(360)
        .bg_role(Role::Surface)
        .border_role(Role::Border, 1)
        .corner(12.0)
        .shadow(Shadow::new(0.0, 16.0, 40.0, Color::rgba(0, 0, 0, 130)))
        .child(
            Element::label("输入法服务未运行")
                .font_size(14.0)
                .font_weight(600)
                .fg_role(Role::Text)
                .width_match()
                .max_lines(1)
                .padding_xy(18, 14),
        )
        .child(Element::divider())
        .child(
            Element::label("设置需要本机的输入法服务在运行。请确认输入法已启动后重试，或退出。")
                .font_size(13.0)
                .fg_role(Role::TextMuted)
                .width_match()
                .max_lines(3)
                .padding_xy(18, 16),
        )
        .child(Element::divider())
        .child(
            Element::row()
                .width_match()
                .cross(Align::Center)
                .padding_xy(16, 12)
                .spacing(8)
                .child(Element::leaf().weight(1.0))
                .child(
                    Element::button("退出")
                        .small()
                        .outline_soft()
                        // 绕过未保存守卫：core 都连不上，此刻的"改动"根本存不下去，
                        // 再拦一道只会让用户在一个已经没救的界面里多点一次。
                        .on_click(move |ctx| {
                            if let Ok(s) = app_exit.try_borrow() {
                                s.close_window(ctx);
                            } else {
                                ctx.request_close();
                            }
                        }),
                )
                .child(Element::button("重试").small().on_click(move |_| {
                    if let Ok(s) = app_retry.try_borrow() {
                        s.conn.set(ConnState::Loading);
                    }
                    crate::main_reconnect(&app_retry);
                })),
        );
    Element::dialog(visible, frame)
}

/// 通用二次确认对话框：标题 + 说明 + [取消][确认]。`on_confirm` 执行后请自行关闭。
fn confirm_dialog(
    visible: Signal<bool>,
    title: &str,
    message: &str,
    confirm_label: &str,
    on_confirm: impl FnMut(&mut EventCtx) + 'static,
) -> Element {
    confirm_dialog_body(
        visible,
        title,
        Element::label(message).max_lines(4),
        confirm_label,
        false,
        on_confirm,
    )
}

/// 正文随信号变化的变体（如协议关联确认：正文里要嵌入注册表当前的命令行）。
///
/// 单开一个入口而不是把 `&str` 那版改成泛型：静态正文是绝大多数调用点的形态。
/// 限行放到 8 —— 命令行是完整的 exe 路径，压在 4 行里会被省略号吃掉尾巴，
/// 而用户恰恰要靠那段路径判断"占用者是不是我自己另一份安装"。
pub(crate) fn confirm_dialog_msg(
    visible: Signal<bool>,
    title: &str,
    message: Signal<String>,
    confirm_label: &str,
    on_confirm: impl FnMut(&mut EventCtx) + 'static,
) -> Element {
    confirm_dialog_body(
        visible,
        title,
        Element::label_signal(message).max_lines(8),
        confirm_label,
        false,
        on_confirm,
    )
}

/// 确认按钮标红的变体：删除类不可撤销的操作（如清理日志）。
pub(crate) fn confirm_dialog_msg_danger(
    visible: Signal<bool>,
    title: &str,
    message: Signal<String>,
    confirm_label: &str,
    on_confirm: impl FnMut(&mut EventCtx) + 'static,
) -> Element {
    confirm_dialog_body(
        visible,
        title,
        Element::label_signal(message).max_lines(8),
        confirm_label,
        true,
        on_confirm,
    )
}

/// 两个入口共用的外壳。`message` 传的是**未加样式**的标签元素，样式在此统一施加。
fn confirm_dialog_body(
    visible: Signal<bool>,
    title: &str,
    message: Element,
    confirm_label: &str,
    danger: bool,
    on_confirm: impl FnMut(&mut EventCtx) + 'static,
) -> Element {
    let cancel = visible;
    let mut confirm = Element::button(confirm_label).small();
    if danger {
        confirm = confirm.danger();
    }
    let frame = Element::col()
        .width(360)
        .bg_role(Role::Surface)
        .border_role(Role::Border, 1)
        .corner(12.0)
        .shadow(Shadow::new(0.0, 16.0, 40.0, Color::rgba(0, 0, 0, 130)))
        .child(
            Element::label(title)
                .font_size(14.0)
                .font_weight(600)
                .fg_role(Role::Text)
                .width_match()
                .max_lines(1)
                .padding_xy(18, 14),
        )
        .child(Element::divider())
        .child(
            message
                .font_size(13.0)
                .fg_role(Role::TextMuted)
                .width_match()
                .padding_xy(18, 16),
        )
        .child(Element::divider())
        .child(
            Element::row()
                .width_match()
                .cross(Align::Center)
                .padding_xy(16, 12)
                .spacing(8)
                .child(Element::leaf().weight(1.0))
                .child(
                    Element::button("取消")
                        .small()
                        .outline_soft()
                        .on_click(move |_| cancel.set(false)),
                )
                .child(confirm.on_click(on_confirm)),
        );
    Element::dialog(visible, frame)
}

#[cfg(test)]
mod tests {
    use crate::state::AppState;

    /// 建整个主窗，在侧栏下方空白与底栏中段各右击一次，返回两份菜单的项文案。
    fn sidebar_and_footer_menus(hide: &[&str]) -> (Vec<String>, Vec<String>) {
        use auroraui::core::Tree;
        use auroraui::event::{MouseButton, PointerEvent, PointerKind};
        use auroraui::geometry::{Point, Size};
        use std::cell::RefCell;
        use std::rc::Rc;

        let _g = crate::rpc::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AURORA_RPC_MOCK", "1") };
        let app = Rc::new(RefCell::new(AppState::empty()));
        let (h, errors) = aurora_config::SettingsHide::parse(hide);
        assert!(errors.is_empty(), "{errors:?}");
        app.borrow_mut().manifest = crate::custom_hide::settings_manifest(&h, false);

        let (w, hgt) = (1000, 700);
        let mut tree = Tree::new();
        let root = super::build(&app).width(w).height(hgt).build(&mut tree);
        tree.root = Some(root);
        tree.layout_root(Size::new(w, hgt), &mut auroraui::text::NullTextEngine);
        // 根 stack → 主列 [标题栏, 分隔线, 主体, 分隔线, 底栏]；主体首个子节点是侧栏。
        let main_col = tree.get(root).unwrap().children[0];
        let kids = tree.get(main_col).unwrap().children.clone();
        let body = kids[2];
        let side = tree.abs_bounds(tree.get(body).unwrap().children[0]);
        let footer = tree.abs_bounds(kids[4]);

        let mut click = |x: i32, y: i32| -> Vec<String> {
            let (mut hover, mut capture) = (None, None);
            let res = tree.dispatch_pointer(
                PointerEvent::single(PointerKind::Down, Point::new(x, y), MouseButton::Right),
                &mut hover,
                &mut capture,
            );
            res.menu
                .map(|m| {
                    m.items
                        .iter()
                        .filter(|i| !i.separator)
                        .map(|i| i.label.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        let menus = (
            click(side.x + side.w / 2, side.y + side.h - 90),
            click(footer.x + footer.w / 2, footer.y + footer.h / 2),
        );
        unsafe { std::env::remove_var("AURORA_RPC_MOCK") };
        menus
    }

    /// `feature:config_import` 连侧栏与底栏右键里的导入入口一起藏；底栏的「设置项精简…」不受牵连。
    #[test]
    fn config_import_hide_takes_the_context_menus() {
        let (side, footer) = sidebar_and_footer_menus(&[]);
        assert_eq!(
            side,
            ["从剪贴板导入配置", "导入文件…"],
            "前提：侧栏右键是导入菜单"
        );
        assert_eq!(footer, ["从剪贴板导入配置", "导入文件…", "设置项精简…"]);

        let (side, footer) = sidebar_and_footer_menus(&["feature:config_import"]);
        assert!(side.is_empty(), "侧栏右键仍有：{side:?}");
        assert_eq!(footer, ["设置项精简…"]);
    }

    /// 建整个主窗，返回内容区左缘的 x 与分节标签条的高度。
    ///
    /// 两个值一起返回：分节标签条改成横向之后，它不再影响左缘 x，只影响内容区顶端多高。
    /// 原先那条「二级导航隐掉 → 内容区左移 SUBNAV_W」的断言随之作废，
    /// 改成钉住新的不变量（见下面两条测试）。
    fn content_geometry(hide: &[&str]) -> (i32, i32) {
        use auroraui::core::Tree;
        use auroraui::geometry::Size;
        use std::cell::RefCell;
        use std::rc::Rc;

        let _g = crate::rpc::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AURORA_RPC_MOCK", "1") };
        let app = Rc::new(RefCell::new(AppState::empty()));
        let (h, errors) = aurora_config::SettingsHide::parse(hide);
        assert!(errors.is_empty(), "{errors:?}");
        app.borrow_mut().manifest = crate::custom_hide::settings_manifest(&h, false);

        let (w, hgt) = (1000, 700);
        let mut tree = Tree::new();
        let root = super::build(&app).width(w).height(hgt).build(&mut tree);
        tree.root = Some(root);
        tree.layout_root(Size::new(w, hgt), &mut auroraui::text::NullTextEngine);
        // 根 stack → 主列 [标题栏, 分隔线, 主体, 分隔线, 底栏]；主体 = [侧栏, 分隔线, 内容]。
        let main_col = tree.get(root).unwrap().children[0];
        let body = tree.get(main_col).unwrap().children[2];
        let kids = tree.get(body).unwrap().children.clone();
        assert_eq!(kids.len(), 3, "主体应为「侧栏 + 分隔线 + 内容」三节点");
        let content = kids[2];
        let cb = tree.abs_bounds(content);
        // 内容区 = [提示条, 分节标签条, 页面栈]；标签条隐掉时高度为 0。
        let subnav = tree.get(content).unwrap().children[1];
        (cb.x, tree.abs_bounds(subnav).h)
    }

    /// 内容区左缘只由侧栏宽度决定：分节标签条是**横向**的，隐掉它不该让内容区左移。
    ///
    /// 这是 v1 → v2 结构变化的核心不变量：v1 里隐掉二级导航会让内容区左移一整列
    /// （`SUBNAV_W + 1`），v2 里那一列不存在了。
    #[test]
    fn subnav_hide_does_not_move_content_left() {
        let (shown_x, shown_h) = content_geometry(&[]);
        let (hidden_x, hidden_h) = content_geometry(&["feature:subnav"]);
        assert_eq!(
            shown_x, hidden_x,
            "内容区左缘必须与侧栏宽度一致、不随标签条变化"
        );
        assert_eq!(
            shown_h,
            crate::subnav::SUBNAV_H,
            "首页（方案，分块 ≥ 2）应显示分节标签条"
        );
        assert_eq!(hidden_h, 0, "`feature:subnav` 应让标签条不占位");
    }

    /// 内容区左缘 = 侧栏宽 + 1px 分隔线。侧栏改窄/改宽都要与 `search::ui` 的搜索框同步，
    /// 那里有一条 `SIDEBAR_W` 断言守着；这里钉住布局实际落到的位置。
    #[test]
    fn content_left_follows_sidebar_width() {
        let (x, _) = content_geometry(&[]);
        assert_eq!(x, super::NAV_W + 1, "内容区左缘应为「侧栏宽 + 1px 分隔线」");
    }

    /// 重建设置树前必须清空 `FieldRegistry`：主窗可以关掉再开，`shell::build` 因此不再是
    /// 「一辈子只跑一次」。
    ///
    /// 不清的后果有两层，第二层才是致命的：
    /// 1. `writers` 只追加不清理 → 保存时同一个键写两遍；
    /// 2. 上一棵树的 Signal 归**那个窗口**的 `SignalScope`，窗一关就 `dispose()`、槽位
    ///    还给 arena 等待复用，而句柄还留在注册表里 —— 之后读到的是别人的值。
    ///
    /// 症状是「第一次重建好好的、第二次崩」，因为第一次时旧句柄还没被复用。
    #[test]
    fn registry_reset_clears_every_registration() {
        let st = AppState::empty();
        // **七个集合都要填**。这条测试存在的全部理由就是「有人加了字段忘了在 reset 里
        // 清」，而对着从没填过的集合断言 `is_empty()` 是空转——它在字段被漏清时照样绿。
        // （`dialog_opens` 加进来时正是这样：实现清了，这里连提都没提。）
        st.fields.writers.borrow_mut().push(Box::new(|_cfg| {}));
        st.fields
            .getters
            .borrow_mut()
            .insert("k".into(), Box::new(|| serde_json::Value::Bool(true)));
        st.fields
            .loaders
            .borrow_mut()
            .insert("k".into(), Box::new(|_cfg| {}));
        st.fields
            .dialogs
            .borrow_mut()
            .push(auroraui::prelude::Element::leaf());
        st.fields
            .global_flags
            .borrow_mut()
            .push(("dummy".into(), auroraui::prelude::signal(true)));
        st.fields
            .side
            .borrow_mut()
            .push(crate::state::SideCommitter {
                label: "测试落点",
                is_dirty: Box::new(|| false),
                commit: Box::new(|| Ok(())),
            });
        st.fields
            .dialog_opens
            .borrow_mut()
            .insert("dlg".into(), auroraui::prelude::signal(false));

        st.fields.reset();

        assert!(st.fields.writers.borrow().is_empty(), "writers 未清空");
        assert!(st.fields.getters.borrow().is_empty(), "getters 未清空");
        assert!(st.fields.loaders.borrow().is_empty(), "loaders 未清空");
        assert!(st.fields.dialogs.borrow().is_empty(), "dialogs 未清空");
        assert!(
            st.fields.global_flags.borrow().is_empty(),
            "global_flags 未清空 —— 它存的是 Signal 句柄，留着就会悬垂"
        );
        assert!(st.fields.side.borrow().is_empty(), "side 未清空");
        assert!(
            st.fields.dialog_opens.borrow().is_empty(),
            "dialog_opens 未清空 —— 同 global_flags，存的是 Signal 句柄"
        );
    }
}
