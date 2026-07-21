.PHONY: dev-up dev-down dev-logs test build clean lint clippy doc docker-build

# ========== Configuration ==========
BUILD_JOBS ?= $(shell nproc)
RUST_TARGET ?= x86_64-unknown-linux-gnu

# 增强选项
SCCACHE_ENABLE ?= 0
CARGO_NET_GIT_FETCH_WITH_CLI ?= false

# ========== Development Commands ==========
dev-up:
	docker-compose up -d

dev-down:
	docker-compose down

dev-logs:
	docker-compose logs -f

# ========== Build Commands ==========
build: ## Build release binary with optimizations
	@echo "Building release binary (jobs: $(BUILD_JOBS))..."
	@if [ "$(SCCACHE_ENABLE)" = "1" ]; then export RUSTC_WRAPPER=sccache; fi
	cargo build --release --locked --jobs $(BUILD_JOBS)

build-dev: ## Build development binary
	@echo "Building development binary..."
	cargo build --jobs $(BUILD_JOBS)

build-bin: ## Build specific binary
	@if [ -z "$(BIN)" ]; then echo "Usage: make build-bin BIN=nebula-id"; exit 1; fi
	cargo build --$(or $(PROFILE),release) --locked -p nebula-server --bin $(BIN)

build-tui: ## Build TUI binary
	cargo build --release --locked -p nebula-server --bin nebula-id-tui

# ========== Docker Build ==========
docker-build: ## Build Docker image
	@echo "Building Docker image..."
	docker build -t nebula-id:$(shell git rev-parse --short HEAD) -f docker/Dockerfile .
	@echo "Image built successfully"

docker-build-multi: ## Build multi-architecture Docker image
	@echo "Building multi-architecture image..."
	docker buildx build -t nebula-id:latest \
		--platform linux/amd64,linux/arm64 \
		-f docker/Dockerfile \
		--push .

# ========== Test Commands ==========
test: ## Run all tests with parallel execution
	@echo "Running tests (threads: 4)..."
	cargo test --all --jobs $(BUILD_JOBS) -- --test-threads=4

test-unit: ## Run unit tests only
	cargo test --lib --bins --jobs $(BUILD_JOBS) -- --test-threads=4

test-integration: ## Run integration tests only
	cargo test --test integration_tests -- --test-threads=4

test-quick: ## Quick test (skip slow tests)
	cargo test --all -- --test-threads=4 --skip bench --skip perf

test-coverage:
	cargo tarpaulin --out Html --jobs $(BUILD_JOBS)

bench: ## Run benchmarks
	cargo bench --all -- --test-threads=1

# ========== Code Quality ==========
lint: ## Check code formatting
	@echo "Checking code formatting..."
	cargo fmt --all -- --check

lint-fix: ## Fix code formatting
	cargo fmt --all

clippy: ## Run clippy lints
	@echo "Running clippy..."
	cargo clippy --all -- \
		-D warnings \
		-A clippy::unnecessary-semicolon

clippy-fix: ## Auto-fix clippy warnings
	cargo clippy --all --fix

# ========== Documentation ==========
doc: ## Generate documentation
	@echo "Generating documentation..."
	cargo doc --no-deps --no-deps --document-private-items

doc-open: doc ## Generate and open documentation
	cargo doc --no-deps --open --document-private-items

# ========== Dependencies ==========
deps-update: ## Update dependencies
	@echo "Updating dependencies..."
	cargo update
	cargo update -p sea-orm --aggressive

deps-check: ## Check for outdated dependencies
	@echo "Checking for outdated dependencies..."
	cargo outdated -R

deps-tree: ## Show dependency tree
	cargo tree --prefix=none -i

# ========== Clean Commands ==========
clean: ## Clean build artifacts
	@echo "Cleaning build artifacts..."
	cargo clean
	docker-compose down -v

clean-cache: ## Clean cargo cache
	@echo "Cleaning cargo cache..."
	cargo clean -p nebula-core
	cargo clean -p nebula-server
	rm -rf target/debug/deps target/release/deps

# ========== Analysis ==========
analyze: ## Analyze build time
	@echo "Analyzing build..."
	cargo build --release --timings=html
	@echo "Report generated at target/release/build-timing/index.html"

size-check: ## Check binary size
	@echo "Binary size:"
	@ls -lh target/release/nebula-id 2>/dev/null || echo "Release binary not found. Run 'make build' first."

# ========== Database ==========
db-migrate:
	cargo run --bin migrate -- crates/server

db-shell:
	docker-compose exec postgres psql -U idgen -d idgen

# ========== One-liners ==========
shell:
	docker-compose exec app bash

redis-cli:
	docker-compose exec redis redis-cli

# ========== Help ==========
help: ## Show this help message
	@echo "Nebula ID Build System"
	@echo ""
	@echo "Usage: make <command> [options]"
	@echo ""
	@echo "Build Commands:"
	@echo "  build          Build release binary"
	@echo "  build-dev      Build development binary"
	@echo "  build-bin      Build specific binary (BIN=name)"
	@echo "  build-tui      Build TUI binary"
	@echo "  docker-build   Build Docker image"
	@echo "  docker-build-multi  Build multi-arch Docker image"
	@echo ""
	@echo "Test Commands:"
	@echo "  test           Run all tests"
	@echo "  test-unit      Run unit tests only"
	@echo "  test-integration  Run integration tests"
	@echo "  test-quick     Quick test (skip slow tests)"
	@echo "  bench          Run benchmarks"
	@echo ""
	@echo "Code Quality:"
	@echo "  lint           Check code formatting"
	@echo "  lint-fix       Fix code formatting"
	@echo "  clippy         Run clippy lints"
	@echo "  clippy-fix     Auto-fix clippy warnings"
	@echo ""
	@echo "Documentation:"
	@echo "  doc            Generate documentation"
	@echo "  doc-open       Generate and open docs"
	@echo ""
	@echo "Maintenance:"
	@echo "  deps-update    Update dependencies"
	@echo "  deps-check     Check outdated dependencies"
	@echo "  deps-tree      Show dependency tree"
	@echo "  clean          Clean build artifacts"
	@echo "  clean-cache    Clean cargo cache"
	@echo "  analyze        Analyze build time"
	@echo "  size-check     Check binary size"
	@echo ""
	@echo "Options:"
	@echo "  BUILD_JOBS=N   Number of parallel build jobs (default: all cores)"
	@echo "  SCCACHE_ENABLE=1  Enable sccache for faster rebuilds"
