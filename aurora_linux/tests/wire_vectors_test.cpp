// 线格式真值表在 Linux 前端侧的消费测试。
//
// 与 `aurora_core/crates/aurora-ipc/tests/wire_vectors.rs` 读**同一份**
// `aurora_core/crates/aurora-ipc/testdata/wire_vectors.json`：那份由 Rust 真源编码器
// 重录，这里逐字节比对本端解码器读出来的东西。
//
// 为什么要有这一份：Linux `Codec.cpp` 是 Rust 编解码器的第三份实现（另外两份是
// TSF 的 `IPCClient.cpp` 与 Swift 的 `BinaryCodec.swift`）。命令码有机器对账
// （`protocol_sync_test.cpp`），但 **payload 的字节布局没有**——字段顺序、有无长度
// 前缀、UTF-16 还是 UTF-8 计数，全靠读对方源码对齐。错了不报错：`textLen` 恰好落在
// `compLen` 上就上屏一串错字，日志里一个字都没有。
//
// 本文件不重录、只消费。要改线格式请改 Rust 真源并重录：
//   AURORA_WIRE_BLESS=1 cargo test -p aurora-ipc --test wire_vectors

#include "Codec.h"
#include "ExtProtocol.h"
#include "HostUi.h"
#include "Protocol.h"
#include "TestHarness.h"

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <optional>
#include <sstream>
#include <string>
#include <vector>

#ifndef AURORA_WIRE_VECTORS
#error "编译时需 -DAURORA_WIRE_VECTORS=\"<仓库>/aurora_core/crates/aurora-ipc/testdata/wire_vectors.json\""
#endif

namespace {

struct VectorCase {
    std::string name;
    std::string why;
    uint16_t cmd = 0;
    std::string producer;
    windlinux::Bytes bytes;
};

std::string readFile(const char* path)
{
    std::ifstream f(path, std::ios::binary);
    if (!f) {
        std::printf("  无法打开真值表 %s\n", path);
        return {};
    }
    std::ostringstream ss;
    ss << f.rdbuf();
    return ss.str();
}

/// 取 `"key": "value"` 里那个 JSON 字符串的值。真值表由 Rust 侧生成、结构固定且
/// 不含转义引号，故手写解析足够；**取不到返回空**让调用方报红，而不是静默跳过。
std::string jsonStr(const std::string& obj, const std::string& key)
{
    const std::string needle = "\"" + key + "\":";
    size_t at = obj.find(needle);
    if (at == std::string::npos) {
        return {};
    }
    at += needle.size();
    while (at < obj.size() && (obj[at] == ' ' || obj[at] == '\t')) {
        at++;
    }
    if (at >= obj.size() || obj[at] != '"') {
        return {};
    }
    at++;
    std::string out;
    while (at < obj.size() && obj[at] != '"') {
        // 只处理生成器会产出的两种转义。
        if (obj[at] == '\\' && at + 1 < obj.size()) {
            const char n = obj[at + 1];
            out += (n == 'n') ? '\n' : (n == 't') ? '\t' : n;
            at += 2;
            continue;
        }
        out += obj[at++];
    }
    return out;
}

long jsonInt(const std::string& obj, const std::string& key)
{
    const std::string needle = "\"" + key + "\":";
    size_t at = obj.find(needle);
    if (at == std::string::npos) {
        return -1;
    }
    return std::strtol(obj.c_str() + at + needle.size(), nullptr, 10);
}

windlinux::Bytes fromHex(const std::string& h)
{
    windlinux::Bytes out;
    out.reserve(h.size() / 2);
    for (size_t i = 0; i + 1 < h.size(); i += 2) {
        out.push_back(static_cast<uint8_t>(std::stoul(h.substr(i, 2), nullptr, 16)));
    }
    return out;
}

std::string toHex(const windlinux::Bytes& b)
{
    static const char* kHex = "0123456789abcdef";
    std::string s;
    s.reserve(b.size() * 2);
    for (uint8_t v : b) {
        s += kHex[v >> 4];
        s += kHex[v & 0x0F];
    }
    return s;
}

/// 按顶层对象切出 `cases` 数组里每个 `{...}`。
std::vector<VectorCase> loadCases()
{
    const std::string text = readFile(AURORA_WIRE_VECTORS);
    std::vector<VectorCase> out;
    size_t pos = text.find("\"cases\"");
    if (text.empty() || pos == std::string::npos) {
        return out;
    }
    while (true) {
        const size_t open = text.find('{', pos);
        if (open == std::string::npos) {
            break;
        }
        const size_t close = text.find('}', open);
        if (close == std::string::npos) {
            break;
        }
        const std::string obj = text.substr(open, close - open + 1);
        VectorCase c;
        c.name = jsonStr(obj, "name");
        c.why = jsonStr(obj, "why");
        c.cmd = static_cast<uint16_t>(jsonInt(obj, "cmd"));
        c.producer = jsonStr(obj, "producer");
        c.bytes = fromHex(jsonStr(obj, "hex"));
        // 数组结束标志：'}' 之后紧跟的 ']' 之前没有更多 '{'。
        if (c.name.empty() || c.bytes.empty()) {
            break;
        }
        out.push_back(std::move(c));
        pos = close + 1;
    }
    return out;
}

} // namespace

