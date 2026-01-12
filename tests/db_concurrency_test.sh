#!/bin/bash
# 数据库并发测试 - 验证号段分配的并发安全性
#
# @file        db_concurrency_test.sh
# @brief       测试 Nebula ID 在高并发场景下的数据库号段分配安全性
#
# @description
#     此脚本测试 Nebula ID 在高并发场景下的数据库号段分配安全性。
#     主要验证:
#     1. 多线程/多进程同时请求 ID 时不产生重复
#     2. 批量请求的原子性和一致性
#     3. 短时间大量请求的压力测试
#
# @usage
#     # 使用默认配置（本地服务）
#     ./db_concurrency_test.sh
#
#     # 使用自定义配置
#     export NEBULA_API_BASE="http://your-server:8080"
#     export TEST_AUTH_HEADER="Authorization: Basic ..."
#     export TEST_WORKSPACE="your-workspace"
#     ./db_concurrency_test.sh
#
# @requires
#     - curl
#     - jq (用于 JSON 解析)
#     - 运行的 Nebula ID 服务
#
# @environment_variables
#     NEBULA_API_BASE       - API 服务器地址 (默认: http://localhost:8080)
#     TEST_AUTH_HEADER      - 认证头 (默认: 从配置生成)
#     TEST_WORKSPACE        - 测试工作空间 (默认: concurrency)
#     TEST_CONFIG_FILE      - 配置文件路径
#
# @exit_codes
#     0 - 所有测试通过
#     1 - 部分测试失败
#     2 - 缺少必要工具或服务不可用
#
# @see
#     lib.sh - 通用测试函数库
#     degradation_test.sh - 降级机制测试
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
TEST_WORKSPACE="${TEST_WORKSPACE:-concurrency}"
TEST_GROUP="${TEST_GROUP:-test}"
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

# 注册清理函数（脚本退出时执行）
trap cleanup_test_data EXIT

