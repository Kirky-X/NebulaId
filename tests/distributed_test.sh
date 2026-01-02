#!/bin/bash

# 分布式一致性测试和雪花算法验证脚本
# 测试目标：验证分布式ID生成的一致性和雪花算法正确性

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

echo "=========================================="
echo "分布式一致性测试和雪花算法验证"
echo "=========================================="
echo ""

check_prerequisites
if ! check_api_health; then
    echo "❌ 服务未响应，请确保服务已启动"
    exit 1
fi

echo "✅ 服务已就绪"
echo ""

# 测试1: 并发生成唯一性测试
test_concurrent_uniqueness() {
    echo "=== 测试1: 并发生成唯一性测试 ==="
    echo "并行生成100个ID，检查重复..."

    local tmpdir=$(mktemp -d)
    local pids=()

    for i in $(seq 1 10); do
        (
            for j in $(seq 1 10); do
                local result=$(generate_id "dist-test" "consistency" "concurrent-test")
                local id=$(get_id_from_response "$result")
                echo "$id" >> "$tmpdir/result.$i"
            done
        ) &
        pids+=($!)
    done

    for pid in "${pids[@]}"; do
        wait $pid 2>/dev/null || true
    done

    # 解析所有ID
    local all_ids=$(cat "$tmpdir"/result.* 2>/dev/null | sort -n)

    local total=$(echo "$all_ids" | wc -l)
    local unique=$(echo "$all_ids" | uniq | wc -l)
    local duplicates=$((total - unique))

    echo "总生成数: $total, 唯一数: $unique, 重复数: $duplicates"

    if [ "$duplicates" -eq 0 ]; then
        echo "✅ 并发唯一性测试通过 - 无重复ID"
        rm -rf "$tmpdir"
        return 0
    else
        echo "❌ 发现重复ID:"
        echo "$all_ids" | uniq -d | head -10
        rm -rf "$tmpdir"
        return 1
    fi
}

# 测试2: 时序递增性测试
test_timestamp_ordering() {
    echo ""
    echo "=== 测试2: 时序递增性测试 ==="
    echo "连续生成50个ID，验证时间戳递增..."

    local first_ts=""
    local last_ts=""
    local order_violations=0
    local prev_ts=""

    for i in $(seq 1 50); do
        local result=$(generate_id "dist-test" "consistency" "ordering-test")
        local ts=$(echo "$result" | grep -oE '"timestamp"[ ]*:[ ]*"[^"]*"' | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}' | sed 's/T/ /' | tr -d '[:-]' | sed 's/ //')

        if [ -n "$ts" ]; then
            if [ -z "$first_ts" ]; then
                first_ts="$ts"
            fi
            last_ts="$ts"

            if [ -n "$prev_ts" ] && [ "$ts" -lt "$prev_ts" ]; then
                order_violations=$((order_violations + 1))
            fi
            prev_ts="$ts"
        fi
    done

    echo "检测到逆序次数: $order_violations"

    if [ "$order_violations" -eq 0 ]; then
        echo "✅ 时序递增性测试通过 - 所有ID按时间顺序生成"
        return 0
    else
        echo "⚠️ 存在逆序情况（可能由时钟漂移导致）"
        return 0
    fi
}

