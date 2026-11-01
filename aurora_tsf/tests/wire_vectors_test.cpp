// 线格式真值表在 Windows TSF DLL 侧的消费测试。
//
// 与 `aurora_core/crates/aurora-ipc/tests/wire_vectors.rs` 读**同一份**
// `aurora_core/crates/aurora-ipc/testdata/wire_vectors.json`：那份由 Rust 真源编码器
// 重录，这里逐字节比对本端结构体与真值表。
//
// 为什么 TSF 这一份最要紧：`SendKeyEvent` 是直接把 `KeyPayload` 结构体 `memcpy` 进帧的
// （`_SendBinaryMessage(CMD_KEY_EVENT, &payload, sizeof(payload))`），Rust 那边逐字段解码。
// 两边没有任何编译期约束把它们钉在一起——加一个字段、调一次顺序，编译全过，运行时只表现为
// 「某个键的行为不对」。`static_assert(sizeof(...))` 也管不到：两个 `uint16_t` 互换顺序、
// 或者把 `uint8_t` 塞进 `uint32_t` 的洞，字节数一模一样。
//
// 本文件不含任何 Win32/COM 头，g++ 在 Linux 上直接编译——与 `tests/*_policy_test.cpp`
// 同一套路，CI 的 Linux job 也会跑。
//
// 跑法（纯 C++17、不含 Win32 头，本机不需要 MSVC）：
//   g++ -std=c++17 -Iinclude -Itests
//       -DAURORA_WIRE_VECTORS='"'"$PWD/../aurora_core/crates/aurora-ipc/testdata/wire_vectors.json"'"'
//       -o /tmp/wire_vectors_test tests/wire_vectors_test.cpp && /tmp/wire_vectors_test
//
// CI 与 `make -C aurora_tsf test-cpp` 都会带这两个参数编译本文件。
//
// 本文件不重录、只消费。要改线格式请改 Rust 真源并重录：
//   AURORA_WIRE_BLESS=1 cargo test -p aurora-ipc --test wire_vectors

// `BinaryProtocol.h` 是纯标准 C++，唯一不可移植的是 inline `GetCurrentModifiers()`（调
// Win32 `GetAsyncKeyState`）。inline 函数体即使不被调用也要能编过，故把那些符号垫成常量
// 再 #undef。这与 `aurora_linux/include/Protocol.h` 是同一手法，理由见那个文件的注释。
// **本文件不调用 `GetCurrentModifiers()`**：这里只比结构体布局，不涉及真实按键状态。
#define GetAsyncKeyState(vk) (static_cast<short>(0))
#define VK_SHIFT 0
#define VK_CONTROL 0
#define VK_MENU 0
#define VK_LWIN 0
#define VK_RWIN 0
#define VK_LSHIFT 0
#define VK_RSHIFT 0
#define VK_LCONTROL 0
#define VK_RCONTROL 0
#include "BinaryProtocol.h"
#undef GetAsyncKeyState
#undef VK_SHIFT
#undef VK_CONTROL
#undef VK_MENU
#undef VK_LWIN
#undef VK_RWIN
#undef VK_LSHIFT
#undef VK_RSHIFT
#undef VK_LCONTROL
#undef VK_RCONTROL

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

#ifndef AURORA_WIRE_VECTORS
#error "编译时需 -DAURORA_WIRE_VECTORS=\"<仓库>/aurora_core/crates/aurora-ipc/testdata/wire_vectors.json\""
#endif

