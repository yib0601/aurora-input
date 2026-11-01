# 纯 C++ 单测的兜底构建（不需要 cmake、不需要 Fcitx5）。addon 本体走 CMakeLists.txt。
#
#   make -C wind_linux test          # 编译并运行全部单测
#   make -C wind_linux clean
#
# 产物落在 $(BUILD)（默认 aurora_linux/build/tests，已 gitignore）。

CXX ?= g++
BUILD ?= build/tests
CXXFLAGS ?= -std=c++17 -O1 -g -Wall -Wextra -Werror -pthread
# 路径与 CMakeLists.txt 的同名定义保持一字不差（两边各编一遍同一批测试，参数漂了就有一边跑空）。
# 注：crates 目录名是 `aurora-ipc`，不是历史上的 `wind-ipc`——改名后这里曾一直是旧名，
# 导致 make test 里 protocol_sync_test 拿不到 AURORA_PROTOCOL_RS 而直接编译失败。
# AURORA_WIRE_VECTORS：线格式真值表（真源在 aurora-ipc/testdata/），与 TSF/macOS 读同一份。
# 命令码与纯判据各有自己的真值表，那一层管不到 **payload 的字节布局**——字段顺序、
# 有无长度前缀、UTF-16 还是 UTF-8 计数，两边各写一份、错了不报错。
CPPFLAGS += -Iinclude -DAURORA_PROTOCOL_RS=\"$(abspath ../aurora_core/crates/aurora-ipc/src/protocol.rs)\" \
	-DAURORA_POLICY_JSON=\"$(abspath ../aurora_core/crates/aurora-punct/testdata/cross_platform_policy_vectors.json)\" \
	-DAURORA_WIRE_VECTORS=\"$(abspath ../aurora_core/crates/aurora-ipc/testdata/wire_vectors.json)\" \
	-DAURORA_REPO_DIR=\"$(abspath ..)\"

CORE_SRC := $(wildcard src/core/*.cpp)
TESTS := codec_test keymap_test router_test bridge_test protocol_sync_test shm_test launcher_test settings_launcher_test menu_test host_ui_test overlay_input_test keysynth_test gnome_caret_geom_test cross_platform_policy_test wire_vectors_test

.PHONY: test clean
test: $(addprefix $(BUILD)/,$(TESTS))
	@fail=0; for t in $(TESTS); do \
		echo "=== $$t"; \
		$(BUILD)/$$t || fail=1; \
	done; \
	if [ $$fail -eq 0 ]; then echo "单测全部通过"; else echo "单测有失败"; exit 1; fi

$(BUILD)/%: tests/%.cpp $(CORE_SRC) $(wildcard include/*.h) tests/TestHarness.h
	@mkdir -p $(BUILD)
	$(CXX) $(CXXFLAGS) $(CPPFLAGS) -o $@ $< $(CORE_SRC)

clean:
	rm -rf $(BUILD)
