#!/usr/bin/env bash
# 发布前硬门禁：核对一个 .rpm 是否「真是这个架构的、装得起来的、东西齐的」。
#
#   scripts/linux/verify-rpm.sh <rpm> [期望架构：x86_64|aarch64，默认取 rpm 自己的]
#
# 查的都是「静默坏包」类问题（装得上、跑不起来，或只在别人机器上才炸）：
#   1. 包内每个 ELF 的机器类型与架构一致（防构建脚本悄悄用了别的 target）；
#   2. 链接期依赖与二进制真实的 NEEDED 对得上（漏写 Requires ⇒ 装完缺 .so）；
#   3. addon 落在 Fcitx5 真正扫描的目录里（装错目录时「输入法列表里没有极光输入法」，
#      全程零报错）；
#   4. 关键文件齐全、词库不是残缺的、compat.toml 零规则；
#   5. 没有 group/other 可写的文件；maintainer 脚本语法通过；
#   6. %post / %postun 里的 pkill 模式确实能匹配到包内那个路径（防「升级不换二进制」）；
#   7. 三个 ELF 都已被 strip（符号表白占磁盘；打包脚本的 strip 漏了就靠这条红）。
#
# 瘦身包：以 `AURORA_SLIM_DATA=1` 构建的包不含出厂默认关的那几组数据，校验时**也要带同一个
# 变量**，否则「出厂默认关数据」相关的断言会整体误报（见下方 slim_skip）。
#
# 与 verify-deb.sh 的分工：那边查 GLIBC 符号上限（因为 deb 要在 22.04 基线之上运行）；
# rpm 的基线就是构建机本身的 glibc —— 它随发行版走，写死一个上限反而会拦住正当的重建。
# 这里改成查「声明的 Requires 覆盖了真实 NEEDED」。
set -euo pipefail

RPM_FILE="${1:?用法: verify-rpm.sh <rpm> [x86_64|aarch64]}"
[[ -f "$RPM_FILE" ]] || { echo "找不到 $RPM_FILE" >&2; exit 1; }
WANT="${2:-}"

fail=0
bad() { echo "  ✗ $*" >&2; fail=1; }
ok()  { echo "  ✓ $*"; }

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

echo "== $RPM_FILE"

# ── 元数据 ──
ARCH="$(rpm -qp --qf '%{ARCH}' "$RPM_FILE")"
[[ -n "$WANT" ]] || WANT="$ARCH"
[[ "$ARCH" == "$WANT" ]] && ok "Arch = $ARCH" || bad "Arch 是 $ARCH，期望 $WANT"
case "$WANT" in
    x86_64)  ELF_MACHINE="Advanced Micro Devices X86-64" ;;
    aarch64) ELF_MACHINE="AArch64" ;;
    *) echo "不认识的架构: $WANT" >&2; exit 2 ;;
esac
NAME_VER="$(rpm -qp --qf '%{NAME}-%{VERSION}-%{RELEASE}' "$RPM_FILE")"
ok "NEVRA = $NAME_VER.$ARCH"

REQUIRES="$(rpm -qp --requires "$RPM_FILE" | grep -v '^rpmlib(' || true)"
echo "$REQUIRES" | grep -q '^fcitx5 ' || bad "Requires 缺 fcitx5（addon 的宿主）"
echo "$REQUIRES" | grep -q '^fontconfig' || bad "Requires 缺 fontconfig（运行时 dlopen 的字体后端）"

# ── 解包 ──
rpm2cpio "$RPM_FILE" | (cd "$T" && cpio -idm --quiet)
R="$T"

# ── 关键文件 ──
for f in usr/lib/aurora-input/aurora_input usr/lib/aurora-input/aurora_setting \
         usr/lib/aurora-input/data/compat.toml \
         usr/share/fcitx5/addon/aurorainput.conf usr/share/fcitx5/inputmethod/aurorainput.conf \
         usr/share/applications/aurora-input-setting.desktop \
         usr/share/mime/packages/aurora-input.xml \
         usr/share/licenses/aurora-input/LICENSE; do
    [[ -e "$R/$f" ]] && ok "$f" || bad "缺文件: $f"
done

# Fcitx5 系统级默认值（吞掉它的「Wayland 诊断」通知）。除文件与格式外，还要确认它被标成
# %config(noreplace)：否则升级时 rpm 会拿包里的版本覆盖用户改动，且不报任何错。
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
    odd="$(grep -vE '^[[:space:]]*(#|$)|^\[HiddenNotifications\]$|^[0-9]+=.*$' "$R/$CONF" || true)"
    if [[ -n "$odd" ]]; then bad "$CONF 有无法解析的行：$odd"; else ok "$CONF 格式可解析"; fi
    # %config(noreplace) 在 rpm 元数据里是 FILEFLAGS 的 noreplace 位。用可读的 %{FILEFLAGS:fflags}
    # 判（noreplace 显示成 cn、普通 config 是 c、无标记为空），别用数值位运算：gawk 的 and() 不是
    # awk 标准，位值也不该靠记忆（实测 17 = config|noreplace，但这个数随 rpm 版本变）。
    FF="$(rpm -qp --qf '[%{FILENAMES}\t%{FILEFLAGS:fflags}\n]' "$RPM_FILE" 2>/dev/null \
        | awk -F'\t' -v p="/$CONF" '$1==p {print $2; exit}' || true)"
    if [[ "$FF" == *n* ]]; then
        ok "标记为 %config(noreplace)"
    else
        bad "$CONF 未标 %config(noreplace)（当前 fflags='$FF'，升级会覆盖用户改动）"
    fi
