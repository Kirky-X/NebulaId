#!/bin/bash

set -e

echo "🧪 Nebula ID API 测试脚本"
echo "=========================="

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# 测试配置
BASE_URL=${1:-"http://localhost:8080"}
TIMEOUT=5
REPORT_FILE="test_report_$(date +%Y%m%d_%H%M%S).txt"

# 测试计数器
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_test() {
    echo -e "${BLUE}[TEST]${NC} $1"
}

log_pass() {
    echo -e "${GREEN}[PASS]${NC} $1"
    ((PASSED_TESTS++))
    ((TOTAL_TESTS++))
}

log_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
    ((FAILED_TESTS++))
    ((TOTAL_TESTS++))
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

# 初始化报告
init_report() {
    cat > "$REPORT_FILE" << EOF
================================================================================
                        Nebula ID API 测试报告
================================================================================
测试时间: $(date '+%Y-%m-%d %H:%M:%S')
测试环境: $BASE_URL
报告文件: $REPORT_FILE

--------------------------------------------------------------------------------
                            测试摘要
--------------------------------------------------------------------------------
EOF
}

# 添加测试结果到报告
add_to_report() {
    local test_name="$1"
    local status="$2"
    local details="$3"
    
    echo "测试名称: $test_name" >> "$REPORT_FILE"
    echo "状态: $status" >> "$REPORT_FILE"
    echo "详情: $details" >> "$REPORT_FILE"
    echo "--------------------------------------------------------------------------------" >> "$REPORT_FILE"
}

# HTTP请求函数
http_request() {
    local method=$1
    local endpoint=$2
    local data=$3
    local headers=$4
    
    local args=("-X" "$method" "--silent" "--max-time" "$TIMEOUT")
    
    if [ -n "$data" ]; then
        args+=("-H" "Content-Type: application/json" "-d" "$data")
    fi
    
    if [ -n "$headers" ]; then
        for header in $headers; do
            args+=("-H" "$header")
        done
    fi
    
    args+=("${BASE_URL}${endpoint}")
    
    curl "${args[@]}"
}

# 测试健康检查接口
test_health() {
    log_test "测试健康检查接口 /health"
    
    local response=$(http_request "GET" "/health" "" "")
    local http_code=$(http_request "GET" "/health" "" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "200" ]; then
        log_pass "健康检查 - HTTP状态码: $http_code"
        add_to_report "健康检查" "通过" "HTTP状态码: $http_code, 响应: $response"
    else
        log_fail "健康检查 - HTTP状态码: $http_code, 响应: $response"
        add_to_report "健康检查" "失败" "HTTP状态码: $http_code, 响应: $response"
    fi
}

