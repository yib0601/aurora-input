#!/usr/bin/env bash
# 在 Ubuntu 22.04（glibc 2.35 / fcitx5 5.0.x）环境里构建并组装 .deb：服务 + Fcitx5 addon + 设置程序。
# 两处调用，都不要在别的发行版上直接跑（会引入更高版本的 glibc 符号，22.04 上装不起来）：
#   - scripts/linux/package-deb.sh：本机 docker 容器里（路径经下面的环境变量传入）；
#   - .github/workflows/linux-build.yml：ubuntu-22.04 / ubuntu-22.04-arm 原生 runner 上。
# 架构取 `dpkg --print-architecture`（amd64 / arm64），产物 <OUT_DIR>/aurora-input_<DEB_VERSION>_<arch>.deb。
#
# 环境变量（括号内为默认值）：
#   DEB_VERSION    必填，deb 的 Version 字段
#   APP_VERSION    必填，产品版本（docs/VERSION 口径），注入设置程序
#   SRC_DIR        WindInput 仓库根（脚本所在仓库）
#   DATA_DIR       随包分发的词库/方案目录（<SRC_DIR>/build_dev/data）
#   SETTING_DIR    设置程序所在目录（<SRC_DIR>/aurora_core/apps/setting，与本仓同 workspace）
#   WORK_DIR       构建缓存与暂存（/tmp/aurora-input-deb-work）
#   OUT_DIR        产物目录（<WORK_DIR>/out）
#   SERVICE_TARGET / SETTING_TARGET   两个 cargo target 目录。服务与设置程序现在同属一个 workspace，
#                  本来可以共用一个 target；保持两份是为了让「只重编设置程序」不触发服务侧全部重编
#                  （两者 feature 组合不同，共用 target 会让 Cargo 反复切换产物）。
set -euxo pipefail

export PATH=/opt/cargo/bin:$PATH
: "${DEB_VERSION:?}" "${APP_VERSION:?}"
SRC_DIR="${SRC_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
DATA_DIR="${DATA_DIR:-$SRC_DIR/build_dev/data}"
SETTING_DIR="${SETTING_DIR:-$SRC_DIR/aurora_core/apps/setting}"
WORK_DIR="${WORK_DIR:-/tmp/aurora-input-deb-work}"
OUT_DIR="${OUT_DIR:-$WORK_DIR/out}"
SERVICE_TARGET="${SERVICE_TARGET:-$WORK_DIR/target}"
SETTING_TARGET="${SETTING_TARGET:-$WORK_DIR/target-setting}"
ARCH="$(dpkg --print-architecture)"
mkdir -p "$WORK_DIR" "$OUT_DIR"

# 设置程序并入本 workspace 后只有一份 Cargo.lock（aurora_core/Cargo.lock），
# 可以加回 --locked；此处暂不加，留到发版链路整体验证时统一处理。

# ── 服务 ──
(cd "$SRC_DIR/aurora_core" && CARGO_TARGET_DIR="$SERVICE_TARGET" \
    cargo build --release -p aurora_service --features linux-host)

# ── 设置程序 ──
# 版本号与服务同源（docs/VERSION），不走 git——worktree 的 .git 指向宿主路径，容器里读不到。
(cd "$SETTING_DIR" && AURORA_APP_VERSION="$APP_VERSION" CARGO_TARGET_DIR="$SETTING_TARGET" \
    cargo build --release -p aurora_setting)

# ── addon ──
cmake -S "$SRC_DIR/aurora_linux" -B "$WORK_DIR/cmake" -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
cmake --build "$WORK_DIR/cmake"
S="$WORK_DIR/stage/pkg"
rm -rf "$S"
DESTDIR="$S" cmake --install "$WORK_DIR/cmake"

