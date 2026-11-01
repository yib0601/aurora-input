#!/usr/bin/env bash
# 在当前 Fedora 环境里构建并组装 .rpm：服务 + Fcitx5 addon + 设置程序 + 词库。
#
#   scripts/linux/package-rpm.sh                 # 产物 dist/linux/AuroraInput-<版本>.x86_64.rpm
#   DATA_DIR=/path/to/data scripts/linux/package-rpm.sh
#
# 与 package-deb.sh 的分工：那边在 Ubuntu 22.04 容器里构建（glibc 基线 2.35，包能在所有
# Debian/Ubuntu 系上装）；rpm 的 ABI 基线按 RPM 发行版走，Fedora 各版本之间的 glibc 与
# fcitx5 版本差得比 Debian 系还大，跨版本兼容没有意义也无法验证。所以这里**就在本机 Fedora
# 上原生构建**，产出的包声明的就是本机的 fcitx5 / glibc 基线，装回本机（或同版本 Fedora）
# 必然成立。换个版本想发 rpm，在那个版本的机器上重跑本脚本。
#
# 环境变量（括号内为默认值）：
#   DATA_DIR       随包分发的词库/方案目录（<仓库>/build_dev/data，即 dev.sh gd 的产物）
#   OUT_DIR        产物目录（<仓库>/dist/linux）
#   WORK_DIR       构建缓存（<仓库>/../.cache/wi-rpm 里的 cargo target、cmake、stage）
#   SETTING_DIR    设置程序目录（默认 <仓库>/aurora_core/apps/setting）
#   SERVICE_TARGET / SETTING_TARGET   两个 cargo target 目录
#   RELEASE        rpm 的 Release 号（默认 1；同一版本重打包时用 2、3…）
#
# 依赖（Fedora 上一次性装好）：
#   sudo dnf install rust cargo cmake ninja-build gcc gcc-c++ pkgconf-pkg-config \
#        fcitx5-devel libxcb-devel wayland-devel wayland-protocols-devel glib2-devel \
#        rpm-build rpmdevtools git curl
set -euo pipefail

# Fedora 的 cargo/rustc 在 /usr/bin，但用户装了 rustup 时 PATH 里那份是 shell 函数/代理，
# 没有默认 toolchain 就直接报错。这里显式优先 /usr/bin，与 body 里 cargo 调用一致。
export PATH="/usr/bin:${PATH}"

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERSION="$(tr -d '[:space:]' <"$REPO/docs/VERSION")"
RELEASE="${RELEASE:-1}"
DATA_DIR="${DATA_DIR:-$REPO/build_dev/data}"
OUT_DIR="${OUT_DIR:-$REPO/dist/linux}"
WORK_DIR="${WORK_DIR:-$REPO/.cache/wi-rpm}"
SETTING_DIR="$(readlink -f "${SETTING_DIR:-$REPO/aurora_core/apps/setting}")"
SERVICE_TARGET="${SERVICE_TARGET:-$WORK_DIR/target-service}"
SETTING_TARGET="${SETTING_TARGET:-$WORK_DIR/target-setting}"
ARCH="$(uname -m)"            # rpm 的 Arch 与 uname 一致（x86_64 / aarch64）
CMAKE_DIR="$WORK_DIR/cmake"

say()  { printf '\n\033[1m== %s\033[0m\n' "$*"; }
err()  { printf '\033[31m错误: %s\033[0m\n' "$*" >&2; }
gray() { printf '\033[90m%s\033[0m\n' "$*"; }

