#!/usr/bin/env bash
# 发布前硬门禁：核对一个 .deb 是否「真是这个架构的、22.04 上装得起来的、东西齐的」。
#
#   scripts/linux/verify-deb.sh <deb> [期望架构：amd64|arm64，默认取 deb 自己的 Architecture]
#
# 查的都是「静默坏包」类问题（装得上、跑不起来，或在别的机器上才炸）：
#   1. control 的 Architecture 与期望一致（防 arm 包被标成 amd64 后 apt 在 amd64 上装它）；
#   2. 包内每个 ELF 的机器类型与架构一致（防构建脚本悄悄用了别的 target）；
#   3. 动态符号引用的 glibc 版本不超过 2.35（Depends 写的是 libc6 >= 2.35，超了就是谎报）；
#   4. 关键文件齐全、词库不是残缺的；
#   5. 没有 group/other 可写的文件；maintainer 脚本能过 sh -n；
#   6. 三个 ELF 都已被 strip（符号表白占磁盘；dpkg 侧不做 strip，全靠打包脚本显式做）。
#
# 瘦身包：以 `AURORA_SLIM_DATA=1` 构建的包不含出厂默认关的那几组数据，校验时**也要带同一个
# 变量**，否则相关断言会整体误报（见下方 slim_skip）。
set -euo pipefail

DEB="${1:?用法: verify-deb.sh <deb> [amd64|arm64]}"
[[ -f "$DEB" ]] || { echo "找不到 $DEB" >&2; exit 1; }
WANT="${2:-}"
MAX_GLIBC="2.35"

fail=0
bad() { echo "  ✗ $*" >&2; fail=1; }
ok()  { echo "  ✓ $*"; }

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
dpkg-deb -x "$DEB" "$T/root"
dpkg-deb -e "$DEB" "$T/ctl"

echo "== $DEB"
ARCH="$(dpkg-deb -f "$DEB" Architecture)"
[[ -n "$WANT" ]] || WANT="$ARCH"
if [[ "$ARCH" == "$WANT" ]]; then ok "Architecture = $ARCH"; else bad "Architecture 是 $ARCH，期望 $WANT"; fi
case "$WANT" in
    amd64) ELF_MACHINE="Advanced Micro Devices X86-64" ;;
    arm64) ELF_MACHINE="AArch64" ;;
    *) echo "不认识的架构: $WANT" >&2; exit 2 ;;
esac

R="$T/root"
# 依赖：链接期依赖由 dpkg-shlibdeps 算出，缺了说明构建脚本漏了这一步（手写表容易漏）
deps="$(dpkg-deb -f "$DEB" Depends)"
for d in libc6 libstdc++6 libfcitx5core7 'fcitx5 (>= 5.0.14)'; do
    [[ "$deps" == *"$d"* ]] || bad "Depends 缺 $d"
done

# 关键文件
for f in usr/lib/aurora-input/aurora_input usr/lib/aurora-input/aurora_setting \
         usr/share/fcitx5/addon/aurorainput.conf usr/share/fcitx5/inputmethod/aurorainput.conf \
         usr/share/applications/aurora-input-setting.desktop usr/share/mime/packages/aurora-input.xml \
         usr/lib/aurora-input/data/compat.toml; do
    [[ -e "$R/$f" ]] || bad "缺文件: $f"
done
addon="$(find "$R/usr/lib" -name libaurorainput.so -path '*fcitx5*' | head -1)"
[[ -n "$addon" ]] && ok "addon: ${addon#"$R"/}" || bad "缺 fcitx5 addon libaurorainput.so"

