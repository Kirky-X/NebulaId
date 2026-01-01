#!/bin/bash

echo "🧪 测试批量大小参数校验"
echo "=========================="

BASE_URL=${1:-"http://localhost:8080"}

# 测试1: 批量大小为 0
echo "测试1: 批量大小为 0"
curl -X POST "${BASE_URL}/api/v1/generate/batch" \
  -H "Content-Type: application/json" \
  -H "Authorization: Basic test-key_test-secret" \
  -d '{
    "workspace": "test-workspace",
    "group": "test-group",
    "biz_tag": "test-tag",
    "size": 0
  }' 2>/dev/null | jq '.'
echo ""

# 测试2: 批量大小为 -1 (应该被拒绝)
echo "测试2: 批量大小为 -1"
curl -X POST "${BASE_URL}/api/v1/generate/batch" \
  -H "Content-Type: application/json" \
  -H "Authorization: Basic test-key_test-secret" \
  -d '{
    "workspace": "test-workspace",
    "group": "test-group",
    "biz_tag": "test-tag",
    "size": -1
  }' 2>/dev/null | jq '.'
echo ""

# 测试3: 批量大小为 1000000 (超过最大值)
echo "测试3: 批量大小为 1000000"
curl -X POST "${BASE_URL}/api/v1/generate/batch" \
  -H "Content-Type: application/json" \
  -H "Authorization: Basic test-key_test-secret" \
  -d '{
    "workspace": "test-workspace",
    "group": "test-group",
    "biz_tag": "test-tag",
    "size": 1000000
  }' 2>/dev/null | jq '.'
echo ""

# 测试4: 批量大小为 100 (边界值，应该被接受)
echo "测试4: 批量大小为 100 (边界值)"
curl -X POST "${BASE_URL}/api/v1/generate/batch" \
  -H "Content-Type: application/json" \
  -H "Authorization: Basic test-key_test-secret" \
  -d '{
    "workspace": "test-workspace",
    "group": "test-group",
    "biz_tag": "test-tag",
    "size": 100
  }' 2>/dev/null | jq '.'
echo ""

# 测试5: 批量大小为 10 (正常值)
echo "测试5: 批量大小为 10 (正常值)"
curl -X POST "${BASE_URL}/api/v1/generate/batch" \
  -H "Content-Type: application/json" \
  -H "Authorization: Basic test-key_test-secret" \
  -d '{
    "workspace": "test-workspace",
    "group": "test-group",
    "biz_tag": "test-tag",
    "size": 10
  }' 2>/dev/null | jq '.'
echo ""

echo "✅ 批量大小校验测试完成"