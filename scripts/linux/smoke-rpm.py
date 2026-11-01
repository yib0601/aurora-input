#!/usr/bin/env python3
"""冒烟：经 DBus 对已装的 fcitx5 + aurorainput addon 打字，断言能出中文。

只验证一件事：`.rpm` 装到系统路径之后，addon 能被 Fcitx5 加载、能拉起
`/usr/lib/aurora-input/aurora_input`、服务能在 `/usr/lib/aurora-input/data` 找到词库并出正确候选。
不测候选窗（那要 X11）与设置程序窗口。

由 scripts/linux/smoke-rpm.sh 在 dbus-run-session 里调用，不直接手跑。
Protocol 与 scripts/linux/e2e_client.py 同源（CreateInputContext / ProcessKeyEvent /
CommitString / UpdateFormattedPreedit）。
"""
import asyncio
import subprocess
import sys

from dbus_next.aio import MessageBus

SERVICE = "org.fcitx.Fcitx5"
IM_PATH = "/org/freedesktop/portal/inputmethod"
IM_IFACE = "org.fcitx.Fcitx.InputMethod1"
IC_IFACE = "org.fcitx.Fcitx.InputContext1"

CAP_PREEDIT = (1 << 1) | (1 << 4)   # Preedit | FormattedPreedit

KEYSYM = {c: ord(c) for c in "abcdefghijklmnopqrstuvwxyz0123456789"}
KEYSYM.update({"space": 0x20, "Return": 0xFF0D, "Escape": 0xFF1B, "BackSpace": 0xFF08})
KEYCODE = {
    **dict(zip("qwertyuiop", range(24, 34))),
    **dict(zip("asdfghjkl", range(38, 47))),
    **dict(zip("zxcvbnm", range(52, 59))),
    **dict(zip("1234567890", range(10, 20))),
    "space": 65, "Return": 36, "Escape": 9, "BackSpace": 22,
}

fail = 0


def check(name, ok, why=""):
    global fail
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {why}"), flush=True)
    if not ok:
        fail = 1


class Ctx:
    def __init__(self, bus, path):
        self.path = path
        self.commits = []
        self.preedits = []

    async def init(self, bus, caps):
        intro = await bus.introspect(SERVICE, self.path)
        self.ic = bus.get_proxy_object(SERVICE, self.path, intro).get_interface(IC_IFACE)
        self.ic.on_commit_string(lambda s: self.commits.append(s))
        self.ic.on_update_formatted_preedit(
            lambda segs, cursor: self.preedits.append("".join(s for s, _ in segs)))
        await self.ic.call_set_capability(caps)

    @property
    def preedit(self):
        return self.preedits[-1] if self.preedits else ""

    async def key(self, name, state=0):
        sym, code = KEYSYM[name], KEYCODE.get(name, 0)
        # 按下 + 抬起两个事件，与 e2e_client 一致（rel=True 才是抬起）。
        eaten = await self.ic.call_process_key_event(sym, code, state, False, 0)
        await self.ic.call_process_key_event(sym, code, state, True, 0)
        await asyncio.sleep(0.03)
        return eaten

    async def type(self, text):
        return [await self.key(c) for c in text]

    def take(self):
        s = "".join(self.commits)
        self.commits.clear()
        return s