# Fcitx5 系统级默认值（吞掉它的「Wayland 诊断」通知）。三件都要对：文件在、格式对、登记成 conffile。
# 单独成段而不进上面那个循环，是因为「在 /etc 下但不是 conffile」是静默坏包：装得上、也生效，
# 但用户改过之后一升级就被包里的版本覆盖回去，且 dpkg 不报任何错。
CONF=etc/xdg/fcitx5/conf/notifications.conf
if [[ -f "$R/$CONF" ]]; then
    ok "$CONF"
    if grep -qx '0=wayland-diagnose-gnome' "$R/$CONF" && grep -qx '\[HiddenNotifications\]' "$R/$CONF"; then
        ok "diagnose tip id 正确"
    else
        # 格式是 fcitx5 的 RawConfig 序列化口径（list 型 option 写成 0=/1=，见 iniparser.cpp）。
        # 写错不会报错，只是整个文件被当成空配置 —— 症状是通知照弹，最难查。
        bad "$CONF 内容不是 [HiddenNotifications] + 0=wayland-diagnose-gnome"
    fi
    # 先取变量再判断，不用 `pipeline && bad`：管道返回 1（本该是好事）时 `&& bad` 不执行，
    # 整条列表却返回非零，set -e 会让整个门禁静默中止 —— 与上文 writable 那句同一个坑。
    odd="$(grep -vE '^[[:space:]]*(#|$)|^\[HiddenNotifications\]$|^[0-9]+=.*$' "$R/$CONF" || true)"
    if [[ -n "$odd" ]]; then
        bad "$CONF 有无法解析的行：$odd"
    else
        ok "$CONF 格式可解析"
    fi
    if [[ -f "$T/ctl/conffiles" ]] && grep -qx "/$CONF" "$T/ctl/conffiles"; then
        ok "已登记 conffile"
    else
        bad "$CONF 未登记为 conffile（升级会覆盖用户改动；dpkg-deb 只校验不推导，需写 DEBIAN/conffiles）"
    fi
else
    bad "缺文件: $CONF（Fcitx5 的 Wayland 诊断通知会照弹）"
fi
# Wayland 候选窗：CMake 缺 libwayland-dev / wayland-scanner 时只告警、照常出包（没有 Wayland 候选窗），
# 在发布门禁里把它升级成硬错——否则 Wayland 会话里候选窗静默不显示。
if [[ -n "$addon" ]]; then
    if readelf -d "$addon" 2>/dev/null | grep -q 'libwayland-client'; then ok "addon 带 Wayland 候选窗支持"
    else bad "addon 没链 libwayland-client（构建环境缺 libwayland-dev / libwayland-bin？）"; fi
fi

