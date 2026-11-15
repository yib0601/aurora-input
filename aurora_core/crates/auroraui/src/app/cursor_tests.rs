//! `App::cursor_shapes` / `Window::cursor_shapes` 的宿主级契约：关掉后控件声明的光标形状
//! 不再落到系统光标上，开着一如既往。
//!
//! 走真实路径（build → 渲染一帧布局 → 指针移到可点容器上 → 问宿主光标），不 mock：
//! 这条决策的唯一入口是 `AppHandler::cursor()`，绕开它就测不到「悬停命中 + 祖先回溯 +
//! 开关」三者的合流。主窗与子窗两条建窗路径分别覆盖——它们的配置不是同一份。

use crate::app::{App, ThemeHandle, UiHost, Window};
use crate::event::{CursorShape, MouseButton, PointerEvent, PointerKind};
use crate::geometry::{Color, Point, Size};
use crate::platform::{AppHandler, NewWindow};
use crate::render::PixmapTarget;
use crate::theme::Theme;
use crate::ui::Element;
use std::cell::RefCell;
use std::rc::Rc;
use tiny_skia::Pixmap;

const W: i32 = 160;
const H: i32 = 80;

/// 一个声明手型的可点容器（外层 padding 8，容器 80×30）。
fn body() -> Element {
    Element::col()
        .fill()
        .padding(8)
        .child(Element::col().width(80).height(30).clickable())
}

/// 指针落在可点容器上的逻辑坐标。
fn click_point() -> Point {
    Point::new(30, 20)
}

/// 渲一帧让布局生效（命中要 bounds），再把指针移上去，返回宿主此刻的光标。
fn probe(host: &mut dyn AppHandler) -> CursorShape {
    host.set_scale(1.0);
    let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
    host.render(&mut PixmapTarget { pixmap: &mut pm }, Size::new(W, H));
    host.on_pointer(PointerEvent::single(
        PointerKind::Move,
        click_point(),
        MouseButton::Left,
    ));
    host.cursor()
}

/// 主窗那条路（`App::cursor_shapes` → `run` → 宿主）。
fn main_host(shapes: bool) -> UiHost {
    let app = App::new("t", W, H).content(body());
    let app = if shapes {
        app
    } else {
        app.cursor_shapes(false)
    };
    app.into_handler_for_test()
}

/// 子窗那条路：`ctx.open_window` 与托盘/热键回调共用 `build_new_window` 做
/// 「`WindowRequest` → 建窗配置 + 宿主」的转换，开关必须在这里搬过去。
fn child_host(shapes: bool) -> Box<dyn AppHandler> {
    let win = Window::new("t", W, H);
    let win = if shapes {
        win
    } else {
        win.cursor_shapes(false)
    };
    let req = win.content(body);
    let theme_src = ThemeHandle::detached(Theme::default());
    let ops: super::HotkeyOpQueue = Rc::new(RefCell::new(Vec::new()));
    match super::build_new_window(req, &theme_src, &ops, Color::WHITE) {
        NewWindow::Create(cfg, host) => {
            // 第二跳的守卫：少了这一行搬运，设置程序的几个子窗会静默退回手型/I 形，
            // 而主窗那条用例照样绿。
            assert_eq!(
                cfg.cursor_shapes, shapes,
                "建窗配置没搬上 `WindowRequest` 的开关"
            );
            host
        }
        NewWindow::Focus(_) => panic!("没设单例键，不该走激活分支"),
    }
}

/// 默认开着：可点容器悬停仍是手型。这条既守住默认值，也守住「开关没有把功能关死」。
#[test]
fn clickable_reports_hand_by_default() {
    assert_eq!(probe(&mut main_host(true)), CursorShape::Hand);
}

/// 关掉后同一个悬停点回箭头。
#[test]
fn cursor_shapes_off_forces_arrow() {
    assert_eq!(probe(&mut main_host(false)), CursorShape::Arrow);
}

/// 子窗同样吃这个开关，且是在建窗配置那一跳就吃下的。三条用例合起来才封住两个建窗路径：
/// 只断言 `WindowRequest` 字段的话，删掉 `build_new_window` 里那一行也没人红。
#[test]
fn child_window_carries_the_switch() {
    assert_eq!(probe(&mut *child_host(true)), CursorShape::Hand);
    assert_eq!(probe(&mut *child_host(false)), CursorShape::Arrow);
}

/// `Window::cursor_shapes(false)` 要真的写到请求上（标题栏那条链之外的最外一环）。
#[test]
fn window_request_carries_the_switch() {
    let req = Window::new("t", 100, 100)
        .cursor_shapes(false)
        .content(Element::col);
    assert!(!req.cursor_shapes);
    let req = Window::new("t", 100, 100).content(Element::col);
    assert!(req.cursor_shapes, "默认必须是开：别把功能关死");
}