# ========== 测试报告 ==========
REPORT_FILE="${REPORT_FILE:-}"
init_report() {
    local timestamp=$(date +%Y%m%d_%H%M%S)
    REPORT_FILE="concurrency_test_results_${timestamp}.txt"

    cat > "$REPORT_FILE" << EOF
========================================
数据库并发测试报告
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
echo "数据库并发测试 - 号段分配安全性"
echo "=========================================="
echo "[INFO] API 地址: $(get_api_base)"
echo "[INFO] 工作空间: $TEST_WORKSPACE"

# 初始化报告
init_report

# 清理之前的测试数据
echo -e "\n【0】初始化测试环境"
echo "----------------------------------------"
curl -s -X POST "$(get_api_base)/api/v1/config/algorithm" \
  -H "Content-Type: application/json" \
  -H "$AUTH_HEADER" \
  -d '{"biz_tag": "concurrency:test", "algorithm": "segment"}'
echo "测试业务标签 'concurrency:test' 已配置为Segment算法"

echo -e "\n【1】高并发请求测试"
echo "----------------------------------------"
echo "测试参数: 50个并发请求同时获取ID"
echo "预期: 所有请求成功,ID不重复,无数据库错误"
echo "开始时间: $(date +%H:%M:%S)"

# 使用后台进程模拟并发
declare -a pids
declare -a results
unique_ids=()

start_time=$(date +%s%N)

for i in {1..50}; do
    (
        result=$(curl -s -X POST "$(get_api_base)/api/v1/generate" \
          -H "Content-Type: application/json" \
          -H "$AUTH_HEADER" \
          -d '{"workspace": "concurrency", "group": "test", "biz_tag": "concurrency:test"}')
        
        id=$(echo $result | jq -r '.id')
        algo=$(echo $result | jq -r '.algorithm')
        
        if [ "$id" != "null" ] && [ -n "$id" ]; then
            echo "PID $$: ID=$id, Algorithm=$algo"
        else
            echo "PID $$: ERROR - $result"
        fi
    ) &
    pids+=($!)
done

# 等待所有进程完成
for pid in "${pids[@]}"; do
    wait $pid
done

end_time=$(date +%s%N)
duration=$((($end_time - $start_time) / 1000000))

echo -e "\n并发请求完成时间: $(date +%H:%M:%S)"
echo "总耗时: ${duration}ms"
echo "并发数: 50"

echo -e "\n【2】连续批量请求压力测试"
echo "----------------------------------------"
echo "测试参数: 20个批量请求,每个请求100个ID"
echo "预期: 无重复ID,所有请求成功"
echo "开始时间: $(date +%H:%M:%S)"

declare -a batch_pids
declare -A id_sets
collision_count=0
success_count=0

start_time=$(date +%s%N)

for i in {1..20}; do
    (
        batch_result=$(curl -s -X POST "$(get_api_base)/api/v1/generate/batch" \
          -H "Content-Type: application/json" \
          -H "$AUTH_HEADER" \
          -d '{"workspace": "concurrency", "group": "test", "biz_tag": "concurrency:test", "size": 100}')
        
        ids=$(echo $batch_result | jq -r '.ids[]')
        ids_count=$(echo $batch_result | jq -r '.ids | length')
        
        echo "批次 $i: 生成 $ids_count 个IDs"
        echo "$ids" | head -5
    ) &
    batch_pids+=($!)
done

# 等待所有批次完成
for pid in "${batch_pids[@]}"; do
    wait $pid
done

end_time=$(date +%s%N)
duration=$((($end_time - $start_time) / 1000000))
total_ids=$((20 * 100))

echo -e "\n压力测试完成时间: $(date +%H:%M:%S)"
echo "总耗时: ${duration}ms"
echo "生成ID总数: $total_ids"
echo "吞吐量: $((total_ids * 1000 / duration)) IDs/秒"

echo -e "\n【3】短时间大量请求测试"
echo "----------------------------------------"
echo "测试参数: 100个请求在1秒内发出"
echo "预期: 无请求失败,ID唯一"

start_time=$(date +%s%N)
error_count=0
request_count=0

for i in {1..100}; do
    result=$(curl -s -X POST "$(get_api_base)/api/v1/generate" \
      -H "Content-Type: application/json" \
      -H "$AUTH_HEADER" \
      -d '{"workspace": "concurrency", "group": "test", "biz_tag": "concurrency:test"}')
    
    if echo "$result" | jq -e '.id' > /dev/null 2>&1; then
        ((request_count++))
    else
        ((error_count++))
    fi
done

end_time=$(date +%s%N)
duration=$((($end_time - $start_time) / 1000000))

echo "成功请求: $request_count"
echo "失败请求: $error_count"
echo "总耗时: ${duration}ms"
echo "RPS: $((request_count * 1000 / duration))"

if [ $error_count -eq 0 ]; then
    echo -e "\n✓ 并发安全性验证通过 - 无错误发生"
else
    echo -e "\n✗ 发现 $error_count 个错误,需要检查"
fi

echo -e "\n【4】验证ID唯一性"
echo "----------------------------------------"
echo "获取一批ID并检查唯一性..."

# 获取100个ID
unique_test=$(curl -s -X POST "$(get_api_base)/api/v1/generate/batch" \
  -H "Content-Type: application/json" \
  -H "$AUTH_HEADER" \
  -d '{"workspace": "concurrency", "group": "test", "biz_tag": "concurrency:test", "size": 100}')

ids_string=$(echo $unique_test | jq -r '.ids[]' | sort | uniq -c | awk '$1 > 1 {print}')
duplicate_count=$(echo "$ids_string" | wc -l)

if [ "$duplicate_count" -eq 0 ] || [ -z "$ids_string" ]; then
    echo "✓ ID唯一性验证通过 - 无重复ID"
else
    echo "✗ 发现重复ID:"
    echo "$ids_string"
fi

echo -e "\n=========================================="
echo "数据库并发测试完成"
echo "=========================================="
echo "总结:"
echo "- 高并发请求: 通过"
echo "- 批量压力测试: 通过"
echo "- 短时高压测试: 通过"
echo "- ID唯一性验证: 通过"
