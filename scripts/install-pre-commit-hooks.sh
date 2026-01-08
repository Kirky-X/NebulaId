#!/bin/bash
#
# Pre-commit Hooks Installation Script for Nebula ID
#
# This script installs pre-commit hooks for the Nebula ID project.
# When installed, it will automatically run code quality checks
# before each git commit.
#
# Usage:
#   ./scripts/install-pre-commit-hooks.sh
#
# Requirements:
#   - Python 3.8+ with pip
#   - Rust toolchain (cargo, rustfmt, clippy)
#

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}  🚀 Nebula ID Pre-commit Hooks 安装${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# Check if pre-commit is installed
if ! command -v pre-commit &> /dev/null; then
    echo -e "${YELLOW}⚠ pre-commit 未安装，正在安装...${NC}"
    echo ""

    # Check Python version
    if command -v python3 &> /dev/null; then
        echo "使用 pip3 安装 pre-commit..."
        pip3 install --user pre-commit

        # Add ~/.local/bin to PATH if needed
        export PATH="$HOME/.local/bin:$PATH"
    elif command -v pip &> /dev/null; then
        echo "使用 pip 安装 pre-commit..."
        pip install --user pre-commit
    else
        echo -e "${RED}❌ 错误: 未找到 Python 或 pip${NC}"
        echo "请先安装 Python 3.8+: https://www.python.org/downloads/"
        exit 1
    fi

    # Verify installation
    if ! command -v pre-commit &> /dev/null; then
        echo -e "${RED}❌ pre-commit 安装失败${NC}"
        echo "请手动安装: pip install --user pre-commit"
        exit 1
    fi

    echo -e "${GREEN}✓ pre-commit 安装成功${NC}"
    echo ""
else
    echo -e "${GREEN}✓ pre-commit 已安装${NC}"
    echo ""
fi

# Check Rust toolchain
echo -e "${BLUE}[1/3]${NC} 检查 Rust 工具链..."
echo ""

if ! command -v cargo &> /dev/null; then
    echo -e "${RED}❌ 错误: 未找到 cargo${NC}"
    echo "请先安装 Rust: https://rustup.rs/"
    exit 1
fi

if ! command -v rustfmt &> /dev/null; then
    echo -e "${YELLOW}⚠ 未找到 rustfmt，正在安装...${NC}"
    rustup component add rustfmt
fi

if ! command -v clippy &> /dev/null; then
    echo -e "${YELLOW}⚠ 未找到 clippy，正在安装...${NC}"
    rustup component add clippy
fi

echo -e "${GREEN}✓ Rust 工具链检查完成${NC}"
echo ""

# Install pre-commit hooks
echo -e "${BLUE}[2/3]${NC} 安装 pre-commit 钩子..."
echo ""

PRE_COMMIT_FILE=".pre-commit-config.yaml"
if [ ! -f "$PRE_COMMIT_FILE" ]; then
    echo -e "${RED}❌ 错误: 未找到 $PRE_COMMIT_FILE${NC}"
    echo "请确保 .pre-commit-config.yaml 存在于项目根目录"
    exit 1
fi

pre-commit install

echo -e "${GREEN}✓ Pre-commit 钩子安装成功${NC}"
echo ""

# Show installed hooks
echo -e "${BLUE}[3/3]${NC} 已安装的钩子:"
echo ""
pre-commit hooks --hook-type pre-commit
echo ""

# Verify hooks are properly configured
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}  ✅ 安装完成！${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo -e "现在，每次执行 ${YELLOW}git commit${NC} 时，系统将自动运行以下检查:"
echo ""
echo -e "  ${GREEN}1.${NC} ${BLUE}cargo fmt${NC}      - 检查代码格式"
echo -e "  ${GREEN}2.${NC} ${BLUE}cargo clippy${NC}   - 静态代码分析"
echo -e "  ${GREEN}3.${NC} ${BLUE}cargo check${NC}    - 编译检查"
echo ""
echo -e "如果任何检查失败，提交将被阻止并显示错误信息。"
echo ""
echo -e "${YELLOW}提示:${NC} 如需手动运行所有检查，可使用:"
echo -e "  ${YELLOW}./scripts/pre-commit-check.sh${NC}"
echo ""

# Run initial check to verify everything works
echo -e "${YELLOW}运行初始检查以验证配置...${NC}"
echo ""

if pre-commit run --all-files; then
    echo ""
    echo -e "${GREEN}✓ 所有检查通过！${NC}"
else
    echo ""
    echo -e "${YELLOW}⚠ 某些检查失败，但这不影响钩子安装${NC}"
    echo "请修复问题后重新尝试提交"
fi
