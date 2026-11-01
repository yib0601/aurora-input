// addon 的日志分类与宏。
//
// 分类定义（FCITX_DEFINE_LOG_CATEGORY）必须只出现在一个编译单元里，所以放在 AuroraLog.cpp；
// 其它文件只 include 本头拿到声明与宏。原先两个宏直接写在 WindEngine.cpp 里，新增的
// FirstRunSetup.cpp 也要记日志时就得复制一份——两个 TU 各定义一次分类会撞符号，故抽出来。

#pragma once

#include <fcitx-utils/log.h>

FCITX_DECLARE_LOG_CATEGORY(aurora_log);

#define AURORA_DEBUG() FCITX_LOGC(aurora_log, Debug)
#define AURORA_INFO() FCITX_LOGC(aurora_log, Info)
#define AURORA_WARN() FCITX_LOGC(aurora_log, Warn)
#define AURORA_ERROR() FCITX_LOGC(aurora_log, Error)