int main()
{
    const std::vector<VectorCase> cases = loadCases();
    if (cases.empty()) {
        std::printf("  真值表为空或解析失败，检查 -DAURORA_WIRE_VECTORS 指向的文件\n");
        return 1;
    }
    std::printf("载入 %zu 条线格式用例\n", cases.size());

    // ── 头：每条用例的 8 字节头都要能被本端 decodeHeader 接受，且长度自洽 ──
    for (const auto& c : cases) {
        CASE(c.name.c_str());
        if (c.bytes.size() < windlinux::HEADER_SIZE) {
            CHECK(false);
            continue;
        }
        windlinux::HeaderInfo h;
        CHECK(windlinux::decodeHeader(c.bytes.data(), h) == windlinux::HeaderError::None);
        CHECK_EQ(h.cmd, c.cmd);
        CHECK_EQ(h.length, static_cast<uint32_t>(c.bytes.size() - windlinux::HEADER_SIZE));
    }

    // ── 上行：本端编码器产出的字节必须与真值表逐字节相同 ──
    for (const auto& c : cases) {
        if (c.producer != "client") {
            continue;
        }
        // CASE 只存指针，故带前缀的名字要先落成一个具名局部量：
        // `CASE(("encode " + c.name).c_str())` 的临时 std::string 在完整表达式结束即销毁。
        const std::string encodeCase = "encode " + c.name;
        CASE(encodeCase.c_str());
        if (c.name == "key_event_all_fields_distinct") {
            // 真值表用 Rust 解码器期望的字段反推：keyCode=0x41 scanCode=0x1E
            // modifiers=0x0005 type=0 toggles=0x03 seq=0x1234 prevChar=0x4E2D
            windlinux::KeyEvent e;
            e.keyCode = 0x41;
            e.scanCode = 0x1E;
            e.modifiers = 0x0005;
            e.eventType = 0;
            e.toggles = 0x03;
            e.eventSeq = 0x1234;
            e.prevChar = 0x4E2D;
            CHECK_EQ(toHex(windlinux::encodeKeyEventFrame(e)), toHex(c.bytes));
        }
    }

    // ── 下行：本端解码器读出来的内容必须与真值表描述的一致 ──
    for (const auto& c : cases) {
        if (c.producer != "service") {
            continue;
        }
        const windlinux::Bytes payload(c.bytes.begin() + windlinux::HEADER_SIZE, c.bytes.end());
        const std::string decodeCase = "decode " + c.name;
        CASE(decodeCase.c_str());

        if (c.name == "commit_text_plain") {
            auto p = windlinux::decodeCommitText(payload);
            CHECK(p.has_value());
            if (p) {
                CHECK_EQ(p->flags, 0x04u); // chineseMode
                CHECK_EQ(p->text, std::string("hi"));
                CHECK_EQ(p->newComposition, std::string());
            }
        } else if (c.name == "commit_text_with_new_composition") {
            auto p = windlinux::decodeCommitText(payload);
            CHECK(p.has_value());
            if (p) {
                CHECK_EQ(p->flags, 0x07u); // modeChanged | hasNewComposition | chineseMode
                CHECK_EQ(p->text, std::string("hi"));
                CHECK_EQ(p->newComposition, std::string("hao"));
            }
        } else if (c.name == "commit_text_replacing_held") {
            auto p = windlinux::decodeCommitText(payload);
            CHECK(p.has_value());
            if (p) {
                CHECK_EQ(p->flags, 0x0Cu); // chineseMode | replacingHeld(bit3)
                CHECK_EQ(p->text, std::string(","));
                CHECK_EQ(p->newComposition, std::string());
            }
        } else if (c.name == "update_composition_ascii") {
            auto p = windlinux::decodeUpdateComposition(payload);
            CHECK(p.has_value());
            if (p) {
                CHECK_EQ(p->caretPos, 2u);
                CHECK_EQ(p->text, std::string("ni"));
            }
        } else if (c.name == "commit_text_with_cursor") {
            auto p = windlinux::decodeCommitTextWithCursor(payload);
            CHECK(p.has_value());
            if (p) {
                CHECK_EQ(p->text, std::string("hi!"));
                CHECK_EQ(p->cursorOffset, 2u);
            }
        } else if (c.name == "move_cursor_count") {
            auto p = windlinux::decodeMoveCursor(payload);
            CHECK(p.has_value());
            if (p) {
                CHECK_EQ(*p, 3u);
            }
        } else if (c.name == "candidate_rects_two") {
            auto r = windlinux::decodeCandidateRects(payload);
            CHECK(r.has_value());
            if (r) {
                CHECK_EQ(r->size(), size_t(2));
                if (r->size() == 2) {
                    CHECK_EQ((*r)[0].index, 0);
                    CHECK_EQ((*r)[0].x, 0);
                    CHECK_EQ((*r)[0].y, 10);
                    CHECK_EQ((*r)[0].w, 20);
                    CHECK_EQ((*r)[0].h, 24);
                    CHECK_EQ((*r)[1].index, 1);
                    CHECK_EQ((*r)[1].x, 10);
                    CHECK_EQ((*r)[1].y, 46);
                }
            }
        } else if (c.name == "mode_push_all_flags") {
            windlinux::Frame f;
            f.cmd = c.cmd;
            f.payload = payload;
            auto m = windlinux::modeStatusOf(f);
            CHECK(m.has_value());
            if (m) {
                CHECK(m->chinese);
            }
        } else if (c.name == "mode_status_with_label") {
            // CMD_MODE_STATUS 走 HostUi 的状态帧分支；它要求 12 字节起的状态头，
            // 而本条只有 flags+effectiveMode+label，故本端不认——这正是**有意保留的差异**：
            // Linux 用 MODE_PUSH 学中英，不消费 MODE_STATUS 的标签（标签走完整状态帧）。
            // 这里断言的是「不崩溃、安静返回」，不是「读出来」。
            windlinux::Frame f;
            f.cmd = c.cmd;
            f.payload = payload;
            (void)windlinux::modeStatusOf(f);
            CHECK(true);
        } else if (c.name == "ext_envelope") {
            auto e = windlinux::decodeExt(payload);
            CHECK(e.has_value());
            if (e) {
                CHECK_EQ(e->kind, std::string("notify"));
                CHECK_EQ(toHex(e->body), std::string("6869")); // "hi"
            }
        }
    }

    TEST_MAIN_END()
}
