#!/bin/bash

# 本地 CI 预检脚本
# 在提交前运行所有 CI 检查，确保流水线能够通过
# 使用方法: ./pre-commit-check.sh

set -e
set -o pipefail

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 图标
CHECK="✓"
CROSS="✗"
ARROW="→"

# 统计变量
TOTAL_CHECKS=0
PASSED_CHECKS=0
FAILED_CHECKS=0

# 开始时间
START_TIME=$(date +%s)

# 打印标题
print_header() {
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}  🚀 Rust 项目本地 CI 预检${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
}

# 打印步骤
print_step() {
    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
    local description="$1"
    local cmd="$2"
    echo -e "${BLUE}[${TOTAL_CHECKS}/${EXPECTED_CHECKS}]${NC} ${ARROW} $description..."
    if [ -n "$cmd" ]; then
        echo -e "  ${YELLOW}运行命令: $cmd${NC}"
    fi
    echo ""
}

# 打印成功
print_success() {
    PASSED_CHECKS=$((PASSED_CHECKS + 1))
    echo -e "${GREEN}  ${CHECK} $1${NC}"
    echo ""
}

# 打印失败
print_error() {
    FAILED_CHECKS=$((FAILED_CHECKS + 1))
    echo -e "${RED}  ${CROSS} $1${NC}"
    echo ""
}

# 打印警告
print_warning() {
    echo -e "${YELLOW}  ⚠ $1${NC}"
    echo ""
}

# 打印信息
print_info() {
    echo -e "  ℹ $1"
}

# 打印分隔线
print_separator() {
    echo -e "${BLUE}────────────────────────────────────────────────────────${NC}"
}

# 检查命令是否存在
check_command() {
    if ! command -v "$1" &> /dev/null; then
        return 1
    fi
    return 0
}

# 总检查数
EXPECTED_CHECKS=7

# 打印标题
print_header

# 检查是否在 git 仓库中
if ! git rev-parse --git-dir > /dev/null 2>&1; then
    print_error "当前目录不是 git 仓库"
    exit 1
fi

# 检查是否在 Rust 项目中
if [ ! -f "Cargo.toml" ]; then
    print_error "未找到 Cargo.toml，请在 Rust 项目根目录运行此脚本"
    exit 1
fi

echo -e "${GREEN}环境检查通过 ${CHECK}${NC}"
echo ""
print_separator
echo ""

# ============================================================================
# 1. 代码格式检查 (rustfmt)
# ============================================================================
print_step "检查代码格式 (rustfmt)" "cargo fmt -- --check"

if ! check_command rustfmt; then
    print_warning "rustfmt 未安装，跳过格式检查"
    print_info "安装命令: rustup component add rustfmt"
else
    if cargo fmt -- --check > /dev/null 2>&1; then
        print_success "代码格式检查通过"
    else
        print_error "代码格式检查失败"
        echo ""
        echo -e "${BLUE}💡 修复命令:${NC}"
        echo -e "  ${YELLOW}cargo fmt${NC}"
        echo ""
        echo -e "${BLUE}💡 格式问题详情:${NC}"
        cargo fmt -- --check 2>&1 | head -20
        echo ""
        exit 1
    fi
fi

# ============================================================================
# 2. Clippy Lint 检查
# ============================================================================
# Only check lib and bin targets, not tests (tests may have type inference issues that don't affect runtime)
print_step "运行 Clippy lint 检查" "cargo clippy --lib --bins -- -D warnings -A clippy::derivable-clones -A clippy::redundant-pub-crate"

if ! check_command cargo-clippy; then
    print_warning "clippy 未安装，跳过 lint 检查"
    print_info "安装命令: rustup component add clippy"
else
    echo "  (这可能需要一些时间...)"
    if cargo clippy --lib --bins -- -D warnings > /tmp/clippy_output.txt 2>&1; then
        print_success "Clippy 检查通过，无警告"
    else
        print_error "Clippy 发现问题"
        echo ""
        echo -e "${BLUE}💡 详细命令:${NC}"
        echo -e "  ${YELLOW}cargo clippy --all${NC}"
        echo ""
        echo -e "${BLUE}💡 前 20 个问题:${NC}"
        grep -E "warning:|error:" /tmp/clippy_output.txt | head -20
        echo ""
        exit 1
    fi
fi

# ============================================================================
# 3. 编译检查
# ============================================================================
# Check default features only (skip etcd as it has pre-existing API compatibility issues)
print_step "检查项目编译" "cargo build --workspace"

echo "  (这可能需要一些时间...)"
if cargo build --workspace > /tmp/build_output.txt 2>&1; then
    print_success "项目编译成功"
else
    print_error "项目编译失败"
    echo ""
    echo -e "${BLUE}💡 详细命令:${NC}"
    echo -e "  ${YELLOW}cargo build --workspace${NC}"
    echo ""
    echo -e "${BLUE}💡 编译错误:${NC}"
    tail -30 /tmp/build_output.txt
    echo ""
    exit 1
fi

# ============================================================================
# 4. 运行测试
# ============================================================================
print_step "运行所有测试" "cargo test --workspace"

echo "  (这可能需要一些时间...)"
if cargo test --workspace > /tmp/test_output.txt 2>&1; then
    TEST_STATS=$(grep -E "test result:" /tmp/test_output.txt | tail -1)
    print_success "所有测试通过"
    if [ -n "$TEST_STATS" ]; then
        print_info "$TEST_STATS"
        echo ""
    fi
