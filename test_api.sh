#!/bin/bash

# Nebula ID API 测试脚本
BASE_URL="http://localhost:8080"

echo "=========================================="
echo "Nebula ID API 测试"
echo "=========================================="
echo ""

# 颜色定义
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# 测试计数
TOTAL=0
PASSED=0
FAILED=0

# 测试函数
test_api() {
    local method=$1
    local endpoint=$2
    local description=$3
    local data=$4
    local auth_header=$5
    
    TOTAL=$((TOTAL + 1))
    echo -e "${YELLOW}测试 #${TOTAL}: ${description}${NC}"
    echo "  ${method} ${endpoint}"
    
    if [ -n "$auth_header" ]; then
        if [ -n "$data" ]; then
            response=$(curl -s -w "\n%{http_code}" -X ${method} "${BASE_URL}${endpoint}" \
                -H "Content-Type: application/json" \
                -H "${auth_header}" \
                -d "${data}")
        else
            response=$(curl -s -w "\n%{http_code}" -X ${method} "${BASE_URL}${endpoint}" \
                -H "${auth_header}")
        fi
    else
        if [ -n "$data" ]; then
            response=$(curl -s -w "\n%{http_code}" -X ${method} "${BASE_URL}${endpoint}" \
                -H "Content-Type: application/json" \
                -d "${data}")
        else
            response=$(curl -s -w "\n%{http_code}" -X ${method} "${BASE_URL}${endpoint}")
        fi
    fi
    
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | sed '$d')
    
    if [ "$http_code" -ge 200 ] && [ "$http_code" -lt 300 ]; then
        echo -e "  ${GREEN}✓ 成功 (HTTP ${http_code})${NC}"
        PASSED=$((PASSED + 1))
    else
        echo -e "  ${RED}✗ 失败 (HTTP ${http_code})${NC}"
        FAILED=$((FAILED + 1))
    fi
    
    echo "  响应: ${body}" | head -c 200
    echo ""
    echo ""
}

echo "=========================================="
echo "1. 公共接口（无需认证）"
echo "=========================================="
echo ""

test_api "GET" "/health" "健康检查"
test_api "GET" "/ready" "就绪检查"
test_api "GET" "/metrics" "Prometheus 指标"
test_api "GET" "/api-docs/openapi.json" "OpenAPI 文档"

echo "=========================================="
echo "2. V1 API 接口（无需认证，因为 auth.enabled=false）"
echo "=========================================="
echo ""

test_api "GET" "/" "API 信息"

echo "=========================================="
echo "3. ID 生成接口"
echo "=========================================="
echo ""

test_api "POST" "/api/v1/generate" "生成单个 ID" '{"workspace":"default","group":"test"}'
test_api "POST" "/api/v1/generate/batch" "批量生成 ID" '{"workspace":"default","group":"test","size":5}'
test_api "POST" "/api/v1/parse" "解析 ID" '{"id":"01HX5X5X5X5X5X5X5X5X5X5X5X"}'

echo "=========================================="
echo "4. 配置管理接口"
echo "=========================================="
echo ""

test_api "GET" "/api/v1/config" "获取配置"
test_api "POST" "/api/v1/config/algorithm" "设置算法" '{"algorithm":"snowflake"}'

echo "=========================================="
echo "5. 业务标签接口"
echo "=========================================="
echo ""

test_api "POST" "/api/v1/biz-tags" "创建业务标签" '{"name":"test-tag","description":"测试标签","workspace_id":"00000000-0000-0000-0000-000000000001"}'
test_api "GET" "/api/v1/biz-tags" "列出业务标签"
test_api "GET" "/api/v1/biz-tags?page=1&page_size=10" "分页列出业务标签"

echo "=========================================="
echo "6. 工作区接口"
echo "=========================================="
echo ""

test_api "GET" "/api/v1/workspaces" "列出工作区"

echo "=========================================="
echo "7. 组接口"
echo "=========================================="
echo ""

test_api "GET" "/api/v1/groups?workspace=default" "列出组"

echo "=========================================="
echo "测试总结"
echo "=========================================="
echo ""
echo "总计: ${TOTAL}"
echo -e "${GREEN}通过: ${PASSED}${NC}"
echo -e "${RED}失败: ${FAILED}${NC}"
echo ""

if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}所有测试通过！${NC}"
    exit 0
else
    echo -e "${RED}有 ${FAILED} 个测试失败${NC}"
    exit 1
fi
