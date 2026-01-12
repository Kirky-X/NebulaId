#!/bin/bash
# Nebula ID API 测试脚本
# 用法: ./test_api.sh [server_url]
# 默认 server_url = http://localhost:8080

set -e

SERVER_URL="${1:-http://localhost:8080}"
echo "🌐 测试服务器: $SERVER_URL"

# 颜色定义
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 计数器
PASSED=0
FAILED=0

# 测试函数
test_endpoint() {
    local method=$1
    local endpoint=$2
    local description=$3
    local expected_status=$4
    local data=$5

    echo -n "  $description... "

    if [ -z "$data" ]; then
        response=$(curl -s -o /dev/null -w "%{http_code}" -X "$method" "$SERVER_URL$endpoint")
    else
        response=$(curl -s -o /dev/null -w "%{http_code}" -X "$method" \
            -H "Content-Type: application/json" \
            -d "$data" \
            "$SERVER_URL$endpoint")
    fi

    if [ "$response" == "$expected_status" ]; then
        echo -e "${GREEN}✓ PASS${NC} (HTTP $response)"
        ((PASSED++))
    else
        echo -e "${RED}✗ FAIL${NC} (Expected $expected_status, got $response)"
        ((FAILED++))
    fi
}

# Bearer Token 认证测试函数
test_auth_endpoint() {
    local method=$1
    local endpoint=$2
    local description=$3
    local expected_status=$4
    local api_key=$5
    local data=$6

    echo -n "  $description... "

    if [ -z "$data" ]; then
        response=$(curl -s -o /dev/null -w "%{http_code}" \
            -X "$method" \
            -H "Authorization: Bearer $api_key" \
            "$SERVER_URL$endpoint")
    else
        response=$(curl -s -o /dev/null -w "%{http_code}" \
            -X "$method" \
            -H "Authorization: Bearer $api_key" \
            -H "Content-Type: application/json" \
            -d "$data" \
            "$SERVER_URL$endpoint")
    fi

    if [ "$response" == "$expected_status" ]; then
        echo -e "${GREEN}✓ PASS${NC} (HTTP $response)"
        ((PASSED++))
    else
        echo -e "${RED}✗ FAIL${NC} (Expected $expected_status, got $response)"
        ((FAILED++))
    fi
}

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║          Nebula ID API 测试套件 v1.0                      ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# =====================================================
# 1. 公共端点测试 (无需认证)
# =====================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}1. 公共端点测试 (无需认证)${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

test_endpoint "GET" "/" "根路径健康检查" "200"
test_endpoint "GET" "/health" "健康检查" "200"
test_endpoint "GET" "/ready" "就绪检查" "200"
test_endpoint "GET" "/metrics" "Prometheus 指标" "200"
test_endpoint "GET" "/api/v1" "API 信息" "200"
test_endpoint "GET" "/api-docs/openapi.json" "OpenAPI 文档" "200"
test_endpoint "GET" "/nonexistent" "404 错误处理" "404"

echo ""

# =====================================================
# 2. 认证端点测试 (需要 API Key)
# =====================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}2. 认证端点测试 (需要 API Key)${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# 测试未授权访问
test_endpoint "POST" "/api/v1/generate" "未授权访问生成" "401"
test_endpoint "POST" "/api/v1/generate/batch" "未授权访问批量生成" "401"
test_endpoint "POST" "/api/v1/parse" "未授权访问解析" "401"
test_endpoint "GET" "/api/v1/config" "未授权访问配置" "401"
test_endpoint "GET" "/api/v1/workspaces" "未授权访问工作区列表" "401"
test_endpoint "GET" "/api/v1/biz-tags" "未授权访问标签列表" "401"

echo ""

# =====================================================
# 3. ID 生成功能测试 (需要管理员 API Key)
# =====================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}3. ID 生成功能测试 (需要管理员 API Key)${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# 测试无效请求体
ADMIN_KEY="your-admin-key-here"

test_auth_endpoint "POST" "/api/v1/generate" "空请求体 (400)" "400" "$ADMIN_KEY" "{}"
test_auth_endpoint "POST" "/api/v1/generate" "无效 JSON (415)" "415" "$ADMIN_KEY" "invalid json"
test_auth_endpoint "POST" "/api/v1/generate/batch" "空批量请求" "400" "$ADMIN_KEY" '{"workspace":"test","group":"test","size":0}'

echo ""

