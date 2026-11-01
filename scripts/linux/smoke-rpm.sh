#!/usr/bin/env bash
# 安装冒烟：装上 .rpm，起一个私有 fcitx5 会话，真打字验证「装得上、加载得起来、出得了中文」。
#
#   scripts/linux/smoke-rpm.sh              # 用 dist/linux 下最新那个 .rpm
#   scripts/linux/smoke-rpm.sh <rpm>        # 指定包
#
# 需要 root（装包）与 python3 + dbus-next（pip install dbus-next）。
#
# 为什么必须真装再起 fcitx5：addon 是 dlopen 进 fcitx5 进程的 .so，它的「装对了」判据全在
# 运行时——so 在不在 Fcitx5 扫描的目录、服务能不能按固定路径拉起、词库能不能从
# /usr/lib/aurora-input/data 读到，任何一条不成立都不会有任何构建期报错。
#
# 全程用隔离的 XDG_* 目录（/tmp 下），不碰当前用户的 fcitx5 配置与 AuroraInput 数据；
# 退出时删掉服务进程。不改系统输入法框架配置（im-config / 桌面设置）——那属于用户自己的选择。
set -euo pipefail
export PATH="/usr/bin:${PATH}"
export LC_ALL=C

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RPM_FILE="${1:-}"
if [[ -z "$RPM_FILE" ]]; then
    RPM_FILE="$(find "$REPO/dist/linux" -name 'AuroraInput-*-linux-*.rpm' -type f 2>/dev/null | sort | tail -1)"
fi
[[ -n "$RPM_FILE" && -f "$RPM_FILE" ]] || { echo "找不到 .rpm（先跑 package-rpm.sh）" >&2; exit 1; }

say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
err() { printf '\033[31m错误: %s\033[0m\n' "$*" >&2; }

command -v fcitx5 >/dev/null || { err "需要 fcitx5（sudo dnf install fcitx5 fcitx5-gtk3 fcitx5-qt6）"; exit 1; }
python3 -c 'import dbus_next' 2>/dev/null || { err "需要 dbus-next：pip install dbus-next"; exit 1; }

say "安装 $RPM_FILE"
# --replacepkgs：重复跑这个脚本时重装一遍，而不是报「已安装」。
sudo dnf -y install --setopt=install_weak_deps=False "$RPM_FILE" 2>&1 | tail -5

# 隔离的运行环境：配置/数据/缓存/运行时目录全在临时目录，退出即清。
P="$(mktemp -d /tmp/wi-smoke.XXXXXX)"
chmod 700 "$P"
mkdir -p "$P"/{conf,data,cache,run}
cleanup() {
    # 只杀这次冒烟拉起的那个服务（按运行时目录认，不误伤用户正在用的实例）。
    pkill -f "AURORA_INPUT_RUNTIME_DIR=$P/run" 2>/dev/null || true
    pkill -x -f '/usr/lib/aurora-input/aurora_input( --restarted)?' 2>/dev/null || true
    [[ -n "${KEEP:-}" ]] && { echo "[保留] $P"; return 0; }
    rm -rf "$P"
}
trap cleanup EXIT

# 让 aurorainput 成为默认输入法：Fcitx5 只在 profile 里列出的输入法之间切换。
# 全新 XDG_CONFIG_HOME 下它自己建的是「只有 keyboard-us」的默认组，aurorainput 永远不会被激活。
# ⚠ 必须先 mkdir 再写：重定向比 `cat` 先执行，目录不在时 bash 直接报
# "No such file or directory" 而不是让 `||` 兜底。
mkdir -p "$P/conf/fcitx5"
cat >"$P/conf/fcitx5/profile" <<'PROFILE'
[Groups/0]
Name=Default
Default Layout=us
DefaultIM=aurorainput

[Groups/0/Items/0]
Name=keyboard-us
Layout=

[Groups/0/Items/1]
Name=aurorainput
Layout=

[GroupOrder]
0=Default
PROFILE

# Fcitx5 的全局配置，与 scripts/linux/e2e.sh 同一份（少了它打字一条都吃不到，且毫无报错）：
#   ActiveByDefault —— 新输入上下文直接处于激活态。默认关，每个上下文要先按触发键激活；
#      DBus 客户端每建一个 context 都得先敲一次 Ctrl+Space，测的就不是输入法本身了。
#   TriggerKeys/ActivateKeys/DeactivateKeys 清空 —— 出厂它是 Ctrl+Space / Shift_L 之类，
#      会在引擎之前把 Shift 单击截走（Shift 切中英由 addon 自己接管）。
cat >"$P/conf/fcitx5/config" <<'FCONF'
[Hotkey]
EnumerateWithTriggerKeys=True

[Hotkey/TriggerKeys]

[Hotkey/ActivateKeys]

[Hotkey/DeactivateKeys]

[Behavior]
ActiveByDefault=True
ShareInputState=No
FCONF

say "起 fcitx5 并打字"
# dbus-run-session 起私有会话总线：不碰用户的 org.fcitx.Fcitx5。
set +e
dbus-run-session -- bash -s <<SMOKE
set -e
export XDG_CONFIG_HOME=$P/conf XDG_DATA_HOME=$P/data XDG_CACHE_HOME=$P/cache \
       XDG_RUNTIME_DIR=$P/run FCITX_CONFIG_HOME=$P/conf/fcitx5
# 不需要 X11：只走 DBus 前端打字，不测候选窗。fcitx5 在无 DISPLAY 时仍能起。
unset DISPLAY WAYLAND_DISPLAY 2>/dev/null || true
fcitx5 -d --disable=all --enable=keyboard,dbus,dbusfrontend,aurorainput \
    > $P/fcitx5.log 2>&1 &
FCPID=\$!
# 等 DBus 名字就位（fcitx5 启动后约 0.5-2s 才注册 org.fcitx.Fcitx5）
for i in \$(seq 1 40); do
    if busctl --user list --no-pager 2>/dev/null | grep -q org.fcitx.Fcitx5; then break; fi
    sleep 0.25
done
sleep 1.5
python3 $REPO/scripts/linux/smoke-rpm.py
rc=\$?
kill \$FCPID 2>/dev/null || true
exit \$rc
SMOKE
RC=$?
set -e

say "fcitx5 日志（出错时看这里）"
grep -E 'aurorainput|WindEngine|AuroraEngine|error|Error|失败' "$P/fcitx5.log" 2>/dev/null | head -15 || true

# addon 是否真的被 Fcitx5 装载，只有日志里看得出来：DBus 的 InputMethod1 没有「列出输入法」
# 的方法，加载失败也不会体现在任何返回值上。而「加载了但打字全 FALSE」是另一种失败
# （词库/服务问题），由 Python 侧那几条覆盖——两条一起才是完整判据。
if grep -q 'Loaded addon aurorainput' "$P/fcitx5.log" 2>/dev/null; then
    echo "PASS addon 被 Fcitx5 装载（日志: Loaded addon aurorainput）"
else
    echo "FAIL addon 没被 Fcitx5 装载（日志里没有 'Loaded addon aurorainput'）"
    RC=1
fi

if [[ $RC -eq 0 ]]; then
    say "冒烟通过"
else
    err "冒烟失败（rc=$RC）"
fi
exit $RC