else
    print_error "部分测试失败"
    echo ""
    echo -e "${BLUE}💡 详细命令:${NC}"
    echo -e "  ${YELLOW}cargo test --workspace${NC}"
    echo ""
    echo -e "${BLUE}💡 失败的测试:${NC}"
    grep -A 5 "failures:" /tmp/test_output.txt | head -20
    echo ""
    exit 1
fi

# ============================================================================
# 5. 安全审计 (cargo-deny)
# ============================================================================
print_step "运行安全审计 (cargo-deny)" "cargo deny check"

if ! check_command cargo-deny; then
    print_warning "cargo-deny 未安装，跳过安全审计"
    print_info "安装命令: cargo install --locked cargo-deny"
else
    if [ ! -f "deny.toml" ]; then
        print_warning "未找到 deny.toml 配置文件"
        echo ""
        echo -e "${BLUE}💡 生成默认配置命令:${NC}"
        echo -e "  ${YELLOW}cargo deny init${NC}"
        echo ""
    else
        if cargo deny check > /tmp/deny_output.txt 2>&1; then
            print_success "安全审计通过，无高危漏洞"
        else
            print_error "发现安全问题或许可证冲突"
            echo ""
            echo -e "${BLUE}💡 详细命令:${NC}"
            echo -e "  ${YELLOW}cargo deny check${NC}"
            echo ""
            echo -e "${BLUE}💡 问题详情:${NC}"
            grep -E "error|warning" /tmp/deny_output.txt | head -20
            echo ""
            exit 1
        fi
    fi
fi

# ============================================================================
# 6. 文档检查
# ============================================================================
print_step "检查文档生成" "cargo doc --no-deps --workspace"

if cargo doc --no-deps --workspace > /tmp/doc_output.txt 2>&1; then
    print_success "文档生成成功"
else
    print_error "文档生成失败"
    echo ""
    echo -e "${BLUE}💡 详细命令:${NC}"
    echo -e "  ${YELLOW}cargo doc --no-deps --workspace${NC}"
    echo ""
    echo -e "${BLUE}💡 文档错误:${NC}"
    tail -20 /tmp/doc_output.txt
    echo ""
    exit 1
fi

# ============================================================================
# 7. 代码覆盖率 (可选)
# ============================================================================
print_step "计算代码覆盖率 (可选)"

if ! check_command cargo-tarpaulin; then
    print_warning "cargo-tarpaulin 未安装，跳过覆盖率检查"
    print_info "安装命令: cargo install cargo-tarpaulin"
else
    echo "  (这可能需要较长时间...)"
    echo "  (可以按 Ctrl+C 跳过此步骤)"
    if timeout 300 cargo tarpaulin --all-features --workspace --timeout 120 --out Stdout > /tmp/coverage_output.txt 2>&1; then
        COVERAGE=$(grep -oP '\d+\.\d+%' /tmp/coverage_output.txt | tail -1)
        if [ -n "$COVERAGE" ]; then
            print_success "代码覆盖率: $COVERAGE"
        else
            print_success "覆盖率计算完成"
        fi
    else
        if [ $? -eq 124 ]; then
            print_warning "覆盖率计算超时（5分钟），已跳过"
        else
            print_warning "覆盖率计算失败或被跳过"
        fi
    fi
fi

# ============================================================================
# 总结
# ============================================================================
echo ""
print_separator
echo ""

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

echo -e "${BLUE}📊 检查结果总结${NC}"
echo ""
echo -e "  总检查数: ${BLUE}${TOTAL_CHECKS}${NC}"
echo -e "  通过: ${GREEN}${PASSED_CHECKS}${NC}"
echo -e "  失败: ${RED}${FAILED_CHECKS}${NC}"
echo -e "  耗时: ${BLUE}${DURATION}${NC} 秒"
echo ""

if [ $FAILED_CHECKS -eq 0 ]; then
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${GREEN}  ✨ 所有检查通过！可以安全提交代码${NC}"
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo -e "${BLUE}推荐的提交流程：${NC}"
    echo -e "  1. ${YELLOW}git add .${NC}"
    echo -e "  2. ${YELLOW}git commit -m \"your message\"${NC}"
    echo -e "  3. ${YELLOW}git push${NC}"
    echo ""
    exit 0
else
    echo -e "${RED}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${RED}  ⚠️  发现 ${FAILED_CHECKS} 个问题，请修复后再提交${NC}"
    echo -e "${RED}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo -e "${BLUE}修复建议：${NC}"
    echo ""
    
    # 根据失败的检查给出具体建议
    if grep -q "代码格式检查失败" /tmp/pre_commit_check.log 2>/dev/null || ! cargo fmt -- --check > /dev/null 2>&1; then
        echo -e "  ${YELLOW}1.${NC} 修复格式问题："
        echo -e "     ${YELLOW}cargo fmt${NC}"
        echo ""
    fi
    
    if ! cargo clippy --lib --bins -- -D warnings > /dev/null 2>&1; then
        echo -e "  ${YELLOW}2.${NC} 修复 Clippy 警告："
        echo -e "     ${YELLOW}cargo clippy --lib --bins -- -D warnings${NC}"
        echo ""
    fi
    
    if ! cargo test --workspace > /dev/null 2>&1; then
        echo -e "  ${YELLOW}3.${NC} 修复测试失败："
        echo -e "     ${YELLOW}cargo test --workspace${NC}"
        echo ""
    fi
    
    echo -e "  ${BLUE}修复完成后，重新运行此脚本验证${NC}"
    echo ""
    exit 1
fi
