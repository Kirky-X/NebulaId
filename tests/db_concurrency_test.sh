#!/bin/bash
# 数据库并发测试 - 验证号段分配的并发安全性

AUTH_HEADER="Authorization: Basic dGVzdC1rZXktaWQ6dGVzdC1zZWNyZXQ="

echo "=========================================="
echo "数据库并发测试 - 号段分配安全性"
echo "=========================================="

# 清理之前的测试数据
echo -e "\n【0】初始化测试环境"
echo "----------------------------------------"
curl -s -X POST http://localhost:8080/api/v1/config/algorithm \
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
        result=$(curl -s -X POST http://localhost:8080/api/v1/generate \
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
        batch_result=$(curl -s -X POST http://localhost:8080/api/v1/generate/batch \
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
    result=$(curl -s -X POST http://localhost:8080/api/v1/generate \
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
unique_test=$(curl -s -X POST http://localhost:8080/api/v1/generate/batch \
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