# ── 前置检查 ──
command -v rpmbuild >/dev/null || { err "需要 rpmbuild（sudo dnf install rpm-build）"; exit 1; }
command -v cmake    >/dev/null || { err "需要 cmake"; exit 1; }
[[ -d "$DATA_DIR/schemas" ]] || { err "DATA_DIR 不是词库目录（缺 schemas/）: $DATA_DIR
  先跑 scripts/dev.sh gd 生成词库。"; exit 1; }
[[ -f "$SETTING_DIR/Cargo.toml" ]] || { err "找不到设置程序仓库: $SETTING_DIR（用 SETTING_DIR 指定）"; exit 1; }
# 拿真实路径再判断，这样 worktree / 任意克隆名下都能正确解析。
# 设置程序的 path 依赖锚点现在是本仓内部；auroraui 仍按 ../aurora-ui-rust 解析，所以
# aurora-input——克隆成别的名字时在这里明确报错，而不是让 cargo 报一堆找不到 crate 的错。
CORE_LINK="$(readlink -f "$SETTING_DIR/../aurora-input")"
[[ -d "$CORE_LINK/aurora_core" ]] || { err "设置程序的 path 依赖锚点 ../aurora-input 不可用: $CORE_LINK
  本仓需以 aurora-input 为目录名（或在该位置建一个指向本仓的符号链接）"; exit 1; }
[[ "$CORE_LINK" == "$REPO" ]] || { err "设置程序解析到的 core 仓库不是本仓: $CORE_LINK ≠ $REPO"; exit 1; }

mkdir -p "$WORK_DIR" "$OUT_DIR"

say "构建 .rpm  $VERSION-$RELEASE  ($ARCH)"

# ── 1. 服务 ──
# linux-host 让协调器/UI/桥接走「外部宿主（Fcitx5 addon）」形态，见 docs/design/linux-port.md §3。
say "[1/4] 服务 aurora_input"
(cd "$REPO/aurora_core" && CARGO_TARGET_DIR="$SERVICE_TARGET" \
    cargo build --release -p aurora_service --features linux-host)

# ── 2. 设置程序 ──
# 版本号与服务同源（docs/VERSION），不走 git——worktree 的 .git 指向宿主路径，换个布局就读不到。
say "[2/4] 设置程序 aurora_setting"
(cd "$SETTING_DIR" && AURORA_APP_VERSION="$VERSION" CARGO_TARGET_DIR="$SETTING_TARGET" \
    cargo build --release)

# ── 3. addon ──
# CMAKE_INSTALL_PREFIX=/usr 让 GNUInstallDirs 在 Fedora 上算出 lib64：addon 落到
# /usr/lib64/fcitx5。这与 fcitx5 自己的 addon 目录（libFcitx5Utils 里硬编的 /usr/lib64/fcitx5）
# 一致；写死 /usr/lib 会装到一个 Fcitx5 不扫描的目录，表现为「输入法列表里看不到极光输入法」。
say "[3/4] Fcitx5 addon"
cmake -S "$REPO/aurora_linux" -B "$CMAKE_DIR" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr \
    -DAURORA_APP_VERSION="$VERSION"
cmake --build "$CMAKE_DIR"

# ── 4. 组装安装树 ──
say "[4/4] 组装安装树"
S="$WORK_DIR/root"
rm -rf "$S"
DESTDIR="$S" cmake --install "$CMAKE_DIR"

# 服务 / 设置程序 / 词库放私有目录：addon 按固定路径拉起它们（ServiceLauncher / SettingsLauncher
# 的 AURORA_*_PATH），服务按可执行文件同目录找 data/（variant::install_root）。
install -Dm755 "$SERVICE_TARGET/release/aurora_input" "$S/usr/lib/aurora-input/aurora_input"
install -Dm755 "$SETTING_TARGET/release/aurora_setting" "$S/usr/lib/aurora-input/aurora_setting"
mkdir -p "$S/usr/lib/aurora-input/data"
cp -a "$DATA_DIR/." "$S/usr/lib/aurora-input/data/"

# 应用兼容规则是平台专属策略：Windows 版 compat.toml 里全是 Windows 宿主的修正（`xxx.exe`
# 进程名、TSF 组合帧、Band 层级），不能随 Linux 包带。换成「字段说明 + 零条内置规则」的
# Linux 版（生成脚本自检不得残留规则表）。
bash "$REPO/scripts/lib/gen-compat.sh" linux "$DATA_DIR/compat.toml" \
    "$S/usr/lib/aurora-input/data/compat.toml"

# 应用菜单入口 + 协议 / 包文件关联
install -Dm644 "$REPO/scripts/linux/pkg/aurora-input-setting.desktop" \
    "$S/usr/share/applications/aurora-input-setting.desktop"
install -Dm644 "$REPO/scripts/linux/pkg/aurora-input-import.desktop" \
    "$S/usr/share/applications/aurora-input-import.desktop"
install -Dm644 "$REPO/scripts/linux/pkg/aurora-input-mime.xml" \
    "$S/usr/share/mime/packages/aurora-input.xml"

# Fcitx5 系统级默认值：吞掉它的「Wayland 诊断」通知（wayland-diagnose-gnome）。
# 那条提示是 fcitx5 自己发的（libwayland 的 selfDiagnose：Wayland 会话 + GNOME + 当前 UI 不是
# kimpanel 就发，5.1.2 起才有），内容是让用户装 kimpanel Shell 扩展；对清风不成立——候选窗
# 走本 addon 自己的 X11 / layer-shell 面板，不经 Fcitx5 的 InputPanel，装不装 kimpanel 都一样。
# 落点必须是 /etc/xdg（StandardPath::PkgConfig 的 XDG_CONFIG_DIRS 层）而不是用户目录：不能替
# 用户写 ~/.config。读取路径 readAsIni → StandardPath::open 是「用户目录命中即停、整份不合并」，
# 所以这份只是默认值：用户点过「不要再显示」或被设置工具写过 ~/.config/fcitx5/conf/notifications.conf
# 时，用户那份整份生效、本文件被忽略（不会把用户自己的列表合并回来，也就不会关不掉）。
install -Dm644 "$REPO/scripts/linux/pkg/fcitx5-notifications.conf" \
    "$S/etc/xdg/fcitx5/conf/notifications.conf"

# 许可证与说明：GPL-3.0 §4 要求分发时附带许可证全文。NOTICE.md 是第三方词库的许可声明
# （rime-frost / OpenCC / rime-stroke / rime-emoji 各有不同许可证，正文在数据文件旁）。
# 源文件在仓库根，git 里带 CRLF（.gitattributes 声明本仓 core.autocrlf=true）。
install -Dm644 "$REPO/LICENSE"   "$S/usr/share/licenses/aurora-input/LICENSE"
install -Dm644 "$REPO/NOTICE.md" "$S/usr/share/licenses/aurora-input/NOTICE.md"
install -Dm644 "$REPO/aurora_linux/README.md" "$S/usr/share/doc/aurora-input/README.md"

# 权限归一：`cp -a` 把宿主 umask（002 → 0775/0664）原样带进包，装到系统里就是 root 组可写。
# 目录 755；带执行位的文件 755、其余 644（符号链接不动）。
find "$S" -type d -exec chmod 755 {} +
find "$S" -type f -perm /111 -exec chmod 755 {} +
find "$S" -type f ! -perm /111 -exec chmod 644 {} +

# ── 5. rpmbuild ──
say "rpmbuild"
SPEC_DIR="$WORK_DIR/spec"
rm -rf "$SPEC_DIR" "$WORK_DIR/rpmbuild"
mkdir -p "$SPEC_DIR" "$WORK_DIR/rpmbuild"
# 模板占位替换。/ 会破坏 sed 的 s///，版本号里不会有；arch/version 都做转义。
# changelog 日期用 rpm 认的英文月份缩写 + 当地区域日期（不是 `date -R`：那个带时区偏移，
# %changelog 只认 `Wed Oct 30 2026` 这种定长格式，格式不对 rpmbuild 会报「虚假的日期」警告）。
DATE_RFC="$(LC_ALL=C date '+%a %b %e %Y')"
sed -e "s/@VERSION@/$VERSION/g" \
    -e "s/@RELEASE@/$RELEASE/g" \
    -e "s/@ARCH@/$ARCH/g" \
    -e "s/@CHANGELOG_DATE@/$DATE_RFC/g" \
    "$REPO/scripts/linux/aurora-input.spec.in" >"$SPEC_DIR/aurora-input.spec"

rpmbuild -bb "$SPEC_DIR/aurora-input.spec" \
    --define "_topdir $WORK_DIR/rpmbuild" \
    --define "buildroot_src $S" \
    --define "debug_package %{nil}" \
    --define "_build_id_links none" \
    --quiet

RPM_BUILT="$(find "$WORK_DIR/rpmbuild/RPMS" -name 'aurora-input-*.rpm' -type f | head -1)"
[[ -n "$RPM_BUILT" ]] || { err "rpmbuild 没有产出 .rpm"; exit 1; }

# 发布资产名与 .deb / Windows / macOS 同口径：AuroraInput-<版本>-linux-<架构>.<扩展名>
# + 同名 .sha256 sidecar（格式对齐 sha256sum，下游校验脚本按第一列取值）。
REL_NAME="AuroraInput-${VERSION}-linux-${ARCH}.rpm"
cp -f "$RPM_BUILT" "$OUT_DIR/$REL_NAME"
(cd "$OUT_DIR" && sha256sum "$REL_NAME" > "$REL_NAME.sha256")

say "完成"
ls -lh "$OUT_DIR/$REL_NAME" "$OUT_DIR/$REL_NAME.sha256"