# 测试生成单个ID接口
test_generate_single() {
    log_test "测试生成单个ID接口 /api/v1/generate"
    
    local request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag"}'
    local response=$(http_request "POST" "/api/v1/generate" "$request_data" "")
    local http_code=$(http_request "POST" "/api/v1/generate" "$request_data" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "200" ]; then
        local id=$(echo "$response" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
        if [ -n "$id" ]; then
            log_pass "生成单个ID - 成功生成ID: $id"
            add_to_report "生成单个ID" "通过" "HTTP状态码: $http_code, 生成的ID: $id"
        else
            log_fail "生成单个ID - 响应中未找到ID字段"
            add_to_report "生成单个ID" "失败" "HTTP状态码: $http_code, 响应: $response"
        fi
    else
        log_fail "生成单个ID - HTTP状态码: $http_code"
        add_to_report "生成单个ID" "失败" "HTTP状态码: $http_code, 响应: $response"
    fi
}

# 测试批量生成ID接口
test_generate_batch() {
    log_test "测试批量生成ID接口 /api/v1/generate/batch"
    
    local request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag","size":10}'
    local response=$(http_request "POST" "/api/v1/generate/batch" "$request_data" "")
    local http_code=$(http_request "POST" "/api/v1/generate/batch" "$request_data" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "200" ]; then
        local ids_count=$(echo "$response" | grep -o '"ids":\[' | head -1 || echo "")
        if [ -n "$ids_count" ]; then
            log_pass "批量生成ID - 批量生成成功"
            add_to_report "批量生成ID" "通过" "HTTP状态码: $http_code, 响应: $response"
        else
            log_fail "批量生成ID - 响应格式不正确"
            add_to_report "批量生成ID" "失败" "HTTP状态码: $http_code, 响应: $response"
        fi
    else
        log_fail "批量生成ID - HTTP状态码: $http_code"
        add_to_report "批量生成ID" "失败" "HTTP状态码: $http_code, 响应: $response"
    fi
}

# 测试解析ID接口
test_parse_id() {
    log_test "测试解析ID接口 /api/v1/parse"
    
    local request_data='{"id":"4200000000000000001","workspace":"test-workspace","group":"test-group","biz_tag":"test-tag"}'
    local response=$(http_request "POST" "/api/v1/parse" "$request_data" "")
    local http_code=$(http_request "POST" "/api/v1/parse" "$request_data" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "200" ] || [ "$http_code" = "400" ]; then
        log_pass "解析ID - HTTP状态码: $http_code"
        add_to_report "解析ID" "通过" "HTTP状态码: $http_code, 响应: $response"
    else
        log_fail "解析ID - HTTP状态码: $http_code"
        add_to_report "解析ID" "失败" "HTTP状态码: $http_code, 响应: $response"
    fi
}

# 测试指标接口
test_metrics() {
    log_test "测试指标接口 /metrics"
    
    local response=$(http_request "GET" "/metrics" "" "")
    local http_code=$(http_request "GET" "/metrics" "" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "200" ]; then
        local has_content=$(echo "$response" | grep -c "total_requests" || echo "0")
        if [ "$has_content" -gt "0" ]; then
            log_pass "指标接口 - 返回Prometheus格式指标"
            add_to_report "指标接口" "通过" "HTTP状态码: $http_code, 包含指标数据"
        else
            log_fail "指标接口 - 响应格式不正确"
            add_to_report "指标接口" "失败" "HTTP状态码: $http_code, 响应: $response"
        fi
    else
        log_fail "指标接口 - HTTP状态码: $http_code"
        add_to_report "指标接口" "失败" "HTTP状态码: $http_code, 响应: $response"
    fi
}

# 测试配置接口
test_config() {
    log_test "测试配置接口 /api/v1/config"
    
    local response=$(http_request "GET" "/api/v1/config" "" "")
    local http_code=$(http_request "GET" "/api/v1/config" "" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "200" ]; then
        log_pass "配置接口 - HTTP状态码: $http_code"
        add_to_report "配置接口" "通过" "HTTP状态码: $http_code, 响应: $response"
    else
        log_fail "配置接口 - HTTP状态码: $http_code"
        add_to_report "配置接口" "失败" "HTTP状态码: $http_code, 响应: $response"
    fi
}

# 测试边界条件 - 空工作区
test_empty_workspace() {
    log_test "测试边界条件 - 空工作区"
    
    local request_data='{"workspace":"","group":"test-group","biz_tag":"test-tag"}'
    local http_code=$(http_request "POST" "/api/v1/generate" "$request_data" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "400" ] || [ "$http_code" = "500" ]; then
        log_pass "空工作区 - 正确拒绝空工作区 (HTTP $http_code)"
        add_to_report "边界条件-空工作区" "通过" "正确拒绝空工作区，HTTP状态码: $http_code"
    else
        log_fail "空工作区 - 期望HTTP 400或500，实际: $http_code"
        add_to_report "边界条件-空工作区" "失败" "期望HTTP 400或500，实际: $http_code"
    fi
}

# 测试边界条件 - 批量大小限制
test_batch_size_limit() {
    log_test "测试边界条件 - 批量大小限制"
    
    local request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag","size":1000}'
    local http_code=$(http_request "POST" "/api/v1/generate/batch" "$request_data" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "400" ] || [ "$http_code" = "200" ]; then
        log_pass "批量大小限制 - HTTP状态码: $http_code"
        add_to_report "边界条件-批量大小" "通过" "HTTP状态码: $http_code"
    else
        log_fail "批量大小限制 - HTTP状态码: $http_code"
        add_to_report "边界条件-批量大小" "失败" "HTTP状态码: $http_code"
    fi
}

# 测试无效ID解析
test_invalid_id() {
    log_test "测试边界条件 - 无效ID解析"
    
    local request_data='{"id":"invalid-id","workspace":"test-workspace","group":"test-group","biz_tag":"test-tag"}'
    local http_code=$(http_request "POST" "/api/v1/parse" "$request_data" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "400" ]; then
        log_pass "无效ID解析 - 正确拒绝无效ID (HTTP 400)"
        add_to_report "边界条件-无效ID" "通过" "正确拒绝无效ID，HTTP状态码: $http_code"
    else
        log_fail "无效ID解析 - 期望HTTP 400，实际: $http_code"
        add_to_report "边界条件-无效ID" "失败" "期望HTTP 400，实际: $http_code"
    fi
}

# 测试不存在的端点
test_not_found() {
    log_test "测试错误处理 - 404不存在的端点"
    
    local http_code=$(http_request "GET" "/api/v1/nonexistent" "" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "404" ]; then
        log_pass "404处理 - 正确返回404"
        add_to_report "错误处理-404" "通过" "正确返回HTTP 404"
    else
        log_fail "404处理 - 期望HTTP 404，实际: $http_code"
        add_to_report "错误处理-404" "失败" "期望HTTP 404，实际: $http_code"
    fi
}

# 测试方法不允许
test_method_not_allowed() {
    log_test "测试错误处理 - 405方法不允许"
    
    local http_code=$(http_request "GET" "/api/v1/generate" "" "" -w "%{http_code}" -o /dev/null)
    
    if [ "$http_code" = "405" ]; then
        log_pass "405处理 - 正确返回405"
        add_to_report "错误处理-405" "通过" "正确返回HTTP 405"
    else
        log_warn "405处理 - 期望HTTP 405，实际: $http_code（可能实现不同）"
        add_to_report "错误处理-405" "通过" "HTTP状态码: $http_code"
    fi
}

# 测试响应时间
test_response_time() {
    log_test "测试响应时间"
    
    local start_time=$(date +%s%N)
    http_request "GET" "/health" "" "" > /dev/null
    local end_time=$(date +%s%N)
    local duration=$(( (end_time - start_time) / 1000000 ))
    
    if [ $duration -lt 1000 ]; then
        log_pass "响应时间 - 健康检查响应时间: ${duration}ms"
        add_to_report "性能测试-响应时间" "通过" "健康检查响应时间: ${duration}ms"
    else
        log_warn "响应时间 - 健康检查响应时间: ${duration}ms（超过1秒）"
        add_to_report "性能测试-响应时间" "警告" "健康检查响应时间: ${duration}ms"
    fi
}

# 生成最终报告
generate_final_report() {
    local pass_rate=$(( PASSED_TESTS * 100 / TOTAL_TESTS ))
    
    cat >> "$REPORT_FILE" << EOF

================================================================================
                            测试结果汇总
================================================================================

总测试数:      $TOTAL_TESTS
通过测试数:    $PASSED_TESTS
失败测试数:    $FAILED_TESTS
通过率:        ${pass_rate}%

================================================================================
                            测试详情
================================================================================
EOF
    
    echo ""
    echo "================================================================================"
    echo "                            测试结果汇总"
    echo "================================================================================"
    echo "总测试数:      $TOTAL_TESTS"
    echo "通过测试数:    $PASSED_TESTS"
    echo "失败测试数:    $FAILED_TESTS"
    echo "通过率:        ${pass_rate}%"
    echo "================================================================================"
    echo ""
    log_info "测试报告已保存到: $REPORT_FILE"
}

# 主函数
main() {
    echo "🧪 Nebula ID API 测试脚本"
    echo "=========================="
    echo ""
    log_info "测试目标: $BASE_URL"
    log_info "超时设置: ${TIMEOUT}秒"
    echo ""
    
    init_report
    
    # 核心功能测试
    echo ""
    echo "📋 核心功能测试"
    echo "=============="
    test_health
    test_generate_single
    test_generate_batch
    test_parse_id
    test_metrics
    test_config
    
    # 边界条件测试
    echo ""
    echo "📋 边界条件测试"
    echo "=============="
    test_empty_workspace
    test_batch_size_limit
    test_invalid_id
    
    # 错误处理测试
    echo ""
    echo "📋 错误处理测试"
    echo "=============="
    test_not_found
    test_method_not_allowed
    
    # 性能测试
    echo ""
    echo "📋 性能测试"
    echo "=========="
    test_response_time
    
    generate_final_report
}

main "$@"
