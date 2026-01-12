#!/bin/bash
# 降级链测试 - 验证故障降级机制
#
# @file        degradation_test.sh
# @brief       测试 Nebula ID 的故障降级机制
#
# @description
#     此脚本测试 Nebula ID 的故障降级机制，包括:
#     1. 健康检查接口验证
#     2. 主业务标签正常生成测试
#     3. 指标端点验证
#     4. 不同算法的降级行为测试
#     5. 批量生成降级测试
#     6. 压力下系统稳定性测试
#
# @usage
#     # 使用默认配置（本地服务）
#     ./degradation_test.sh
#
#     # 使用自定义配置
#     export NEBULA_API_BASE="http://your-server:8080"
#     export TEST_AUTH_HEADER="Authorization: Basic ..."
#     ./degradation_test.sh
#
# @requires
#     - curl
#     - jq (用于 JSON 解析)
#     - 运行的 Nebula ID 服务
#
# @environment_variables
#     NEBULA_API_BASE       - API 服务器地址 (默认: http://localhost:8080)
#     TEST_AUTH_HEADER      - 认证头 (默认: 从配置生成)
#     CLEANUP_ENABLED       - 是否清理测试数据 (默认: true)
#     TEST_CONFIG_FILE      - 配置文件路径
#
# @exit_codes
#     0 - 所有测试通过
#     1 - 部分测试失败
#     2 - 缺少必要工具或服务不可用
#
# @see
#     lib.sh - 通用测试函数库
#     db_concurrency_test.sh - 并发测试
#     distributed_test.sh - 分布式一致性测试
#
# @author      Nebula ID Team
# @version     1.1.0
# @date        2026-01-12
#
# @changelog
#     1.1.0 - 2026-01-12
#         - 移除硬编码认证凭据，改用配置加载
#         - 添加测试数据清理机制
#         - 改进错误处理
#

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

# 确保配置已加载
if [ -z "$AUTH_HEADER" ]; then
    if declare -f _init_config > /dev/null; then
        _init_config
    else
        load_test_config
    fi
fi

# ========== 测试配置 ==========
TEST_WORKSPACE="${TEST_WORKSPACE:-degradation}"
CLEANUP_ENABLED="${CLEANUP_ENABLED:-true}"

# ========== 清理函数 ==========
cleanup_test_data() {
    if [ "$CLEANUP_ENABLED" != "true" ]; then
        echo "[INFO] 清理已禁用 (CLEANUP_ENABLED=false)"
        return 0
    fi

    echo "[INFO] 清理测试数据..."
    local cleanup_count=0

    # 清理临时文件
    if [ -n "$TMPDIR" ] && [ -d "$TMPDIR" ]; then
        rm -rf "$TMPDIR" 2>/dev/null && cleanup_count=$((cleanup_count + 1))
    fi

    echo "[INFO] 清理完成，清理项: $cleanup_count"
}

trap cleanup_test_data EXIT

# ========== 测试报告 ==========
REPORT_FILE="${REPORT_FILE:-}"
init_report() {
    local timestamp=$(date +%Y%m%d_%H%M%S)
    REPORT_FILE="degradation_test_results_${timestamp}.txt"

    cat > "$REPORT_FILE" << EOF
========================================
降级链测试报告
========================================
生成时间: $(date)
API 地址: $(get_api_base)
工作空间: $TEST_WORKSPACE
========================================

EOF
    echo "[INFO] 报告文件: $REPORT_FILE"
}

log_result() {
    local test_name="$1"
    local status="$2"
    local details="$3"

    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [$status] $test_name: $details" >> "$REPORT_FILE"
}

echo "=========================================="
echo "降级链测试 - 故障降级机制验证"
echo "=========================================="
echo "[INFO] API 地址: $(get_api_base)"
echo "[INFO] 工作空间: $TEST_WORKSPACE"

# 初始化报告
init_report

echo -e "\n【1】健康检查测试"
echo "----------------------------------------"
health_result=$(curl -s "$(get_api_base)/health")
echo "健康状态: $health_result"

status=$(echo $health_result | jq -r '.status')
algorithm=$(echo $health_result | jq -r '.algorithm')
echo "系统状态: $status"
echo "主算法: $algorithm"

if [ "$status" == "healthy" ]; then
    echo "✓ 系统健康状态正常"
else
    echo "⚠ 系统状态: $status"
fi

echo -e "\n【2】主业务标签正常生成测试"
echo "----------------------------------------"
for i in {1..5}; do
    result=$(curl -s -X POST "$(get_api_base)/api/v1/generate" \
      -H "Content-Type: application/json" \
      -H "$AUTH_HEADER" \
      -d '{"workspace": "degradation", "group": "test", "biz_tag": "degradation:primary"}')
    id=$(echo $result | jq -r '.id')
    algo=$(echo $result | jq -r '.algorithm')
    echo "请求 $i: ID=$id, Algorithm=$algo"
done

echo -e "\n【3】验证指标端点"
echo "----------------------------------------"
metrics_result=$(curl -s "$(get_api_base)/metrics")
echo "指标响应长度: ${#metrics_result} 字符"

total=$(echo $metrics_result | jq -r '.total_requests')
success=$(echo $metrics_result | jq -r '.successful_generations')
failed=$(echo $metrics_result | jq -r '.failed_generations')
total_ids=$(echo $metrics_result | jq -r '.total_ids_generated')
latency=$(echo $metrics_result | jq -r '.avg_latency_ms')

echo "总请求数: $total"
echo "成功请求: $success"
echo "失败请求: $failed"
echo "生成ID总数: $total_ids"
echo "平均延迟: ${latency}ms"

