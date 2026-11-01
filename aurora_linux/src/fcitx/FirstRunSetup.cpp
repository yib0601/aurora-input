#include "FirstRunSetup.h"

#include "AuroraLog.h"
#include "GnomeCaretGeom.h"

#include <fcitx-config/rawconfig.h>
#include <fcitx/globalconfig.h>
#include <fcitx/inputmethodgroup.h>
#include <fcitx/inputmethodmanager.h>
#include <fcitx/instance.h>

#include <fcntl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include <cstdio>
#include <cstdlib>
#include <string>

namespace windlinux {

namespace {

/// 本 addon 提供的输入法在 Fcitx5 里的唯一名（= data/aurorainput.conf.in 的 Addon= 那一行
/// 去掉 .conf 后缀之后 Fcitx5 用的那个名字，也就是 profile 里 Items 的 Name）。
constexpr const char* kInputMethodName = "aurorainput";
/// 标记文件：写完就不再自动改任何 Fcitx5 配置。位置用 XDG_CONFIG_HOME，与 fcitx5 的
/// `~/.config/fcitx5` 同一棵树，用户想撤销时能一眼找到并删除。
std::string markerPath()
{
    const char* xdg = std::getenv("XDG_CONFIG_HOME");
    std::string base;
    if (xdg && xdg[0] == '/') {
        base = xdg;
    } else if (const char* home = std::getenv("HOME"); home && home[0] == '/') {
        base = std::string(home) + "/.config";
    } else {
        return {};
    }
    return base + "/fcitx5/aurorainput-setup-done";
}

bool markerExists(const std::string& path)
{
    if (path.empty()) {
        return false;
    }
    struct stat st {};
    return ::stat(path.c_str(), &st) == 0 && S_ISREG(st.st_mode);
}

void writeMarker(const std::string& path)
{
    if (path.empty()) {
        return;
    }
    // 目录已存在（fcitx5 自己会建）；万一不存在就补建，失败也无所谓——下次还会重试。
    const auto slash = path.rfind('/');
    if (slash != std::string::npos) {
        ::mkdir(path.substr(0, slash).c_str(), 0755);
    }
    if (FILE* f = std::fopen(path.c_str(), "w")) {
        std::fputs("极光输入法首启引导已完成。\n"
                   "删除本文件，下次 Fcitx5 启动时会重新做一次自动配置\n"
                   "（把「极光输入法」加回输入法组、并设共享输入状态）。\n",
                   f);
        std::fclose(f);
    }
}

/// fcitx5 的用户配置文件路径（`$XDG_CONFIG_HOME/fcitx5/config`，回退 `~/.config`）。
std::string configFilePath()
{
    const char* xdg = std::getenv("XDG_CONFIG_HOME");
    if (xdg && xdg[0] == '/') {
        return std::string(xdg) + "/fcitx5/config";
    }
    if (const char* home = std::getenv("HOME"); home && home[0] == '/') {
        return std::string(home) + "/.config/fcitx5/config";
    }
    return {};
}

/// 该键是否在用户配置文件里**显式存在**（未被注释掉）。
///
/// 用来区分「从没设过、取的是出厂值」与「用户主动选了那个值」——两者读出来的当前值一样
/// （都是 No / False），只看值分不开。用户动过这两个开关就说明是有意选择，引导不该覆盖。
bool keyWrittenByUser(const std::string& section, const std::string& key)
{
    const auto path = configFilePath();
    if (path.empty()) {
        return false;
    }
    FILE* f = std::fopen(path.c_str(), "r");
    if (!f) {
        return false;
    }
    const std::string wantSection = "[" + section + "]";
    const std::string wantKey = key + "=";
    bool inSection = false;
    bool found = false;
    char line[512];
    while (!found && std::fgets(line, sizeof(line), f)) {
        std::string s(line);
        // 去掉行尾空白，便于判 [Section]
        while (!s.empty() && (s.back() == '\n' || s.back() == '\r' || s.back() == ' ')) {
            s.pop_back();
        }
        if (!s.empty() && s.front() == '[') {
            inSection = (s == wantSection);
            continue;
        }
        if (!inSection) {
            continue;
        }
        // 注释行不算；键名后紧跟 '=' 才算（避免 CustomMappings_A 之类误命中）。
        if (!s.empty() && s.front() == '#') {
            continue;
        }
        if (s.rfind(wantKey, 0) == 0) {
            found = true;
        }
    }
    std::fclose(f);
    return found;
}

/// 组里是否已经有极光。命中就不动这个组——用户可能刚把它删掉，标记文件负责记「别再管了」。
bool groupHasAurora(const fcitx::InputMethodGroup& group)
{
    for (const auto& item : group.inputMethodList()) {
        if (item.name() == kInputMethodName) {
            return true;
        }
    }
    return false;
}

/// 第 1 步：把极光加进当前组（放在键盘之后），并让它成为该组的默认输入法。
///
/// 只补不改：组里已有极光时直接返回，连默认输入法都不动。
bool addSelfToGroup(fcitx::Instance* instance)
{
    auto& mgr = instance->inputMethodManager();
    const auto& current = mgr.currentGroup();
    if (groupHasAurora(current)) {
        AURORA_INFO() << "首启引导：输入法组里已有极光，跳过";
        return false;
    }

    fcitx::InputMethodGroup group(current.name());
    group.setDefaultLayout(current.defaultLayout());
    // 原样保留用户现有的每一项（布局设置也照抄），再把极光追加到最后。
    for (const auto& item : current.inputMethodList()) {
        group.inputMethodList().emplace_back(item.name()).setLayout(item.layout());
    }
    group.inputMethodList().emplace_back(kInputMethodName).setLayout("");
    // 设成默认：用户第一次激活就是中文，不必先切一次。
    group.setDefaultInputMethod(kInputMethodName);

    const std::string groupName = current.name();
    mgr.setGroup(std::move(group));
    // setGroup 只替换组定义；**当前会话正在用的输入法**仍是旧的（keyboard-us），用户这一次
    // 登录还是英文。重设一次当前组让输入上下文按新组的 DefaultIM 重新推导。
    mgr.setCurrentGroup(groupName);
    mgr.setDefaultInputMethod(kInputMethodName);
    mgr.save();
    AURORA_INFO() << "首启引导：已把极光加入输入法组 " << current.name();
    return true;
}

/// 第 2 步：共享输入状态 = 所有（新开的应用直接是中文），默认激活 = 是。
///
/// 只在仍是出厂默认值时改：用户把它设成 Program / No 就说明是有意选择，不再覆盖。
bool preferChineseByDefault(fcitx::Instance* instance)
{
    auto& global = instance->globalConfig();

    // 只改「用户从没设过」的那一项：
    //   shareInputState 出厂 No（每个应用各记各的，新应用从键盘起步）
    //   activeByDefault 出厂 false（输入上下文不自动激活）
    // 判据是配置文件里**有没有显式写过这个键**，而不是当前值是否等于出厂值——用户主动选
    // 「No」与从未设置读出来一样，只看值会把用户的选择当成默认值覆盖掉。
    const bool shareUntouched = !keyWrittenByUser("Behavior", "ShareInputState");
    const bool activeUntouched = !keyWrittenByUser("Behavior", "ActiveByDefault");
    const bool needShare =
        shareUntouched && global.shareInputState() == fcitx::PropertyPropagatePolicy::No;
    const bool needActive = activeUntouched && !global.activeByDefault();
    if (!needShare && !needActive) {
        AURORA_INFO() << "首启引导：行为配置已是推荐值，跳过";
        return false;
    }

    // 经 RawConfig 整体回写：GlobalConfig 只有 config() 一个可写入口，而配置文件落盘走
    // save()——直接改运行时策略（setPropertyPropagatePolicy）不会反映到已保存的 config 里，
    // 下次启动又变回 No。这里读现状、只覆盖要改的两项、再 partial load + safeSave。
    fcitx::RawConfig raw;
    global.save(raw);
    if (needShare) {
        raw.setValueByPath("Behavior/ShareInputState", "All");
    }
    if (needActive) {
        raw.setValueByPath("Behavior/ActiveByDefault", "True");
    }
    global.load(raw, /*partial=*/true);
    global.safeSave();
    if (needShare) {
        // load() 只改配置树，运行中的输入上下文管理器仍持有旧策略；同步一次让「本次会话」
        // 也生效，否则用户得重启 fcitx5 才看到效果。
        instance->inputContextManager().setPropertyPropagatePolicy(
            fcitx::PropertyPropagatePolicy::All);
        AURORA_INFO() << "首启引导：共享输入状态设为「所有」";
    }
    if (needActive) {
        AURORA_INFO() << "首启引导：默认激活输入法";
    }
    return true;
}

/// 第 3 步（仅 GNOME）：启用随包安装的 GNOME Shell 扩展。
///
/// GNOME Wayland 没有 input-method-v2，候选窗要靠该扩展把焦点窗口的光标矩形折到 X11 坐标，
/// 否则窗口浮动时会偏。扩展文件由包装到 `/usr/share/gnome-shell/extensions/`，但 **装上不会
/// 自动启用**——`gnome-extensions enable` 写的是用户级 dconf，必须由用户会话执行，
/// 这正是它放在本模块而不是 postinst 的原因。
///
/// ⚠️ 绝不能在这里同步等子进程：本函数跑在 fcitx5 主线程（事件循环）上，
/// `system()` 会把它卡住——用户此刻按的键全都延迟到命令结束。故 `fork` 出子进程、
/// 父进程立刻返回；子进程 `setsid` 脱离会话、stdio 接 /dev/null，不留僵尸。
/// 反正这一步的结果不影响打字，晚一两秒启用完全无所谓（且要注销重登才生效）。
void enableGnomeCaretExtensionAsync()
{
    if (!xdgCurrentDesktopIsGnome(std::getenv("XDG_CURRENT_DESKTOP"))) {
        return;
    }
    const pid_t pid = ::fork();
    if (pid != 0) {
        // 父进程（或 fork 失败）：立刻返回，不等待。
        if (pid < 0) {
            AURORA_DEBUG() << "首启引导：fork 失败，跳过 GNOME 扩展启用";
        }
        return;
    }
    // 子进程：脱离父会话，避免被 fcitx5 的信号 / 终端影响。
    ::setsid();
    const int devnull = ::open("/dev/null", O_RDWR);
    if (devnull >= 0) {
        ::dup2(devnull, STDIN_FILENO);
        ::dup2(devnull, STDOUT_FILENO);
        ::dup2(devnull, STDERR_FILENO);
        if (devnull > STDERR_FILENO) {
            ::close(devnull);
        }
    }
    // `gnome-extensions enable` 本身幂等：已启用时直接成功。
    ::execlp("gnome-extensions", "gnome-extensions", "enable", "aurorainput-caret@aurorainput",
             static_cast<char*>(nullptr));
    // exec 失败（没装 gnome-extensions 的桌面）：静默退出，父进程早已继续。
    ::_exit(127);
}

} // namespace

void ensureFirstRunSetup(fcitx::Instance* instance)
{
    if (!instance) {
        return;
    }
    // 测试/自动化可关掉：e2e 自己有隔离的 fcitx 配置，引导去改它会让用例前提漂移。
    if (const char* off = std::getenv("AURORA_NO_FIRST_RUN_SETUP"); off && *off && *off != '0') {
        return;
    }
    // 标记文件在就不碰任何配置：用户可能故意删掉了极光、或把共享状态改回 No。
    if (const auto path = markerPath(); markerExists(path)) {
        return;
    }

    // 每条独立 try：一步失败不该把另一步也带走，更不该影响 addon 加载。
    try {
        addSelfToGroup(instance);
    } catch (const std::exception& e) {
        AURORA_WARN() << "首启引导：加入输入法组失败：" << e.what();
    } catch (...) {
        AURORA_WARN() << "首启引导：加入输入法组失败（未知异常）";
    }
    try {
        preferChineseByDefault(instance);
    } catch (const std::exception& e) {
        AURORA_WARN() << "首启引导：写行为配置失败：" << e.what();
    } catch (...) {
        AURORA_WARN() << "首启引导：写行为配置失败（未知异常）";
    }
    try {
        enableGnomeCaretExtensionAsync();
    } catch (...) {
        AURORA_WARN() << "首启引导：启用 GNOME 扩展失败";
    }

    // 无论上面成败都落标记：失败通常意味着这个环境（如权限）本就做不了，
    // 每次启动重试只会刷日志。用户可删除标记让下次再试一次。
    writeMarker(markerPath());
}

} // namespace windlinux