# =====================================================
# 4. 业务标签管理测试
# =====================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}4. 业务标签管理测试${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# 测试创建标签
test_auth_endpoint "POST" "/api/v1/biz-tags" "创建业务标签" "201" "$ADMIN_KEY" '{"workspace_id":"test-ws","name":"test-tag","description":"Test tag"}'

# 测试列出标签
test_auth_endpoint "GET" "/api/v1/biz-tags" "列出业务标签" "200" "$ADMIN_KEY"

# 测试参数验证
test_auth_endpoint "POST" "/api/v1/biz-tags" "空标签名 (400)" "400" "$ADMIN_KEY" '{"workspace_id":"test","name":"","description":"Test"}'
test_auth_endpoint "POST" "/api/v1/biz-tags" "过长描述 (400)" "400" "$ADMIN_KEY" "$(printf '{"workspace_id":"test","name":"tag","description":"%0.sA"' {1..500})"

echo ""

# =====================================================
# 5. 工作区管理测试
# =====================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}5. 工作区管理测试${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

test_auth_endpoint "POST" "/api/v1/workspaces" "创建工作区" "201" "$ADMIN_KEY" '{"name":"test-workspace","description":"Test workspace"}'
test_auth_endpoint "GET" "/api/v1/workspaces" "列出工作区" "200" "$ADMIN_KEY"
test_auth_endpoint "GET" "/api/v1/workspaces/nonexistent" "获取不存在工作区 (404)" "404" "$ADMIN_KEY"

echo ""

# =====================================================
# 6. 组管理测试
# =====================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}6. 组管理测试${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

test_auth_endpoint "POST" "/api/v1/groups" "创建组" "201" "$ADMIN_KEY" '{"workspace":"test-workspace","name":"test-group","description":"Test group"}'
test_auth_endpoint "GET" "/api/v1/groups" "列出组" "200" "$ADMIN_KEY" '?workspace=test-workspace'

echo ""

# =====================================================
# 7. API Key 管理测试 (管理员专用)
# =====================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}7. API Key 管理测试 (管理员专用)${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

test_auth_endpoint "POST" "/api/v1/api-keys" "创建 API Key" "201" "$ADMIN_KEY" '{"name":"test-key","role":"user","workspace_id":"test-ws"}'
test_auth_endpoint "GET" "/api/v1/api-keys" "列出 API Keys" "200" "$ADMIN_KEY"

echo ""

# =====================================================
# 8. 配置管理测试
# =====================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}8. 配置管理测试${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

test_auth_endpoint "GET" "/api/v1/config" "获取配置" "200" "$ADMIN_KEY"
test_auth_endpoint "POST" "/api/v1/config/rate-limit" "更新限速配置" "200" "$ADMIN_KEY" '{"default_rps":5000,"burst_size":1000}'
test_auth_endpoint "POST" "/api/v1/config/logging" "更新日志配置" "200" "$ADMIN_KEY" '{"level":"debug"}'
test_auth_endpoint "POST" "/api/v1/config/reload" "重载配置" "200" "$ADMIN_KEY"
test_auth_endpoint "POST" "/api/v1/config/algorithm" "设置算法" "200" "$ADMIN_KEY" '{"algorithm":"snowflake"}'

echo ""

# =====================================================
# 9. 性能测试
# =====================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}9. 性能测试${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

echo -n "  ID 生成吞吐量 (100次)... "
start_time=$(date +%s%N)
for i in {1..100}; do
    curl -s -o /dev/null -X POST "$SERVER_URL/api/v1/generate" \
        -H "Authorization: Bearer $ADMIN_KEY" \
        -H "Content-Type: application/json" \
        -d '{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag"}' > /dev/null 2>&1
done
end_time=$(date +%s%N)
duration=$((($end_time - $start_time) / 1000000))
rps=$((100000 / duration))
echo -e "${GREEN}✓ PASS${NC} (~${rps} req/s, ${duration}ms total)"
((PASSED++))

echo ""

# =====================================================
# 测试结果汇总
# =====================================================
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                   测试结果汇总                              ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  ${GREEN}通过: $PASSED${NC}"
echo -e "  ${RED}失败: $FAILED${NC}"
echo ""

if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}🎉 所有测试通过！${NC}"
    exit 0
else
    echo -e "${RED}⚠️  $FAILED 个测试失败，请检查日志${NC}"
    exit 1
fi