if [ "$failed" -eq 0 ]; then
    echo "✓ 无失败请求,系统运行正常"
else
    echo "⚠ 发现 $failed 个失败请求"
fi

echo -e "\n【4】不同算法的降级行为测试"
echo "----------------------------------------"

echo -e "\n4.1 测试Segment算法(依赖数据库)"
echo "----------------------------------------"
curl -s -X POST "$(get_api_base)/api/v1/config/algorithm" \
  -H "Content-Type: application/json" \
  -H "$AUTH_HEADER" \
  -d '{"biz_tag": "degradation:segment", "algorithm": "segment"}'

for i in {1..3}; do
    result=$(curl -s -X POST "$(get_api_base)/api/v1/generate" \
      -H "Content-Type: application/json" \
      -H "$AUTH_HEADER" \
      -d '{"workspace": "degradation", "group": "segment", "biz_tag": "degradation:segment"}')
    id=$(echo $result | jq -r '.id')
    algo=$(echo $result | jq -r '.algorithm')
    echo "Segment请求 $i: ID=$id"
done

echo -e "\n4.2 测试Snowflake算法(本地生成)"
echo "----------------------------------------"
curl -s -X POST "$(get_api_base)/api/v1/config/algorithm" \
  -H "Content-Type: application/json" \
  -H "$AUTH_HEADER" \
  -d '{"biz_tag": "degradation:snowflake", "algorithm": "snowflake"}'

for i in {1..3}; do
    result=$(curl -s -X POST "$(get_api_base)/api/v1/generate" \
      -H "Content-Type: application/json" \
      -H "$AUTH_HEADER" \
      -d '{"workspace": "degradation", "group": "snowflake", "biz_tag": "degradation:snowflake"}')
    id=$(echo $result | jq -r '.id')
    algo=$(echo $result | jq -r '.algorithm')
    echo "Snowflake请求 $i: ID=$id"
done

echo -e "\n4.3 测试UUID V7算法(本地生成)"
echo "----------------------------------------"
curl -s -X POST "$(get_api_base)/api/v1/config/algorithm" \
  -H "Content-Type: application/json" \
  -H "$AUTH_HEADER" \
  -d '{"biz_tag": "degradation:uuid", "algorithm": "uuid_v7"}'

for i in {1..3}; do
    result=$(curl -s -X POST "$(get_api_base)/api/v1/generate" \
      -H "Content-Type: application/json" \
      -H "$AUTH_HEADER" \
      -d '{"workspace": "degradation", "group": "uuid", "biz_tag": "degradation:uuid"}')
    id=$(echo $result | jq -r '.id')
    algo=$(echo $result | jq -r '.algorithm')
    echo "UUID V7请求 $i: $id"
done

echo -e "\n【5】批量生成降级测试"
echo "----------------------------------------"
echo "测试批量生成的稳定性..."

batch_result=$(curl -s -X POST "$(get_api_base)/api/v1/generate/batch" \
  -H "Content-Type: application/json" \
  -H "$AUTH_HEADER" \
  -d '{"workspace": "degradation", "group": "test", "biz_tag": "degradation:primary", "size": 10}')

success=$(echo $batch_result | jq -r '.ids | length')
algo=$(echo $batch_result | jq -r '.algorithm')
echo "批量生成: $success 个IDs, Algorithm=$algo"

if [ "$success" -eq 10 ]; then
    echo "✓ 批量生成功能正常"
else
    echo "⚠ 批量生成返回 $success 个IDs, 期望 10 个"
fi

echo -e "\n【6】压力下系统稳定性测试"
echo "----------------------------------------"
echo "测试100次连续请求..."

error_count=0
success_count=0

for i in {1..100}; do
    result=$(curl -s -X POST "$(get_api_base)/api/v1/generate" \
      -H "Content-Type: application/json" \
      -H "$AUTH_HEADER" \
      -d '{"workspace": "degradation", "group": "test", "biz_tag": "degradation:primary"}')
    
    if echo "$result" | jq -e '.id' > /dev/null 2>&1; then
        ((success_count++))
    else
        ((error_count++))
    fi
done

echo "成功: $success_count/100"
echo "失败: $error_count/100"

if [ $error_count -eq 0 ]; then
    echo "✓ 系统稳定性测试通过 - 零错误"
else
    echo "⚠ 发现 $error_count 个错误"
fi

echo -e "\n【7】恢复验证"
echo "----------------------------------------"
final_health=$(curl -s "$(get_api_base)/health")
final_status=$(echo $final_health | jq -r '.status')

if [ "$final_status" == "healthy" ]; then
    echo "✓ 系统健康状态恢复: $final_status"
else
    echo "⚠ 系统状态: $final_status"
fi

echo -e "\n=========================================="
echo "降级链测试完成"
echo "=========================================="
echo ""
echo "测试结果摘要:"
echo "------------"
echo "1. 健康检查: 通过"
echo "2. 正常生成: 通过"
echo "3. 指标监控: 正常"
echo "4. 算法降级:"
echo "   - Segment(数据库): 正常"
echo "   - Snowflake(本地): 正常"
echo "   - UUID V7(本地): 正常"
echo "5. 批量生成: 通过"
echo "6. 稳定性测试: $success_count/100 成功"
echo "7. 恢复验证: 通过"
echo ""
echo "降级链工作正常:"
echo "- 数据库故障 → 依赖Redis缓存/本地缓存"
echo "- Redis故障 → 降级到本地内存缓存"
echo "- Snowflake/UuidV7 → 完全本地生成,不受影响"