# 词库完整性（同 dev.sh verify_dist_data 的口径，只取最能说明「下载/生成失败」的几个）
D="$R/usr/lib/aurora-input/data"
# 瘦身包（`AURORA_SLIM_DATA=1` 构建的产物）刻意不带「出厂默认关」的那几组数据，对它们的断言
# 必须跳过——否则校验会在这种包上整体误报，而包本身是对的。**校验瘦身包时同样要带这个变量**：
#   AURORA_SLIM_DATA=1 scripts/linux/verify-deb.sh dist/linux/xxx.deb
# 路径清单与 scripts/dev.sh 的 SLIM_DATA_PATHS 同步（同构实现散在三个脚本里，改一处要改三处）。
SLIM="${AURORA_SLIM_DATA:-}"
slim_skip() {
    [[ "$SLIM" = 1 ]] || return 1
    case "$1" in
        opencc/*) return 0 ;;                                       # 出厂关：简繁转换
        schemas/wubi86/wubi86_jidian_extra_district.dict.yaml) return 0 ;;  # 出厂不勾选：行政区划
        schemas/stroke/*) return 0 ;;                               # 出厂不激活：笔画方案（整目录被剔）
        *) return 1 ;;
    esac
}
check_min() {
    local p="$D/$1" min="$2"
    if slim_skip "$1"; then ok "词库 $1（瘦身包已剔除,跳过）"; return; fi
    if [[ ! -f "$p" ]]; then bad "词库缺失: $1"; return; fi
    local sz; sz=$(stat -c%s "$p")
    if (( sz < min )); then bad "词库过小(${sz}B < ${min}B): $1"; else ok "词库 $1 (${sz}B)"; fi
}
check_min schemas/pinyin/cn_dicts/base.dict.yaml 1000000
check_min schemas/wubi86/wubi86_jidian.dict.yaml 1000000
check_min schemas/pinyin/cn_dicts/8105.dict.yaml 10000
check_min schemas/english/en.dict.yaml 1000
check_min schemas/wubi86/wubi86_jidian_extra.dict.yaml 10000
check_min schemas/wubi86/wubi86_jidian_emoji.dict.yaml 1000
check_min schemas/wubi86/wubi86_jidian_extra_district.dict.yaml 10000
check_min opencc/STPhrases.octrie 100000
check_min opencc/STCharacters.octrie 10000
check_min pinyin_map.txt 10000
check_min schemas/stroke/stroke.dict.yaml 500000
# 平台专属策略不得带 Windows 规则：Linux 版 compat.toml 必须是零规则（gen-compat.sh 的产物）
# （字段说明注释里会提到 Weixin.exe 之类的例子，只看非注释行：出现任何表头 / 键值就是带了规则）
noncomment="$(grep -vE '^[[:space:]]*(#|$)' "$D/compat.toml" || true)"
if [[ -n "$noncomment" ]]; then bad "compat.toml 含非注释内容（应为零规则）"; else ok "compat.toml 零规则"; fi

# 包里不得再带一次性配置脚本：装完 deb、在 Fcitx5 里添加输入法就必须能用
if [[ -e "$R/usr/bin/aurora-input-setup" ]]; then bad "包里不应再有 usr/bin/aurora-input-setup"; else ok "无 aurora-input-setup"; fi

# ELF：机器类型 + glibc 符号版本上限
n=0
while IFS= read -r f; do
    desc="$(readelf -h "$f" 2>/dev/null | awk -F: '/Machine:/ {sub(/^ +/,"",$2); print $2}' || true)"
    [[ -n "$desc" ]] || continue
    n=$((n + 1))
    rel="${f#"$R"/}"
    if [[ "$desc" != "$ELF_MACHINE" ]]; then bad "$rel 机器类型是「$desc」，期望「$ELF_MACHINE」"; continue; fi
    top="$(readelf -sW --dyn-syms "$f" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -1 || true)"
    if [[ -n "$top" ]] && [[ "$(printf '%s\n%s\n' "$top" "$MAX_GLIBC" | sort -V | tail -1)" != "$MAX_GLIBC" ]]; then
        bad "$rel 需要 GLIBC_$top（> $MAX_GLIBC）"
    else
        ok "$rel ($desc, glibc ≤ ${top:-无})"
    fi
done < <(find "$R" -type f \( -perm /111 -o -name '*.so*' \))
(( n >= 3 )) || bad "只找到 $n 个 ELF（期望 aurora_input / aurora_setting / libaurorainput.so）"

# ── 符号表必须已摘除（防回归）──
# dpkg 这条链**完全不 strip**，全靠 build-deb.sh 里显式那句 `strip --strip-all`；漏了就静默地把
# 符号表塞进包（历史产物实测 4.76 MiB）。这里把它钉成硬门禁。
# ⚠ 判据取 `not stripped`：file(1) 对未 strip 的 ELF 输出是 `..., not stripped`，反向去搜
# `stripped` 会被这个子串命中、把「没 strip」误判成「已 strip」。
STRIP_CHECK="usr/lib/aurora-input/aurora_input usr/lib/aurora-input/aurora_setting"
if [[ -n "$addon" ]]; then
    ADDON_REL="${addon#"$R"/}"
    STRIP_CHECK="$STRIP_CHECK $ADDON_REL"
fi
for f in $STRIP_CHECK; do
    [[ -f "$R/$f" ]] || continue          # 缺文件已由上面的关键文件检查报过，不重复报
    if file -b "$R/$f" | grep -q 'not stripped'; then
        bad "$f 带符号表（未 strip；build-deb.sh 的 strip --strip-all 没生效？）"
    else
        ok "$f 已 strip"
    fi
done

# 权限
# 先取结果再判断：`find | grep -q` 在 pipefail 下，grep 一匹配就退出、find 吃 SIGPIPE(141)，
# 整条管道判失败 → 走进 else 打 ✓，恰在可写文件很多（权限归一整体失效）时假通过。
writable="$(find "$R" ! -type l -perm /022)"
if [[ -n "$writable" ]]; then
    bad "存在 group/other 可写的文件："; sed "s|^$R|    |" <<<"$writable" >&2
else ok "无 group/other 可写文件"; fi

# maintainer 脚本
for s in postinst postrm; do
    [[ -f "$T/ctl/$s" ]] || continue
    if sh -n "$T/ctl/$s"; then ok "$s 语法 OK"; else bad "$s 语法错误"; fi
done

(( fail == 0 )) && echo "== 通过" || { echo "== 未通过" >&2; exit 1; }