else
    bad "缺文件: $CONF（Fcitx5 的 Wayland 诊断通知会照弹）"
fi

# addon 必须落在 Fcitx5 真会扫描的目录：libFcitx5Utils 里硬编的就是 <prefix>/lib64/fcitx5。
# 落到 /usr/lib/fcitx5 也能装，但 Fcitx5 找不到 —— 症状是输入法列表里没有极光输入法，
# 零报错，最难查。
# ⚠ find 对不存在的路径返回 1，而本脚本开 set -e + pipefail：漏掉 `|| true` 会在只装了
# /usr/lib64/fcitx5（正常情况，另一个目录不存在）时让整个门禁静默中止——前面的 ✓ 照常
# 打印，后面的检查一条都不跑，看起来却像「通过了一部分」。这类假绿是本脚本存在的理由。
ADDON="$(find "$R/usr/lib64/fcitx5" "$R/usr/lib/fcitx5" -name libaurorainput.so 2>/dev/null | head -1 || true)"
if [[ -n "$ADDON" ]]; then
    rel="${ADDON#"$R"/}"
    ok "addon: $rel"
    case "$rel" in
        usr/lib64/fcitx5/*) : ;;
        *) bad "addon 在 $rel，但 Fcitx5 只扫描 /usr/lib64/fcitx5（Fedora 64 位）" ;;
    esac
else
    bad "缺 fcitx5 addon libaurorainput.so"
fi
if [[ -n "$ADDON" ]] && readelf -d "$ADDON" 2>/dev/null | grep -q 'libwayland-client'; then
    ok "addon 带 Wayland 候选窗支持"
else
    bad "addon 没链 libwayland-client（构建环境缺 wayland-devel / wayland-scanner？）"
fi

# ── 词库完整性（同 dev.sh verify_dist_data 的口径，只取最能说明「下载/生成失败」的几个）──
D="$R/usr/lib/aurora-input/data"
# 瘦身包（`AURORA_SLIM_DATA=1` 构建的产物）刻意不带「出厂默认关」的那几组数据，对它们的断言
# 必须跳过——否则校验会在这种包上整体误报，而包本身是对的。**校验瘦身包时同样要带这个变量**：
#   AURORA_SLIM_DATA=1 scripts/linux/verify-rpm.sh dist/linux/xxx.rpm
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

# 平台专属策略不得带 Windows 规则：Linux 版 compat.toml 必须是零规则（gen-compat.sh 的产物）。
# 字段说明注释里会提到 Weixin.exe 之类的例子，只看非注释行。
noncomment="$(grep -vE '^[[:space:]]*(#|$)' "$D/compat.toml" || true)"
if [[ -n "$noncomment" ]]; then bad "compat.toml 含非注释内容（应为零规则）"; else ok "compat.toml 零规则"; fi

# ── ELF：机器类型 + NEEDED 覆盖 ──
# 用 rpm 自己算出的自动依赖比对 NEEDED 的库名：RPM 会在构建时把 libX.so.N 转成提供它的包名，
# 所以反过来查「每个 NEEDED 的 .so 都能被某个 Provides 满足」等价于「装完不会缺库」。
#
# ⚠️ LC_ALL=C 是必须的，不是洁癖：LANG=zh_CN 下 readelf 输出「系统架构: Advanced Micro Devices
# X86-64」而不是「Machine:」，按 /Machine:/ 取字段会取到空串 → 每个 ELF 都被 `continue` 跳过
# （不登记为失败！），最后只剩「只找到 0 个 ELF」一条错，图上完全看不出是本地化干的。
# 同一处若改成 `[[ -z "$desc" ]] && continue` 之外的写法，就会变成「静默跳过 = 通过」。
export LC_ALL=C

# ELF 判定不能只看可执行位或 *.so*：`-name '*.so*'` 会匹配到 system.softkeyboard.toml
# 这类数据文件（以 .so 开头的词），readelf 打不开、被跳过，计数与提示都对不上。
# 用 file(1) 认真正的 ELF，与扩展名/权限解耦。
is_elf() {
    file -b --mime-type "$1" 2>/dev/null | grep -q '^application/x-\(pie-\)\?executable$' \
        || file -b --mime-type "$1" 2>/dev/null | grep -q '^application/x-sharedlib$'
}

n=0
while IFS= read -r f; do
    is_elf "$f" || continue
    desc="$(readelf -h "$f" 2>/dev/null | awk -F: '/Machine:/ {sub(/^ +/,"",$2); sub(/ +$/,"",$2); print $2}')"
    n=$((n + 1))
    rel="${f#"$R"/}"
    if [[ -z "$desc" ]]; then bad "$rel 是 ELF 但读不出机器类型"; continue; fi
    if [[ "$desc" != "$ELF_MACHINE" ]]; then bad "$rel 机器类型是「$desc」，期望「$ELF_MACHINE」"; continue; fi
    ok "$rel ($desc)"
done < <(find "$R" -type f | sort)
(( n >= 3 )) || bad "只找到 $n 个 ELF（期望 aurora_input / aurora_setting / libaurorainput.so）"

# ── 符号表必须已摘除（防回归）──
# 打包脚本现在显式 `strip --strip-all`：cargo 的 `strip = true` 之外再兜一层，因为 rpm 的
# brp-strip 只做 `-g`（留 .symtab）、dpkg 侧根本不做 strip。历史产物实测残留 4.76 MiB 符号表，
# 而它装到用户机器上只白占磁盘——这类包装得上、跑得动，只能靠门禁拦。
# ⚠ 判据取 `not stripped`：file(1) 对未 strip 的 ELF 输出是 `..., not stripped`，反向去搜
# `stripped` 会被这个子串命中、把「没 strip」误判成「已 strip」。
STRIP_CHECK="usr/lib/aurora-input/aurora_input usr/lib/aurora-input/aurora_setting"
if [[ -n "$ADDON" ]]; then
    ADDON_REL="${ADDON#"$R"/}"
    STRIP_CHECK="$STRIP_CHECK $ADDON_REL"
fi
for f in $STRIP_CHECK; do
    [[ -f "$R/$f" ]] || continue          # 缺文件已由上面的关键文件检查报过，不重复报
    if file -b "$R/$f" | grep -q 'not stripped'; then
        bad "$f 带符号表（未 strip；打包脚本的 strip --strip-all 没生效？）"
    else
        ok "$f 已 strip"
    fi
done

# 手工核对几个「不查就会在别人机器上炸」的 NEEDED：它们不是来自我们自己的包，
# 必须由 Requires（或 Requires 的传递闭包）提供。这里只查本机装不装得上不出来的几个：
# 缺 fcitx5-libs 时 libFcitx5Core.so.7 找不到，Fcitx5 加载 addon 直接失败。
if [[ -n "$ADDON" ]]; then
    NEEDED_SOS="$(readelf -d "$ADDON" 2>/dev/null | grep NEEDED | grep -o '\[.*\]' | tr -d '[]' || true)"
    for so in $NEEDED_SOS; do
        case "$so" in
            libFcitx5*) ;;
            *) continue ;;
        esac
        # rpm 的自动依赖会生成 libFcitx5Core.so.7()(64bit)，由 fcitx5-libs 满足。
        if echo "$REQUIRES" | grep -q "^${so%.so.*}\.so\..*"; then
            ok "Requires 覆盖 $so"
        else
            bad "Requires 没覆盖 addon 的 NEEDED: $so（缺 fcitx5-libs 时 addon 加载失败）"
        fi
    done
fi

# ── 权限 ──
# 先取结果再判断：`find | grep -q` 在 pipefail 下，grep 一匹配就退出、find 吃 SIGPIPE(141)，
# 整条管道判失败 → 走进 else 打 ✓，恰在可写文件很多（权限归一整体失效）时假通过。
writable="$(find "$R" ! -type l -perm /022)"
if [[ -n "$writable" ]]; then
    bad "存在 group/other 可写的文件："; sed "s|^$R|    |" <<<"$writable" >&2
else ok "无 group/other 可写文件"; fi

# ── 脚本语法 + pkill 模式 ──
POST="$(rpm -qp --qf '[%{POSTIN}][%{POSTUN}]' "$RPM_FILE" 2>/dev/null || true)"
if [[ -n "$POST" ]]; then
    if printf '%s\n' "$POST" | sh -n; then ok "%post/%postun 语法 OK"; else bad "%post/%postun 语法错误"; fi
    # 升级换二进制全靠这句：模式写错就是「装完还是旧版在跑」，且旧进程不会自己退出。
    if printf '%s\n' "$POST" | grep -q "pkill -x -f '/usr/lib/aurora-input/aurora_input( --restarted)?'"; then
        ok "%post 的 pkill 模式与包内路径一致"
    else
        bad "%post 里的 pkill 模式与 /usr/lib/aurora-input/aurora_input 不一致（升级不会换二进制）"
    fi
else
    bad "包没有 %post（升级时旧服务不会退出）"
fi

(( fail == 0 )) && echo "== 通过" || { echo "== 未通过" >&2; exit 1; }