# ── 组装 ──
install -Dm755 $SERVICE_TARGET/release/aurora_input "$S/usr/lib/aurora-input/aurora_input"
mkdir -p "$S/usr/lib/aurora-input/data"
cp -a "$DATA_DIR/." "$S/usr/lib/aurora-input/data/"
# 应用兼容规则是平台专属策略：Windows 版 compat.toml 里全是 Windows 宿主的修正，不能随 Linux 包带。
# 换成「字段说明 + 零条内置规则」的 Linux 版（生成脚本自检不得残留规则表）。
bash "$SRC_DIR/scripts/lib/gen-compat.sh" linux "$DATA_DIR/compat.toml" "$S/usr/lib/aurora-input/data/compat.toml"
# addon / 输入法描述：cmake 只装了库，描述文件（构建目录里 configure_file 出来的）补进去。
install -Dm644 $WORK_DIR/cmake/share/fcitx5/addon/aurorainput.conf "$S/usr/share/fcitx5/addon/aurorainput.conf"
install -Dm644 $WORK_DIR/cmake/share/fcitx5/inputmethod/aurorainput.conf "$S/usr/share/fcitx5/inputmethod/aurorainput.conf"
install -Dm755 $SETTING_TARGET/release/aurora_setting "$S/usr/lib/aurora-input/aurora_setting"
install -Dm644 $SRC_DIR/scripts/linux/pkg/aurora-input-setting.desktop "$S/usr/share/applications/aurora-input-setting.desktop"
install -Dm644 $SRC_DIR/scripts/linux/pkg/aurora-input-import.desktop "$S/usr/share/applications/aurora-input-import.desktop"
install -Dm644 $SRC_DIR/scripts/linux/pkg/aurora-input-mime.xml "$S/usr/share/mime/packages/aurora-input.xml"
# 图标（aurorainput / aurorainput-zh / aurorainput-en，各尺寸）由上面的 cmake --install 装好。
install -Dm644 $SRC_DIR/aurora_linux/README.md "$S/usr/share/doc/aurora-input/README.md"

# Fcitx5 系统级默认值：吞掉它的「Wayland 诊断」通知（wayland-diagnose-gnome）。
# 那条提示是 fcitx5 自己发的（libwayland 的 selfDiagnose：Wayland 会话 + GNOME + 当前 UI 不是
# kimpanel 就发，5.1.2 起才有），内容是让用户装 kimpanel Shell 扩展；对清风不成立——候选窗
# 走本 addon 自己的 X11 / layer-shell 面板，不经 Fcitx5 的 InputPanel，装不装 kimpanel 都一样。
# 落点必须是 /etc/xdg（StandardPath::PkgConfig 的 XDG_CONFIG_DIRS 层）而不是用户目录：不能替
# 用户写 ~/.config。读取路径 readAsIni → StandardPath::open 是「用户目录命中即停、整份不合并」，
# 所以这份只是默认值：用户点过「不要再显示」或被设置工具写过 ~/.config/fcitx5/conf/notifications.conf
# 时，用户那份整份生效、本文件被忽略（不会把用户自己的列表合并回来，也就不会关不掉）。
install -Dm644 $SRC_DIR/scripts/linux/pkg/fcitx5-notifications.conf \
    "$S/etc/xdg/fcitx5/conf/notifications.conf"

# 权限归一：`cp -a` 把宿主 umask（002 → 0775/0664）原样带进包，装到系统里就是 root 组可写。
# 目录 755；带执行位的文件 755、其余 644（符号链接不动）。
find "$S" -type d -exec chmod 755 {} +
find "$S" -type f -perm /111 -exec chmod 755 {} +
find "$S" -type f ! -perm /111 -exec chmod 644 {} +

# 链接期依赖交给 dpkg-shlibdeps 按真实 ELF 符号算（libc6 / libstdc++6 / libgcc-s1 / libfcitx5* / libxcb*），
# 而不是手写——以后升级编译器或第三方库时手写表很容易漏。算不到的才手写：fcitx5 本体与最低版本
# （addon 按 5.0.14 的头文件编，是发布基线）、运行时 dlopen 的 libfontconfig1（ELF 里没有 NEEDED）、
# 字体 / procps 这类非库依赖。
SHL="$WORK_DIR/shlibs"
rm -rf "$SHL"; mkdir -p "$SHL/debian"
printf 'Source: aurora-input\n\nPackage: aurora-input\nArchitecture: any\n' >"$SHL/debian/control"
SHLIBS_DEPENDS="$(cd "$SHL" && dpkg-shlibdeps -O \
    -e"$S/usr/lib/aurora-input/aurora_input" -e"$S/usr/lib/aurora-input/aurora_setting" \
    -e"$(find "$S/usr/lib" -name libaurorainput.so -path '*fcitx5*')" | sed -n 's/^shlibs:Depends=//p')"
[[ -n "$SHLIBS_DEPENDS" ]] || { echo "dpkg-shlibdeps 没有算出依赖" >&2; exit 1; }
# fcitx5-configtool：改输入法列表的图形界面在独立包里，fcitx5 本体只 Recommends 它。
# 走 Depends：用户要调整输入法就离不开它，且体积只有 ~1MB。
EXTRA_DEPENDS="fcitx5 (>= 5.0.14), fcitx5-configtool, libfontconfig1, fonts-noto-cjk | fonts-wqy-microhei | fonts-wqy-zenhei, procps"