async def main():
    bus = await MessageBus().connect()
    intro = await bus.introspect(SERVICE, IM_PATH)
    im = bus.get_proxy_object(SERVICE, IM_PATH, intro).get_interface(IM_IFACE)

    # 1. 服务由包内路径启动（addon 的 ServiceLauncher 拉起的）。
    #    「addon 加载成功」由外层脚本查 fcitx5 日志里的 `Loaded addon aurorainput` ——
    #    Fcitx5 的 DBus 接口只有 CreateInputContext / Version，没有「列出输入法」的方法
    #    （InputMethod1 就这么大），在 DBus 侧问不到；而 addon 加载失败时下面打字必然全 FALSE，
    #    两者合起来才是完整判据。
    out = subprocess.run(["pgrep", "-a", "-x", "aurora_input"],
                         capture_output=True, text=True).stdout
    check("服务由 /usr/lib/aurora-input/aurora_input 启动",
          "/usr/lib/aurora-input/aurora_input" in out, f"pgrep={out.strip()!r}")

    # 2. 建输入上下文，激活。出厂配置里 ActiveByDefault=True（由外层脚本写入），
    #    所以 focus_in 之后直接就是激活态，不必再敲触发键。
    path, _uuid = await im.call_create_input_context([["program", "smoke-test"], ["display", ""]])
    c = Ctx(bus, str(path))
    await c.init(bus, CAP_PREEDIT)
    await c.ic.call_focus_in()
    await asyncio.sleep(0.4)

    # 3. 出厂默认方案是**五笔**（data/config.toml 的 schema.active = "wubi86"），
    #    所以先按 Ctrl+Shift+E 轮换到拼音，再打拼音串。不切就打 "nihao"，五笔里那是
    #    不成字的编码，永远出不了中文——那会是测试自己写错了，不是包坏了。
    ctrl, shift, e = 0xFFE3, 0xFFE1, 0x65
    await c.ic.call_process_key_event(ctrl, 37, 0, False, 0)     # Ctrl 按下
    await c.ic.call_process_key_event(shift, 50, 4, False, 0)    # Shift 按下（state 含 Ctrl）
    await c.ic.call_process_key_event(e, 26, 5, False, 0)        # e 按下（state 含 Ctrl|Shift）
    await c.ic.call_process_key_event(e, 26, 5, True, 0)         # e 抬起
    await c.ic.call_process_key_event(shift, 50, 4, True, 0)
    await c.ic.call_process_key_event(ctrl, 37, 0, True, 0)
    await asyncio.sleep(0.6)

    # 4. 打字出中文。
    #    首次运行（缓存目录空的）时，服务要在这一键上现编 65 万条词库缓存（实测约 6-8 秒），
    #    远超 addon 的 2 秒请求超时：那一键被透传（eaten=False），addon 记一条「服务无响应」
    #    并进入熔断（3s），随后自动恢复。这是首次安装的真实表现，不是坏包 —— 所以这里允许
    #    重试，但必须最终成功；重试用完还不行才算失败（那才是真的装坏了）。
    eaten = []
    for attempt in range(8):
        if attempt:
            await asyncio.sleep(2.5)        # 等熔断过去、服务把缓存写完
            await c.ic.call_reset()
        eaten = await c.type("nihao")
        if all(eaten):
            break
    check("按键被输入法吃掉（没有漏给应用）", all(eaten),
          f"eaten={eaten}（重试 8 次仍被透传：服务没起来或 addon 没连上）")
    check("有预编辑或上屏", bool(c.preedit) or bool(c.commits),
          f"preedit={c.preedit!r} commits={c.commits}")
    await c.key("space")
    await asyncio.sleep(0.4)
    text = c.take()
    check("首选上屏是中文（nihao → 你好 等）",
          any("\u4e00" <= ch <= "\u9fff" for ch in text), f"上屏={text!r}")

    # 4. 切到五笔（再轮一次），打一码验证五笔词库真的进了包、服务能加载它。
    await c.ic.call_reset()
    await asyncio.sleep(0.2)
    await c.ic.call_process_key_event(ctrl, 37, 0, False, 0)
    await c.ic.call_process_key_event(shift, 50, 4, False, 0)
    await c.ic.call_process_key_event(e, 26, 5, False, 0)
    await c.ic.call_process_key_event(e, 26, 5, True, 0)
    await c.ic.call_process_key_event(shift, 50, 4, True, 0)
    await c.ic.call_process_key_event(ctrl, 37, 0, True, 0)
    await asyncio.sleep(0.6)
    await c.type("s")
    check("五笔方案下打 s 有预编辑（五笔词库在包里）", bool(c.preedit),
          f"preedit={c.preedit!r}")
    await c.key("Escape")
    await c.ic.call_reset()


asyncio.run(main())
sys.exit(fail)
