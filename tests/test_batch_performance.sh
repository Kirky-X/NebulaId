#!/bin/bash
# 批量生成性能测试

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

main() {
    print_header "批量生成性能测试套件"
    
    check_prerequisites
    if ! check_api_health; then
        echo "请先启动服务: cargo run --bin nebula-id"
        exit 1
    fi

    test_segment_performance
    test_snowflake_performance
    test_uuid_v7_performance

    print_header "性能测试完成"
}

test_segment_performance() {
    print_section "Segment 批量生成性能测试"
    
    echo "测试参数: 批量大小=100, 并发数=10, 总请求数=1000"
    echo "开始时间: $(date +%H:%M:%S)"
    
    if command -v hey &> /dev/null; then
        echo "使用 hey 进行并发测试..."
        hey -n 1000 -c 10 -m POST -H "Content-Type: application/json" -H "$AUTH_HEADER" \
            -d '{"workspace": "perf", "group": "default", "biz_tag": "perf:segment", "size": 100}' \
            "${API_BASE}/api/v1/generate/batch"
    else
        echo "使用 curl 串行测试 (单次批量 100 个)..."
        
        start_time=$(date +%s%N)
        for i in {1..10}; do
            local result=$(generate_batch "perf" "default" "perf:segment" 100)
            local ids_count=$(echo "$result" | jq -r '.ids | length')
            echo "请求 $i: 生成 $ids_count 个 IDs"
        done
        end_time=$(date +%s%N)
        duration=$((($end_time - $start_time) / 1000000))
        echo ""
        echo "总耗时: ${duration}ms"
        echo "平均每批: $((duration / 10))ms"
    fi
}

test_snowflake_performance() {
    print_section "Snowflake 批量生成性能测试"
    
    switch_algorithm "perf:snowflake" "snowflake"
    
    echo "测试参数: 批量大小=50, 并发数=5, 总请求数=500"
    echo "开始时间: $(date +%H:%M:%S)"
    
    if command -v hey &> /dev/null; then
        hey -n 500 -c 5 -m POST -H "Content-Type: application/json" -H "$AUTH_HEADER" \
            -d '{"workspace": "perf", "group": "snowflake", "biz_tag": "perf:snowflake", "size": 50}' \
            "${API_BASE}/api/v1/generate/batch"
    else
        start_time=$(date +%s%N)
        for i in {1..10}; do
            local result=$(generate_batch "perf" "snowflake" "perf:snowflake" 50)
            local ids_count=$(echo "$result" | jq -r '.ids | length')
            echo "请求 $i: 生成 $ids_count 个 IDs"
        done
        end_time=$(date +%s%N)
        duration=$((($end_time - $start_time) / 1000000))
        echo ""
        echo "总耗时: ${duration}ms"
        echo "平均每批: $((duration / 10))ms"
    fi
}

test_uuid_v7_performance() {
    print_section "UUID V7 批量生成性能测试"
    
    switch_algorithm "perf:uuid" "uuid_v7"
    
    echo "测试参数: 批量大小=50, 并发数=5, 总请求数=500"
    echo "开始时间: $(date +%H:%M:%S)"
    
    if command -v hey &> /dev/null; then
        hey -n 500 -c 5 -m POST -H "Content-Type: application/json" -H "$AUTH_HEADER" \
            -d '{"workspace": "perf", "group": "uuid", "biz_tag": "perf:uuid", "size": 50}' \
            "${API_BASE}/api/v1/generate/batch"
    else
        start_time=$(date +%s%N)
        for i in {1..10}; do
            local result=$(generate_batch "perf" "uuid" "perf:uuid" 50)
            local ids_count=$(echo "$result" | jq -r '.ids | length')
            echo "请求 $i: 生成 $ids_count 个 UUIDs"
        done
        end_time=$(date +%s%N)
        duration=$((($end_time - $start_time) / 1000000))
        echo ""
        echo "总耗时: ${duration}ms"
        echo "平均每批: $((duration / 10))ms"
    fi
}

main "$@"