mkdir -p "$S/DEBIAN"
# /etc 下的文件登记成 conffile：这是「系统级默认值」，用户或发行版改过之后，升级 / 卸载时
# dpkg 必须保留他们的版本、而不是拿包里的覆盖回去。`dpkg-deb --build` 只**校验**已经存在的
# DEBIAN/conffiles（dpkg 源码 src/deb/build.c 的 check_conffiles），不会从 `/etc` 自动推导，
# 漏了这一行它就是个普通文件，且升级覆盖时不报任何错。
printf '/etc/xdg/fcitx5/conf/notifications.conf\n' >"$S/DEBIAN/conffiles"
# 文件对话框：门户与 zenity 分开推荐。门户在、但没有文件选择后端时也弹不出来，rfd 那时不退
# zenity；设置端再兜底试 kdialog。写成「portal | zenity」的话装了门户就不装 zenity，兜底落空。
SIZE=$(du -sk --apparent-size "$S" | cut -f1)
cat >"$S/DEBIAN/control" <<CONTROL
Package: aurora-input
Version: $DEB_VERSION
Architecture: $ARCH
Maintainer: Aurora Input <noreply@example.invalid>
Section: utils
Priority: optional
Installed-Size: $SIZE
Depends: $SHLIBS_DEPENDS, $EXTRA_DEPENDS
Recommends: fcitx5-frontend-gtk3, fcitx5-frontend-qt5 | fcitx5-frontend-qt6, im-config, xclip | wl-clipboard, xdg-utils, xdg-desktop-portal, zenity | kdialog, libxkbcommon0, fonts-noto-color-emoji
Homepage: https://github.com/yib0601/aurora-input
Description: 极光输入法 (WindInput) —— Fcitx5 输入法引擎
 极光输入法的 Linux 版（测试版）：Rust 服务负责输入逻辑与候选窗渲染，
 Fcitx5 addon 负责与应用对接，另带图形设置程序（应用菜单「极光输入法设置」，
 或在输入时按 Ctrl+Shift+]）。安装后重启 Fcitx5 或重新登录即可使用：首次启动会自动
 把「极光输入法」加进输入法组并配好共享输入状态，无需手动配置。
CONTROL
# 结束服务只认包里那个路径：addon 用 posix_spawn 拉起时 argv 恰为这一个全路径、不带参数，
# 菜单「重启服务」自拉起的再多一个 `--restarted`；`pkill -x -f` 要求整条命令行与模式完全
# 匹配——同名的开发构建、`aurora_input restart` 这类 CLI 调用都不会被误杀。
# 装的是所有用户共用的二进制，升级 / 卸载时每个用户的旧实例都该结束，故不限 -u。
# 服务没有 SIGTERM 处理，被结束时防抖窗口（1s）内未落盘的运行时状态会丢（docs/design/linux-port.md §7）。
cat >"$S/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    # 升级时旧服务还在跑旧二进制：结束它，addon 在下次连不上时会自动拉起新版。
    # （已加载进 fcitx5 的旧 addon 需重启 fcitx5 或重新登录才换新。）
    pkill -x -f '/usr/lib/aurora-input/aurora_input( --restarted)?' 2>/dev/null || true
    echo "极光输入法已安装。重启 Fcitx5（fcitx5 -rd）或重新登录即可使用——首次启动会自动把「极光输入法」加进输入法组并配好共享输入状态，无需手动添加。"
fi
exit 0
POSTINST
chmod 755 "$S/DEBIAN/postinst"
# 卸载放在 postrm 而不是 prerm：文件删掉之后再结束，addon（仍在已运行的 fcitx5 里）想重新
# 拉起时程序已不在（它先 access(X_OK)），不会在 prerm 与删文件之间又起一个孤儿。
cat >"$S/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
if [ "$1" = remove ]; then
    pkill -x -f '/usr/lib/aurora-input/aurora_input( --restarted)?' 2>/dev/null || true
fi
exit 0
POSTRM
chmod 755 "$S/DEBIAN/postrm"

dpkg-deb --root-owner-group --build "$S" "$OUT_DIR/aurora-input_${DEB_VERSION}_${ARCH}.deb"
