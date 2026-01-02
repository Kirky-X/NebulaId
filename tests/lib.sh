#!/bin/bash
# 测试通用库 - Nebula ID 测试套件

# 全局配置
AUTH_HEADER="Authorization: Basic dGVzdC1rZXktaWQ6dGVzdC1zZWNyZXQ="
API_BASE="http://localhost:8080"
REPORT_FILE=""

# 日志颜色
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# ========== 基础检查 ==========

check_prerequisites() {
    local missing=0
    for cmd in curl jq; do
        if ! command -v $cmd &> /dev/null; then
            echo -e "${RED}❌ 缺少必要工具: $cmd${NC}"
            missing=1
        fi
    done
    if [ $missing -eq 1 ]; then
        exit 1
    fi
}

check_api_health() {
    echo -e "\n${YELLOW}【健康检查】${NC}"
    local health=$(curl -s "${API_BASE}/health")
    local status=$(echo "$health" | jq -r '.status')
    echo "系统状态: $status"
    
    if [ "$status" != "healthy" ]; then
        echo -e "${YELLOW}⚠️  服务未就绪，请确保 nebula-id 服务正在运行${NC}"
        return 1
    fi
    echo -e "${GREEN}✅ 服务健康${NC}"
    return 0
}

# ========== API 调用 ==========

switch_algorithm() {
    local biz_tag="$1"
    local algorithm="$2"
    curl -s -X POST "${API_BASE}/api/v1/config/algorithm" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"biz_tag\": \"${biz_tag}\", \"algorithm\": \"${algorithm}\"}"
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

generate_id_with_algo() {
    local workspace="$1"
    local group="$2"
    local biz_tag="$3"
    local algorithm="$4"
    curl -s -X POST "${API_BASE}/api/v1/generate" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\", \"algorithm\": \"${algorithm}\"}"
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

parse_id() {
    local id="$1"
    local workspace="$2"
    local group="$3"
    local biz_tag="$4"
    local algorithm="$5"
    curl -s -X POST "${API_BASE}/api/v1/parse" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"id\": \"${id}\", \"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\", \"algorithm\": \"${algorithm}\"}"
}

# ========== 响应解析 ==========

get_id_from_response() {
    echo "$1" | jq -r '.id'
}

get_algorithm_from_response() {
    echo "$1" | jq -r '.algorithm'
}

get_ids_from_batch_response() {
    echo "$1" | jq -r '.ids[]'
}

get_id_count_from_batch() {
    echo "$1" | jq -r '.ids | length'
}

# ========== 雪花ID解析 ==========

parse_snowflake_id() {
    local id="$1"
    local timestamp=$((id >> 22))
    local datacenter=$(((id >> 17) & 31))
    local worker=$(((id >> 12) & 31))
    local sequence=$((id & 4095))
    
    echo "timestamp:${timestamp},datacenter:${datacenter},worker:${worker},sequence:${sequence}"
}

# ========== 格式验证 ==========

verify_id_not_null() {
    local id="$1"
    if [ "$id" == "null" ] || [ -z "$id" ]; then
        return 1
    fi
    return 0
}

verify_numeric_format() {
    local id="$1"
    if [[ "$id" =~ ^[0-9]+$ ]]; then
        return 0
    fi
    return 1
}

verify_uuid_format() {
    local id="$1"
    if echo "$id" | grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'; then
        return 0
    fi
    return 1
}

verify_uuid_v7_format() {
    local id="$1"
    if [[ "$id" =~ ^\{?[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}\}?$ ]]; then
        return 0
    fi
    return 1
}

# ========== 输出格式化 ==========

print_header() {
    echo ""
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}========================================${NC}"
}

print_section() {
    echo ""
    echo -e "${YELLOW}【$1】${NC}"
    echo -e "${YELLOW}----------------------------------------${NC}"
}

print_pass() {
    echo -e "  ${GREEN}[PASS]${NC} $1"
}

print_fail() {
    echo -e "  ${RED}[FAIL]${NC} $1"
}

print_skip() {
    echo -e "  ${YELLOW}[SKIP]${NC} $1"
}

print_info() {
    echo -e "  ${YELLOW}[INFO]${NC} $1"
}

# ========== 报告生成 ==========

init_report() {
    REPORT_FILE="test_results_$(date +%Y%m%d_%H%M%S).txt"
    echo "Nebula ID API Test Report" > "$REPORT_FILE"
    echo "Generated: $(date)" >> "$REPORT_FILE"
    echo "Base URL: $API_BASE" >> "$REPORT_FILE"
    echo "========================================" >> "$REPORT_FILE"
}

log_result() {
    local test_name="$1"
    local status="$2"
    local details="$3"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [$status] $test_name: $details" >> "$REPORT_FILE"
}

# ========== 并发测试辅助 ==========

run_concurrent_test() {
    local name="$1"
    local worker_count="$2"
    local requests_per_worker="$3"
    local workspace="$4"
    local group="$5"
    local biz_tag="$6"
    local algorithm="$7"
    
    local tmpdir=$(mktemp -d)
    local pids=()
    local total_success=0
    
    for w in $(seq 1 $worker_count); do
        (
            local local_success=0
            for r in $(seq 1 $requests_per_worker); do
                local http_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "${API_BASE}/api/v1/generate" \
                    -H "Content-Type: application/json" \
                    -H "$AUTH_HEADER" \
                    -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\", \"algorithm\": \"${algorithm}\"}")
                if [ "$http_code" = "200" ]; then
                    local_success=$((local_success + 1))
                fi
            done
            echo "$local_success" > "${tmpdir}/worker_${w}.tmp"
        ) &
        pids+=($!)
    done
    
    for pid in "${pids[@]}"; do
        wait $pid 2>/dev/null || true
    done
    
    for w in $(seq 1 $worker_count); do
        if [ -f "${tmpdir}/worker_${w}.tmp" ]; then
            local count=$(cat "${tmpdir}/worker_${w}.tmp")
            total_success=$((total_success + count))
            rm -f "${tmpdir}/worker_${w}.tmp"
        fi
    done
    
    rm -rf "$tmpdir"
    echo "$total_success"
}

# ========== 性能测试辅助 ==========

measure_performance() {
    local name="$1"
    local request_count="$2"
    local workspace="$3"
    local group="$4"
    local biz_tag="$5"
    
    local start_time=$(date +%s%N)
    local success=0
    
    for i in $(seq 1 $request_count); do
        local http_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "${API_BASE}/api/v1/generate" \
            -H "Content-Type: application/json" \
            -H "$AUTH_HEADER" \
            -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\"}")
        if [ "$http_code" = "200" ]; then
            success=$((success + 1))
        fi
    done
    
    local end_time=$(date +%s%N)
    local duration=$((($end_time - $start_time) / 1000000))
    local tps=$((request_count * 1000 / duration))
    
    echo "$success/$request_count, ${duration}ms, TPS: $tps"
}