namespace
{

int g_failures = 0;
const char* g_case = "";

#define CHECK(expr)                                                                    \
    do {                                                                               \
        if (!(expr)) {                                                                 \
            std::printf("  FAIL  %s:%d  [%s]  %s\n", __FILE__, __LINE__, g_case, #expr); \
            g_failures++;                                                              \
        }                                                                              \
    } while (0)

#define CHECK_EQ(a, b)                                                                 \
    do {                                                                               \
        const auto _va = (a);                                                          \
        const auto _vb = (b);                                                          \
        if (!(_va == _vb)) {                                                           \
            std::printf("  FAIL  %s:%d  [%s]  %s == %s\n", __FILE__, __LINE__, g_case,  \
                        #a, #b);                                                           \
            g_failures++;                                                              \
        }                                                                              \
    } while (0)

#define CASE(name)                                                                     \
    g_case = name;                                                                     \
    std::printf("- %s\n", name);

struct VectorCase
{
    std::string name;
    std::string why;
    uint16_t cmd = 0;
    std::string producer;
    std::vector<uint8_t> bytes;
};

std::string readFile(const char* path)
{
    std::ifstream f(path, std::ios::binary);
    if (!f)
    {
        std::printf("  无法打开真值表 %s\n", path);
        return {};
    }
    std::ostringstream ss;
    ss << f.rdbuf();
    return ss.str();
}

/// 取 `"key": "value"` 里的字符串值。真值表由 Rust 侧生成、结构固定，手写解析足够；
/// **取不到返回空**让调用方报红，而不是静默跳过。
std::string jsonStr(const std::string& obj, const std::string& key)
{
    const std::string needle = "\"" + key + "\":";
    const size_t at = obj.find(needle);
    if (at == std::string::npos)
    {
        return {};
    }
    size_t i = at + needle.size();
    while (i < obj.size() && (obj[i] == ' ' || obj[i] == '\t'))
    {
        i++;
    }
    if (i >= obj.size() || obj[i] != '"')
    {
        return {};
    }
    i++;
    std::string out;
    while (i < obj.size() && obj[i] != '"')
    {
        if (obj[i] == '\\' && i + 1 < obj.size())
        {
            const char n = obj[i + 1];
            out += (n == 'n') ? '\n' : (n == 't') ? '\t' : n;
            i += 2;
            continue;
        }
        out += obj[i++];
    }
    return out;
}

long jsonInt(const std::string& obj, const std::string& key)
{
    const std::string needle = "\"" + key + "\":";
    const size_t at = obj.find(needle);
    if (at == std::string::npos)
    {
        return -1;
    }
    return std::strtol(obj.c_str() + at + needle.size(), nullptr, 10);
}

std::vector<uint8_t> fromHex(const std::string& h)
{
    std::vector<uint8_t> out;
    out.reserve(h.size() / 2);
    for (size_t i = 0; i + 1 < h.size(); i += 2)
    {
        out.push_back(static_cast<uint8_t>(std::stoul(h.substr(i, 2), nullptr, 16)));
    }
    return out;
}

std::string toHex(const uint8_t* p, size_t n)
{
    static const char* kHex = "0123456789abcdef";
    std::string s;
    s.reserve(n * 2);
    for (size_t i = 0; i < n; i++)
    {
        s += kHex[p[i] >> 4];
        s += kHex[p[i] & 0x0F];
    }
    return s;
}

std::vector<VectorCase> loadCases()
{
    const std::string text = readFile(AURORA_WIRE_VECTORS);
    std::vector<VectorCase> out;
    size_t pos = text.find("\"cases\"");
    if (text.empty() || pos == std::string::npos)
    {
        return out;
    }
    while (true)
    {
        const size_t open = text.find('{', pos);
        if (open == std::string::npos)
        {
            break;
        }
        const size_t close = text.find('}', open);
        if (close == std::string::npos)
        {
            break;
        }
        const std::string obj = text.substr(open, close - open + 1);
        VectorCase c;
        c.name = jsonStr(obj, "name");
        c.why = jsonStr(obj, "why");
        c.cmd = static_cast<uint16_t>(jsonInt(obj, "cmd"));
        c.producer = jsonStr(obj, "producer");
        c.bytes = fromHex(jsonStr(obj, "hex"));
        if (c.name.empty() || c.bytes.empty())
        {
            break;
        }
        out.push_back(std::move(c));
        pos = close + 1;
    }
    return out;
}

const VectorCase* findCase(const std::vector<VectorCase>& cases, const char* name)
{
    for (const auto& c : cases)
    {
        if (c.name == name)
        {
            return &c;
        }
    }
    return nullptr;
}

} // namespace

int main()
{
    const std::vector<VectorCase> cases = loadCases();
    if (cases.empty())
    {
        std::printf("  真值表为空或解析失败，检查 -DAURORA_WIRE_VECTORS 指向的文件\n");
        return 1;
    }
    std::printf("载入 %zu 条线格式用例\n", cases.size());

    // ── IpcHeader：偏移必须逐个对，不只是 sizeof ──
    //
    // `static_assert(sizeof(IpcHeader) == 8)` 已经在头文件里，但那只保证总宽：
    // 把 `command` 与 `length` 换个位置仍然 8 字节。这里把真值表的头部逐字节比一遍。
    CASE("ipc_header_layout");
    {
        IpcHeader h{};
        h.version = PROTOCOL_VERSION;
        h.command = CMD_KEY_EVENT;
        h.length = 0x00000012;
        const uint8_t want[] = {0x01, 0x10, 0x01, 0x01, 0x12, 0x00, 0x00, 0x00};
        CHECK_EQ(toHex(reinterpret_cast<const uint8_t*>(&h), sizeof(h)), toHex(want, sizeof(want)));
    }

    // ── KeyPayload：上行最高频帧，C++ 直接 memcpy 上送 ──
    CASE("key_payload_memcpy_matches_truth");
    {
        const VectorCase* c = findCase(cases, "key_event_all_fields_distinct");
        CHECK(c != nullptr);
        if (c != nullptr)
        {
            KeyPayload p{};
            p.keyCode = 0x41;
            p.scanCode = 0x1E;
            p.modifiers = 0x0005;
            p.eventType = 0;
            p.toggles = 0x03;
            p.eventSeq = 0x1234;
            p.prevChar = 0x4E2D;
            // C++ 侧实际发送的就是这段内存：`&payload, sizeof(payload)`。
            const std::string got =
                toHex(reinterpret_cast<const uint8_t*>(&p), sizeof(p));
            const std::vector<uint8_t> wantPayload(c->bytes.begin() + 8, c->bytes.end());
            CHECK_EQ(got, toHex(wantPayload.data(), wantPayload.size()));
        }
    }

    // ── CommitTextHeader：12 字节头，字段顺序 + 宽度 ──
    CASE("commit_text_header_layout");
    {
        CommitTextHeader h{};
        h.flags = 0x07;
        h.textLength = 2;
        h.compositionLength = 3;
        const uint8_t want[] = {
            0x07, 0x00, 0x00, 0x00, // flags
            0x02, 0x00, 0x00, 0x00, // textLength
            0x03, 0x00, 0x00, 0x00, // compositionLength
        };
        CHECK_EQ(toHex(reinterpret_cast<const uint8_t*>(&h), sizeof(h)), toHex(want, sizeof(want)));
    }

    // ── CompositionHeader：只有 caretPos，文本无长度前缀 ──
    CASE("composition_header_layout");
    {
        CompositionHeader h{};
        h.caretPos = 2;
        const uint8_t want[] = {0x02, 0x00, 0x00, 0x00};
        CHECK_EQ(toHex(reinterpret_cast<const uint8_t*>(&h), sizeof(h)), toHex(want, sizeof(want)));
    }

    // ── 与真值表端到端对齐：本端结构体的字节 == 真源产出的载荷字节 ──
    //
    // 上面几条是「结构体自身布局」，这条把「真源实际发什么」与「本端读什么」接起来：
    // 真值表的载荷长度必须等于本端结构体的大小（定长帧），或按本端口径能切出变长尾部。
    CASE("payload_sizes_match_vectors");
    {
        // 定长帧：载荷长度 == 结构体大小，一个字节都不多。
        struct
        {
            const char* name;
            size_t fixed;
        } kFixed[] = {
            {"move_cursor_count", sizeof(MoveCursorPayload)},
        };
        for (const auto& f : kFixed)
        {
            const VectorCase* c = findCase(cases, f.name);
            CHECK(c != nullptr);
            if (c != nullptr)
            {
                CHECK_EQ(c->bytes.size() - 8, f.fixed);
            }
        }

        // `update_composition` **不是**定长帧：4 字节头 + 余下全是文本（无长度前缀）。
        // 把它当 sizeof(CompositionHeader) 核对是错的——正是最容易犯的那类错。
        const VectorCase* uc = findCase(cases, "update_composition_ascii");
        CHECK(uc != nullptr);
        if (uc != nullptr)
        {
            CHECK(uc->bytes.size() > 8 + sizeof(CompositionHeader));
            if (uc->bytes.size() > 8 + sizeof(CompositionHeader))
            {
                CompositionHeader h{};
                std::memcpy(&h, uc->bytes.data() + 8, sizeof(h));
                CHECK_EQ(h.caretPos, 2);
                // 尾部即文本 "ni"，长度 = 载荷长 − 头长（没有长度前缀可读）。
                CHECK_EQ(uc->bytes.size() - 8 - sizeof(h), size_t(2));
            }
        }

        // 变长帧：头 + 文本。CommitText 是 12 字节头 + textLen + compLen。
        const VectorCase* ct = findCase(cases, "commit_text_plain");
        CHECK(ct != nullptr);
        if (ct != nullptr)
        {
            CHECK(ct->bytes.size() >= 8 + sizeof(CommitTextHeader));
            if (ct->bytes.size() >= 8 + sizeof(CommitTextHeader))
            {
                CommitTextHeader h{};
                std::memcpy(&h, ct->bytes.data() + 8, sizeof(h));
                CHECK_EQ(h.textLength, 2u);          // "hi"
                CHECK_EQ(h.compositionLength, 0u);
                CHECK_EQ(ct->bytes.size() - 8 - sizeof(h), size_t(h.textLength + h.compositionLength));
            }
        }

        const VectorCase* cc = findCase(cases, "commit_text_with_cursor");
        CHECK(cc != nullptr);
        if (cc != nullptr && cc->bytes.size() >= 8 + sizeof(CommitTextWithCursorPayload))
        {
            CommitTextWithCursorPayload h{};
            std::memcpy(&h, cc->bytes.data() + 8, sizeof(h));
            CHECK_EQ(h.textLength, 3u);   // "hi!"
            CHECK_EQ(h.cursorOffset, 2u);
            CHECK_EQ(cc->bytes.size() - 8 - sizeof(h), size_t(h.textLength));
        }
    }

    // ── 命令码：真值表里的 cmd 必须与本端同名常量一致 ──
    //
    // 命令码本身由 `gen_protocol` 与 `toggle_bits_match_cpp` 对账；这里再核一遍是为了让
    // 「真值表的这一行对应哪个本端常量」有机器可读的答案，而不是靠读者去查表。
    CASE("cmd_constants_match_vectors");
    {
        struct
        {
            const char* name;
            uint16_t cmd;
        } kMap[] = {
            {"key_event_all_fields_distinct", CMD_KEY_EVENT},
            {"commit_text_plain", CMD_COMMIT_TEXT},
            {"commit_text_with_new_composition", CMD_COMMIT_TEXT},
            {"commit_text_replacing_held", CMD_COMMIT_TEXT},
            {"update_composition_ascii", CMD_UPDATE_COMPOSITION},
            {"commit_text_with_cursor", CMD_COMMIT_TEXT_WITH_CURSOR},
            {"move_cursor_count", CMD_MOVE_CURSOR},
            // CANDIDATE_RECTS / MODE_STATUS 是 macOS / Linux 前端专用（TSF 由宿主画候选、
            // 由语言栏自己的接口报模式），故不在本端 BinaryProtocol.h 里——真值表里那两条
            // 由 `aurora_linux/tests/wire_vectors_test.cpp` 负责核。
            {"mode_push_all_flags", CMD_MODE_PUSH},
            {"ext_envelope", CMD_EXT},
        };
        for (const auto& m : kMap)
        {
            const VectorCase* c = findCase(cases, m.name);
            CHECK(c != nullptr);
            if (c != nullptr)
            {
                CHECK_EQ(c->cmd, m.cmd);
            }
        }
    }

    // ── 状态位：MODE_PUSH 的三个 bit 必须与本端判据一致 ──
    CASE("status_bits_match_vectors");
    {
        const VectorCase* c = findCase(cases, "mode_push_all_flags");
        CHECK(c != nullptr);
        if (c != nullptr && c->bytes.size() == 8 + 4)
        {
            uint32_t flags = 0;
            std::memcpy(&flags, c->bytes.data() + 8, 4);
            CHECK_EQ(flags, STATUS_CHINESE_MODE | STATUS_FULL_WIDTH | STATUS_CHINESE_PUNCT);
        }
    }

    if (g_failures == 0)
    {
        std::printf("wire_vectors_test (TSF): all passed\n");
        return 0;
    }
    std::printf("wire_vectors_test (TSF): %d failure(s)\n", g_failures);
    return 1;
}
