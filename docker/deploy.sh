#!/bin/bash

set -e

echo "🚀 Nebula ID 部署脚本"
echo "====================="

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# 检查环境
check_environment() {
    log_info "检查环境依赖..."
    
    if ! command -v docker &> /dev/null; then
        log_error "Docker 未安装，请先安装 Docker"
        exit 1
    fi
    
    if ! command -v docker compose &> /dev/null; then
        log_error "Docker Compose 未安装，请先安装 Docker Compose"
        exit 1
    fi
    
    log_info "环境检查完成 ✓"
}

# 停止现有服务
stop_services() {
    log_info "停止现有服务..."
    docker compose -f docker/docker-compose.yml down --remove-orphans 2>/dev/null || true
    log_info "现有服务已停止 ✓"
}

# 清理数据卷
clean_volumes() {
    log_warn "这将删除所有数据卷，是否继续? (输入 y 确认)"
    read -r confirm
    if [ "$confirm" = "y" ] || [ "$confirm" = "Y" ]; then
        log_info "清理数据卷..."
        docker volume rm nebulaid_postgres_data nebulaid_redis_data nebulaid_etcd_data 2>/dev/null || true
        docker volume rm nebulaid_cargo_cache 2>/dev/null || true
        log_info "数据卷已清理 ✓"
    else
        log_info "跳过数据卷清理"
    fi
}

# 构建应用
build_app() {
    log_info "构建应用镜像..."
    docker compose -f docker/docker-compose.yml build --no-cache app
    log_info "应用镜像构建完成 ✓"
}

# 启动服务
start_services() {
    log_info "启动所有服务..."
    docker compose -f docker/docker-compose.yml up -d
    
    log_info "等待服务启动..."
    sleep 10
    
    log_info "检查服务健康状态..."
    docker compose -f docker/docker-compose.yml ps
}

# 等待服务就绪
wait_for_services() {
    log_info "等待服务就绪..."
    
    local max_attempts=30
    local attempt=1
    
    while [ $attempt -le $max_attempts ]; do
        log_info "健康检查尝试 $attempt/$max_attempts..."
        
        if curl -s http://localhost:8080/health > /dev/null 2>&1; then
            log_info "应用服务已就绪 ✓"
            return 0
        fi
        
        sleep 2
        ((attempt++))
    done
    
    log_error "服务启动超时"
    return 1
}

# 验证服务
verify_services() {
    log_info "验证所有服务..."
    
    local services=("postgres" "redis" "etcd" "app")
    local all_healthy=true
    
    for service in "${services[@]}"; do
        local status=$(docker inspect -f '{{.State.Health.Status}}' nebula-$service 2>/dev/null || echo "not found")
        if [ "$status" = "healthy" ] || [ "$status" = "not found" ]; then
            log_info "  $service: $status"
        else
            log_error "  $service: $status"
            all_healthy=false
        fi
    done
    
    if [ "$all_healthy" = false ]; then
        log_warn "部分服务未就绪，但应用可能仍在启动中"
    fi
}

# 显示服务状态
show_status() {
    echo ""
    echo "📊 服务状态"
    echo "============"
    docker compose -f docker/docker-compose.yml ps
    
    echo ""
    echo "🌐 访问地址"
    echo "============"
    echo "  HTTP API:  http://localhost:8080"
    echo "  Metrics:   http://localhost:9091"
    echo "  Health:    http://localhost:8080/health"
    
    echo ""
    echo "🔧 常用命令"
    echo "============"
    echo "  查看日志:    docker compose -f docker/docker-compose.yml logs -f app"
    echo "  停止服务:    docker compose -f docker/docker-compose.yml down"
    echo "  重新启动:    docker compose -f docker/docker-compose.yml restart"
    echo "  完全清理:    ./docker/deploy.sh --clean"
}

# 主函数
main() {
    local mode=${1:-start}
    
    case $mode in
        start)
            check_environment
            stop_services
            build_app
            start_services
            wait_for_services
            verify_services
            show_status
            ;;
        stop)
            stop_services
            log_info "服务已停止"
            ;;
        restart)
            stop_services
            start_services
            wait_for_services
            show_status
            ;;
        clean)
            stop_services
            clean_volumes
            log_info "环境已完全清理"
            ;;
        rebuild)
            stop_services
            build_app
            start_services
            wait_for_services
            show_status
            ;;
        status)
            docker compose -f docker/docker-compose.yml ps
            ;;
        logs)
            docker compose -f docker/docker-compose.yml logs -f "${2:-app}"
            ;;
        *)
            echo "用法: $0 {start|stop|restart|clean|rebuild|status|logs}"
            echo ""
            echo "命令:"
            echo "  start   - 启动所有服务（默认）"
            echo "  stop    - 停止所有服务"
            echo "  restart - 重启所有服务"
            echo "  clean   - 停止服务并清理数据卷"
            echo "  rebuild - 重新构建并启动"
            echo "  status  - 查看服务状态"
            echo "  logs    - 查看日志（可指定服务名）"
            exit 1
            ;;
    esac
}

main "$@"
