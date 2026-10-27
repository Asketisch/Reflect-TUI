# ── ZhuYe-CLI Makefile ──────────────────────────────────────────
# 终端 TUI 独立仓库 —— Reflect-Agent 核心 crate 通过 submodule 复用。
.PHONY: build run test test-fast test-suite test-suite-fast test-suite-live coverage-audit release-check check check-fast watch clean gc dist dist-dry install release help

BINARY_NAME := zhuye-cli

## build: Compile the TUI in release mode
build:
	cargo build --release -p zhuye-cli

## run: Build and launch the TUI directly
run: build
	./target/release/$(BINARY_NAME)

## install: Build + install to /usr/local/bin (or ~/.local/bin with --local)
install:
	./scripts/install.sh

## dist: 构建并打包当前平台分发包(tar.gz/zip)到 dist/
dist:
	./scripts/build.sh --dist

## dist-dry: 预览 dist 打包命令(不实际执行)
dist-dry:
	./scripts/build.sh --dist --dry-run

## release: 完整 release 构建(LTO,小体积),不打包
release:
	./scripts/build.sh

## test: Run all tests (cargo test)
test:
	cargo test --workspace

## test-fast: Run all tests via nextest(需 cargo-nextest)
test-fast:
	cargo nextest run --workspace

## test-suite: 跑全功能测试套件(能力覆盖层 + Rust 侧 + 覆盖自审)
test-suite:
	./tests/run_all.sh

## test-suite-fast: 只跑能力覆盖层(跳过 cargo test/clippy/fmt/doc)
test-suite-fast:
	./tests/run_all.sh functional

## test-suite-live: live 端到端(需真实 LLM + API key)
test-suite-live:
	./tests/run_all.sh live

## coverage-audit: 覆盖自审(slash 命令 × 测试矩阵的差集核对;有缺口即失败)
coverage-audit:
	python3 tests/tools/coverage_audit.py --strict

## release-check: 发布前全量审查(编译面 + 能力覆盖层 + 覆盖自审)
release-check:
	./tests/run_all.sh
	python3 tests/tools/coverage_audit.py --strict

## check: Compile check 全 workspace(类型检查,不链接)
check:
	cargo check --workspace

## check-fast: 快速类型检查单个 crate(示例:make check-fast C=zhuye-cli)
check-fast:
	cargo check -p $(C)

## watch: 文件变更自动 check(需先 cargo install cargo-watch)
watch:
	cargo watch -x check

## clean: Remove build artifacts
clean:
	cargo clean

## gc: 清掉 debug 里 cargo 不会自动 GC 的旧中间产物
gc:
	rm -rf target/debug/incremental target/debug/.fingerprint
	rm -rf target/debug/examples
	@echo "✅ removed target/debug/{incremental,.fingerprint,examples}"

## help: Show this message
help:
	@echo "Available targets:"
	@echo "  build         - Compile TUI in release mode"
	@echo "  run           - Build and launch TUI"
	@echo "  install       - Build + install to PATH"
	@echo "  dist          - 构建并打包当前平台分发包(tar.gz/zip)到 dist/"
	@echo "  dist-dry      - 预览 dist 打包命令(不实际执行)"
	@echo "  release       - 完整 release 构建(LTO,小体积)"
	@echo "  test          - cargo test (全量兼容)"
	@echo "  test-fast     - nextest 加速跑全部测试"
	@echo "  test-suite    - 全功能测试套件(能力覆盖层 + Rust 侧 + 覆盖自审)"
	@echo "  test-suite-fast - 只跑能力覆盖层(跳过 cargo 编译面)"
	@echo "  test-suite-live - live 端到端(需真实 LLM + API key)"
	@echo "  coverage-audit - 覆盖自审(slash 命令 × 测试矩阵的差集核对)"
	@echo "  release-check  - 发布前全量审查(编译面 + 能力覆盖层 + 覆盖自审)"
	@echo "  check         - cargo check 全 workspace"
	@echo "  check-fast    - cargo check 单 crate:make check-fast C=<crate>"
	@echo "  watch         - 文件变更自动 check"
	@echo "  clean         - Remove all build artifacts"
	@echo "  gc            - 清 incremental/.fingerprint/examples"
