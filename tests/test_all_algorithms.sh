#!/bin/bash
# 测试所有算法功能

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

main() {
    print_header "Nebula ID 算法测试套件"
    
    check_prerequisites
    if ! check_api_health; then
        echo "请先启动服务: cargo run --bin nebula-id"
        exit 1
    fi

    # 测试 Segment 算法
    test_segment

    # 测试 Segment 批量生成
    test_segment_batch

    # 测试 Snowflake 算法
    test_snowflake

    # 测试 Snowflake 批量生成
    test_snowflake_batch

    # 测试 UUID V7 算法
    test_uuid_v7

    # 测试 UUID V7 批量生成
    test_uuid_v7_batch

    print_header "所有算法测试完成"
}

test_segment() {
    print_section "测试默认 Segment 算法"
    
    local result=$(generate_id "test" "default" "test:default")
    local id=$(get_id_from_response "$result")
    local algo=$(get_algorithm_from_response "$result")
    
    echo "Response: $result"
    echo "ID: $id, Algorithm: $algo"
    echo "ID长度: ${#id} 位"
    verify_numeric_format "$id"
}

test_segment_batch() {
    print_section "测试 Segment 批量生成"
    
    local result=$(generate_batch "test" "default" "test:default" 5)
    local ids=$(echo "$result" | jq -r '.ids[]')
    local algo=$(get_algorithm_from_response "$result")
    local count=$(echo "$result" | jq -r '.ids | length')
    
    echo "Response: $result"
    echo "Generated $count IDs with algorithm: $algo"
    echo "$ids" | head -3
}

test_snowflake() {
    print_section "测试 Snowflake 算法"
    
    switch_algorithm "test:snowflake" "snowflake"
    
    for i in 1 2 3; do
        local result=$(generate_id "test" "snowflake" "test:snowflake")
        local id=$(get_id_from_response "$result")
        local algo=$(get_algorithm_from_response "$result")
        echo "  ID $i: $id (Algorithm: $algo)"
        if [ "$i" -eq 1 ]; then
            verify_numeric_format "$id"
        fi
    done
}

test_snowflake_batch() {
    print_section "测试 Snowflake 批量生成"
    
    local result=$(generate_batch "test" "snowflake" "test:snowflake" 10)
    local ids=$(echo "$result" | jq -r '.ids[]')
    local algo=$(get_algorithm_from_response "$result")
    local count=$(echo "$result" | jq -r '.ids | length')
    
    echo "Generated $count IDs with algorithm: $algo"
    echo "$ids" | head -5
}

test_uuid_v7() {
    print_section "测试 UUID V7 算法"
    
    switch_algorithm "test:uuid" "uuid_v7"
    
    local result=$(generate_id "test" "uuid" "test:uuid")
    local id=$(get_id_from_response "$result")
    local algo=$(get_algorithm_from_response "$result")
    
    echo "UUID: $id (algorithm: $algo)"
    verify_uuid_format "$id"
}

test_uuid_v7_batch() {
    print_section "测试 UUID V7 批量生成"
    
    local result=$(generate_batch "test" "uuid" "test:uuid" 5)
    local ids=$(echo "$result" | jq -r '.ids[]')
    local algo=$(get_algorithm_from_response "$result")
    local count=$(echo "$result" | jq -r '.ids | length')
    
    echo "Generated $count UUIDs with algorithm: $algo"
    echo "$ids" | head -3
    
    echo ""
    echo "验证 UUID 格式:"
    echo "$ids" | while read uuid; do
        verify_uuid_format "$uuid"
    done
}

main "$@"