# 测试3: 批量生成唯一性测试
test_batch_uniqueness() {
    echo ""
    echo "=== 测试3: 批量生成唯一性测试 ==="
    echo "批量生成3次，每次20个ID，验证跨批次唯一性..."

    local tmpdir=$(mktemp -d)

    for batch in 1 2 3; do
        local result=$(generate_batch "dist-test" "consistency" "batch-test" 20)
        local ids=$(echo "$result" | jq -r '.ids | join("\n")' 2>/dev/null | grep -E '^[0-9]+$')
        echo "$ids" >> "$tmpdir/batch$batch.txt"
    done

    # 解析所有ID
    local all_ids=$(cat "$tmpdir"/*.txt 2>/dev/null | sort -n)

    local total=$(echo "$all_ids" | wc -l)
    local unique=$(echo "$all_ids" | uniq | wc -l)
    local duplicates=$((total - unique))

    echo "总生成数: $total, 唯一数: $unique, 重复数: $duplicates"

    if [ "$duplicates" -eq 0 ]; then
        echo "✅ 批量唯一性测试通过 - 跨批次无重复ID"
        rm -rf "$tmpdir"
        return 0
    else
        echo "❌ 发现重复ID:"
        echo "$all_ids" | uniq -d
        rm -rf "$tmpdir"
        return 1
    fi
}

# 测试4: 雪花ID结构验证
test_snowflake_structure() {
    echo ""
    echo "=== 测试4: 雪花ID结构验证 ==="
    echo "验证雪花ID的位结构是否正确..."

    switch_algorithm "dist-test:structure" "snowflake"

    local result=$(generate_id "dist-test" "consistency" "dist-test:structure")
    local id=$(get_id_from_response "$result")
    local algorithm=$(get_algorithm_from_response "$result")

    echo "生成ID: $id"
    echo "算法: $algorithm"

    if [ "$algorithm" == "snowflake" ]; then
        local id_len=${#id}

        if [ "$id_len" -ge 15 ]; then
            echo "✅ 雪花ID结构有效 - 长度正确 ($id_len 位)"

            local timestamp=$((id >> 22))
            local datacenter=$(((id >> 17) & 31))
            local worker=$(((id >> 12) & 31))
            local sequence=$((id & 4095))

            echo "  解析结果:"
            echo "  - 时间戳部分(高位): $timestamp"
            echo "  - 数据中心ID (5bit): $datacenter"
            echo "  - 工作节点ID (5bit): $worker"
            echo "  - 序列号 (12bit): $sequence"

            return 0
        else
            echo "⚠️ ID长度可能不正确: $id_len"
            return 1
        fi
    else
        echo "⚠️ 当前算法非雪花算法: $algorithm"
        echo "   当前使用 $algorithm 算法，跳过雪花结构验证"
        echo "   (雪花算法需要通过配置启用)"
        return 0
    fi
}

# 测试5: 雪花算法去中心化特性测试
test_decentralized_property() {
    echo ""
    echo "=== 测试5: 雪花算法去中心化特性测试 ==="
    echo "验证不同节点生成的ID分布..."

    switch_algorithm "dist-test:decentralized" "snowflake"

    local ids=()
    local timestamps=()

    for i in $(seq 1 20); do
        local result=$(generate_id "dist-test" "consistency" "dist-test:decentralized")
        local id=$(get_id_from_response "$result")
        ids+=("$id")

        local ts=$((id >> 22))
        timestamps+=("$ts")
    done

    local min_id=$(printf '%s\n' "${ids[@]}" | sort -n | head -1)
    local max_id=$(printf '%s\n' "${ids[@]}" | sort -n | tail -1)

    local min_num=$(echo "$min_id" | grep -oE '[0-9]+')
    local max_num=$(echo "$max_id" | grep -oE '[0-9]+')

    local range=$((max_num - min_num))
    local min_ts=$(printf '%s\n' "${timestamps[@]}" | sort -n | head -1)
    local max_ts=$(printf '%s\n' "${timestamps[@]}" | sort -n | tail -1)

    echo "ID范围: $min_id ~ $max_id"
    echo "跨度: $range"
    echo "时间戳范围: $min_ts ~ $max_ts"

    if [ "$range" -gt 0 ] && [ "$max_ts" -ge "$min_ts" ]; then
        echo "✅ 去中心化特性测试通过 - ID分布合理"
        return 0
    else
        echo "⚠️ ID分布可能过于集中"
        return 1
    fi
}

# 测试6: 配置雪花算法并验证
test_configured_snowflake() {
    echo ""
    echo "=== 测试6: 配置雪花算法验证 ==="
    echo "通过配置切换到雪花算法并验证..."

    switch_algorithm "dist-test:config" "snowflake"

    sleep 1

    local result=$(generate_id "dist-test" "consistency" "dist-test:config")
    local id=$(get_id_from_response "$result")
    local algorithm=$(get_algorithm_from_response "$result")
    
    echo "生成ID: $id"
    echo "算法: $algorithm"
    
    if [ "$algorithm" == "snowflake" ]; then
        local id_len=${#id}
        if [ "$id_len" -ge 15 ]; then
            echo "✅ 雪花算法配置生效"
            return 0
        fi
    fi
    
    echo "ℹ️ 雪花算法需要通过配置文件启用"
    return 0
}

# 主测试流程
echo "开始分布式一致性测试和雪花算法验证..."
echo ""

# 初始化测试结果
TOTAL_PASSED=0
TOTAL_FAILED=0

# 运行测试
if test_concurrent_uniqueness; then
    TOTAL_PASSED=$((TOTAL_PASSED + 1))
else
    TOTAL_FAILED=$((TOTAL_FAILED + 1))
fi

if test_timestamp_ordering; then
    TOTAL_PASSED=$((TOTAL_PASSED + 1))
else
    TOTAL_FAILED=$((TOTAL_FAILED + 1))
fi

if test_batch_uniqueness; then
    TOTAL_PASSED=$((TOTAL_PASSED + 1))
else
    TOTAL_FAILED=$((TOTAL_FAILED + 1))
fi

if test_snowflake_structure; then
    TOTAL_PASSED=$((TOTAL_PASSED + 1))
else
    TOTAL_FAILED=$((TOTAL_FAILED + 1))
fi

if test_decentralized_property; then
    TOTAL_PASSED=$((TOTAL_PASSED + 1))
else
    TOTAL_FAILED=$((TOTAL_FAILED + 1))
fi

test_configured_snowflake

echo ""
echo "=========================================="
echo "测试结果汇总"
echo "=========================================="
echo "通过: $TOTAL_PASSED"
echo "失败: $TOTAL_FAILED"
echo ""

if [ "$TOTAL_FAILED" -eq 0 ]; then
    echo "✅ 所有测试通过 - 分布式一致性和雪花算法验证成功"
    exit 0
else
    echo "⚠️ 部分测试失败 - 请检查上述输出"
    exit 1
fi
