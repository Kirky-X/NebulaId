#!/bin/bash

echo "🧪 测试 gRPC 端口连接"
echo "======================"

GRPC_HOST=${1:-"localhost"}
GRPC_PORT=${2:-9091}

echo "测试 gRPC 端口: ${GRPC_HOST}:${GRPC_PORT}"

# 测试1: 检查端口是否开放
echo "测试1: 检查端口是否开放"
if nc -zv ${GRPC_HOST} ${GRPC_PORT} 2>&1 | grep -q "succeeded"; then
    echo "✅ 端口 ${GRPC_PORT} 是开放的"
else
    echo "❌ 端口 ${GRPC_PORT} 未开放或连接被拒绝"
fi
echo ""

# 测试2: 检查进程是否在监听
echo "测试2: 检查进程监听状态"
if command -v netstat >/dev/null 2>&1; then
    netstat -tuln | grep ":${GRPC_PORT}" || echo "未找到监听端口 ${GRPC_PORT} 的进程"
elif command -v ss >/dev/null 2>&1; then
    ss -tuln | grep ":${GRPC_PORT}" || echo "未找到监听端口 ${GRPC_PORT} 的进程"
else
    echo "无法检查端口监听状态 (需要 netstat 或 ss)"
fi
echo ""

# 测试3: 检查服务日志
echo "测试3: 检查服务日志"
if [ -f "./logs/nebula-id.log" ]; then
    echo "最近的日志:"
    tail -20 ./logs/nebula-id.log | grep -i "grpc\|port\|9091\|50051" || echo "未找到相关日志"
else
    echo "日志文件不存在"
fi
echo ""

echo "✅ gRPC 端口测试完成"