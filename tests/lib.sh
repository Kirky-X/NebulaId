#!/bin/bash
# 测试通用库 - Nebula ID 测试套件

AUTH_HEADER="Authorization: Basic dGVzdC1rZXktaWQ6dGVzdC1zZWNyZXQ="
API_BASE="http://localhost:8080"

check_prerequisites() {
    local missing=0
    for cmd in curl jq; do
        if ! command -v $cmd &> /dev/null; then
            echo "❌ 缺少必要工具: $cmd"
            missing=1
        fi
    done
    if [ $missing -eq 1 ]; then
        exit 1
    fi
}

check_api_health() {
    echo -e "\n【健康检查】"
    local health=$(curl -s "${API_BASE}/health")
    local status=$(echo "$health" | jq -r '.status')
    echo "系统状态: $status"
    
    if [ "$status" != "healthy" ]; then
        echo "⚠️  服务未就绪，请确保 nebula-id 服务正在运行"
        return 1
    fi
    return 0
}

switch_algorithm() {
    local biz_tag="$1"
    local algorithm="$2"
    local result=$(curl -s -X POST "${API_BASE}/api/v1/config/algorithm" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"biz_tag\": \"${biz_tag}\", \"algorithm\": \"${algorithm}\"}")
    echo "$result"
}

generate_id() {
    local workspace="$1"
    local group="$2"
    local biz_tag="$3"
    curl -s -X POST "${API_BASE}/api/v1/generate" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\"}"
}

generate_batch() {
    local workspace="$1"
    local group="$2"
    local biz_tag="$3"
    local size="$4"
    curl -s -X POST "${API_BASE}/api/v1/generate/batch" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\", \"size\": ${size}}"
}

get_id_from_response() {
    echo "$1" | jq -r '.id'
}

get_algorithm_from_response() {
    echo "$1" | jq -r '.algorithm'
}

print_header() {
    echo ""
    echo "=========================================="
    echo "$1"
    echo "=========================================="
}

print_section() {
    echo ""
    echo "【$1】"
    echo "----------------------------------------"
}

verify_id_not_null() {
    local id="$1"
    if [ "$id" == "null" ] || [ -z "$id" ]; then
        echo "❌ ID 为空，测试失败"
        return 1
    fi
    return 0
}

verify_uuid_format() {
    local id="$1"
    if echo "$id" | grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'; then
        echo "✓ UUID 格式正确"
        return 0
    else
        echo "❌ UUID 格式错误: $id"
        return 1
    fi
}

verify_numeric_format() {
    local id="$1"
    if [[ "$id" =~ ^[0-9]+$ ]]; then
        echo "✓ 数值格式正确: $id"
        return 0
    else
        echo "❌ 数值格式错误: $id"
        return 1
    fi
}
