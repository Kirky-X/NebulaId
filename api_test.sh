#!/bin/bash

echo "========================================"
echo "   Nebula ID 系统 - 全面API接口测试"
echo "========================================"
echo ""

BASE_URL="http://localhost:8080"

PASS=0
FAIL=0

test_endpoint() {
    local name=$1
    local method=$2
    local endpoint=$3
    local data=$4
    local expected_status=$5
    
    echo -n "测试 [$name]... "
    
    if [ "$method" == "GET" ]; then
        response=$(curl -s -w "\n%{http_code}" "$BASE_URL$endpoint")
    else
        response=$(curl -s -w "\n%{http_code}" -X "$method" \
            -H "Content-Type: application/json" \
            -d "$data" \
            "$BASE_URL$endpoint")
    fi
    
    status_code=$(echo "$response" | tail -1)
    body=$(echo "$response" | head -n -1)
    
    if [ "$status_code" == "$expected_status" ]; then
        echo "✓ PASS"
        ((PASS++))
    else
        echo "✗ FAIL (期望: $expected_status, 实际: $status_code)"
        echo "  响应: $body"
        ((FAIL++))
    fi
}

echo "1. 健康检查测试"
echo "----------------------------------------"
test_endpoint "健康检查" "GET" "/health" "" "200"
test_endpoint "API信息" "GET" "/api/v1" "" "200"

echo ""
echo "2. ID生成测试"
echo "----------------------------------------"
test_endpoint "默认算法生成" "POST" "/api/v1/generate" \
    '{"workspace": "test", "group": "default", "biz_tag": "test"}' "200"
test_endpoint "指定snowflake算法" "POST" "/api/v1/generate" \
    '{"workspace": "test", "group": "default", "biz_tag": "test", "algorithm": "snowflake"}' "200"
test_endpoint "指定segment算法" "POST" "/api/v1/generate" \
    '{"workspace": "test", "group": "default", "biz_tag": "test", "algorithm": "segment"}' "200"
test_endpoint "批量生成" "POST" "/api/v1/generate/batch" \
    '{"workspace": "test", "group": "default", "biz_tag": "test", "size": 5}' "200"

echo ""
echo "3. ID解析测试"
echo "----------------------------------------"
SNOWFLAKE_ID=$(curl -s -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace": "test", "group": "default", "biz_tag": "test", "algorithm": "snowflake"}' | jq -r '.id')

echo "生成的雪花ID: $SNOWFLAKE_ID"
test_endpoint "雪花ID解析" "POST" "/api/v1/parse" \
    "{\"id\": \"$SNOWFLAKE_ID\", \"workspace\": \"test\", \"group\": \"default\", \"biz_tag\": \"test\", \"algorithm\": \"snowflake\"}" "200"

echo ""
echo "4. 配置管理测试"
echo "----------------------------------------"
test_endpoint "获取配置" "GET" "/api/v1/config" "" "200"
test_endpoint "获取指标" "GET" "/metrics" "" "200"
test_endpoint "重载配置" "POST" "/api/v1/config/reload" '{}' "200"

echo ""
echo "========================================"
echo "   测试结果汇总"
echo "========================================"
echo "通过: $PASS"
echo "失败: $FAIL"
echo ""

if [ $FAIL -eq 0 ]; then
    echo "✓ 所有测试通过!"
    exit 0
else
    echo "✗ 有 $FAIL 个测试失败"
    exit 1
fi
