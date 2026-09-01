#!/bin/bash

set -e

# ============================================
# Nebula ID 部署脚本
# ============================================

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# 配置
COMPOSE_FILE="docker/docker-compose.yml"
ENV_FILE="docker/.env"
BACKUP_DIR="docker/backups"
MAX_RETRIES=30
RETRY_INTERVAL=2

# 日志函数
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# 检查环境依赖
check_environment() {
    log_step "检查环境依赖..."

    if ! command -v docker &> /dev/null; then
        log_error "Docker 未安装，请先安装 Docker"
        exit 1
    fi

    if ! command -v docker compose &> /dev/null; then
        log_error "Docker Compose 未安装，请先安装 Docker Compose"
        exit 1
    fi

    if ! command -v curl &> /dev/null; then
        log_warn "curl 未安装，某些健康检查可能失败"
    fi

    log_success "环境检查完成 ✓"
}

# 验证环境变量
validate_env_vars() {
    log_step "验证环境变量..."

    if [ ! -f "$ENV_FILE" ]; then
        if [ -f "docker/.env.example" ]; then
            log_warn ".env 文件不存在，从 .env.example 创建..."
            cp docker/.env.example docker/.env
            log_info "已创建 .env 文件，请根据需要修改配置"
        else
            log_error ".env 文件不存在且无法创建"
            exit 1
        fi
    fi

    # 检查必需的环境变量
    source "$ENV_FILE"

    if [ -z "$POSTGRES_PASSWORD" ]; then
        log_error "POSTGRES_PASSWORD 未设置"
        exit 1
    fi

    log_success "环境变量验证完成 ✓"
}

# 检查数据库初始化状态
check_database_init() {
    log_step "检查数据库初始化状态..."

    if docker compose -f "$COMPOSE_FILE" ps postgres 2>/dev/null | grep -q "Up"; then
        log_info "PostgreSQL 正在运行，检查初始化状态..."

        # 尝试连接数据库
        if docker compose -f "$COMPOSE_FILE" exec -T postgres \
            psql -U idgen -d idgen -c "SELECT 1" &> /dev/null; then
            log_success "数据库已初始化 ✓"
        else
            log_warn "数据库未完全初始化，将在首次启动时自动初始化"
        fi
    else
        log_info "PostgreSQL 未运行，将在首次启动时初始化"
    fi
}

# 停止现有服务
stop_services() {
    log_step "停止现有服务..."

    if docker compose -f "$COMPOSE_FILE" ps 2>/dev/null | grep -q "Up"; then
        docker compose -f "$COMPOSE_FILE" down --remove-orphans
        log_success "现有服务已停止 ✓"
    else
        log_info "没有运行中的服务"
    fi
}

# 清理数据卷
clean_volumes() {
    log_step "清理数据卷..."

    log_warn "⚠️  这将删除所有数据卷，包括数据库数据！"
    log_warn "⚠️  此操作不可逆！"
    read -p "确认清理? (输入 'yes' 继续): " confirm

    if [ "$confirm" = "yes" ]; then
        log_info "停止服务..."
        docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true

        log_info "删除数据卷..."
        docker volume rm nebulaid_postgres_data nebulaid_redis_data nebulaid_etcd_data 2>/dev/null || true

        log_success "数据卷已清理 ✓"
    else
        log_info "已取消清理操作"
    fi
}

# 备份数据
backup_data() {
    log_step "备份数据..."

    local backup_dir="$BACKUP_DIR/$(date +%Y%m%d_%H%M%S)"
    mkdir -p "$backup_dir"

    # 备份 PostgreSQL
    if docker compose -f "$COMPOSE_FILE" ps postgres 2>/dev/null | grep -q "Up"; then
        log_info "备份 PostgreSQL 数据..."
        docker compose -f "$COMPOSE_FILE" exec -T postgres \
            pg_dump -U idgen idgen > "$backup_dir/postgres_backup.sql"
        log_success "PostgreSQL 备份完成 ✓"
    fi

    # 备份配置
    if [ -f "$ENV_FILE" ]; then
        cp "$ENV_FILE" "$backup_dir/.env"
        log_success "配置文件备份完成 ✓"
    fi

    log_success "数据已备份到: $backup_dir"
}

# 构建应用
build_app() {
    log_step "构建应用镜像..."

    local build_args=""
    if [ "$1" = "--no-cache" ]; then
        build_args="--no-cache"
    fi

    docker compose -f "$COMPOSE_FILE" build $build_args app
    log_success "应用镜像构建完成 ✓"
}

# 启动服务
start_services() {
    log_step "启动所有服务..."

    docker compose -f "$COMPOSE_FILE" up -d

    log_info "等待服务启动..."
    sleep 10

    log_info "检查服务健康状态..."
    docker compose -f "$COMPOSE_FILE" ps
}

# 等待服务就绪
wait_for_services() {
    log_step "等待服务就绪..."

    local attempt=1

    while [ $attempt -le $MAX_RETRIES ]; do
        log_info "健康检查尝试 $attempt/$MAX_RETRIES..."

        # 检查应用健康端点
        if curl -sf http://localhost:8080/health > /dev/null 2>&1; then
            log_success "应用服务已就绪 ✓"
            return 0
        fi

        # 检查服务状态
        local unhealthy_count=$(docker compose -f "$COMPOSE_FILE" ps --format json | \
            jq -r '[.[] | select(.Health != "healthy" and .State == "running")] | length' 2>/dev/null || echo "0")

        if [ "$unhealthy_count" -eq "0" ]; then
            log_success "所有服务已就绪 ✓"
            return 0
        fi

        sleep $RETRY_INTERVAL
        ((attempt++))
    done

    log_error "服务启动超时"
    log_info "请查看日志了解详情: $0 logs"
    return 1
}

# 验证服务健康状态
verify_services() {
    log_step "验证服务健康状态..."

    local services=("postgres" "redis" "etcd" "app")
    local all_healthy=true

    for service in "${services[@]}"; do
        local status=$(docker inspect -f '{{.State.Health.Status}}' nebula-$service 2>/dev/null || echo "not found")
        local state=$(docker inspect -f '{{.State.Status}}' nebula-$service 2>/dev/null || echo "not found")

        if [ "$status" = "healthy" ]; then
            log_info "  $service: $status ✓"
        elif [ "$state" = "running" ] && [ "$status" = "starting" ]; then
            log_warn "  $service: 启动中..."
            all_healthy=false
        elif [ "$state" = "running" ]; then
            log_warn "  $service: $status (运行中)"
        else
            log_error "  $service: $state"
            all_healthy=false
        fi
    done

    if [ "$all_healthy" = false ]; then
        log_warn "部分服务未就绪，但应用可能仍在启动中"
        log_info "请稍后检查: $0 status"
    fi
}

# 显示服务状态
show_status() {
    echo ""
    echo "📊 服务状态"
    echo "============"
    docker compose -f "$COMPOSE_FILE" ps

    echo ""
    echo "🌐 访问地址"
    echo "============"
    echo "  HTTP API:  http://localhost:8080"
    echo "  Metrics:   http://localhost:9092"
    echo "  Health:    http://localhost:8080/health"

    echo ""
    echo "🔧 常用命令"
    echo "============"
    echo "  查看日志:    $0 logs [service]"
    echo "  停止服务:    $0 stop"
    echo "  重新启动:    $0 restart"
    echo "  完全清理:    $0 clean"
    echo "  备份数据:    $0 backup"
    echo "  查看状态:    $0 status"
}

# 显示帮助信息
show_help() {
    cat << EOF
用法: $0 {start|stop|restart|clean|rebuild|status|logs|backup|help}

命令:
  start           启动所有服务（默认）
  stop            停止所有服务
  restart         重启所有服务
  clean           停止服务并清理数据卷
  rebuild         重新构建并启动
  status          查看服务状态
  logs [service]  查看日志（可指定服务名）
  backup          备份数据
  help            显示此帮助信息

示例:
  $0 start              # 启动服务
  $0 logs app           # 查看应用日志
  $0 logs postgres      # 查看 PostgreSQL 日志
  $0 rebuild            # 重新构建并启动
  $0 clean              # 清理所有数据

EOF
}

# 主函数
main() {
    local mode=${1:-start}
    local arg2=${2:-}

    echo "🚀 Nebula ID 部署脚本"
    echo "====================="
    echo ""

    case $mode in
        start)
            check_environment
            validate_env_vars
            check_database_init
            stop_services
            build_app
            start_services
            wait_for_services
            verify_services
            show_status
            ;;
        stop)
            stop_services
            log_success "服务已停止 ✓"
            ;;
        restart)
            stop_services
            start_services
            wait_for_services
            verify_services
            show_status
            ;;
        clean)
            clean_volumes
            ;;
        rebuild)
            check_environment
            validate_env_vars
            stop_services
            build_app --no-cache
            start_services
            wait_for_services
            verify_services
            show_status
            ;;
        status)
            docker compose -f "$COMPOSE_FILE" ps
            ;;
        logs)
            if [ -z "$arg2" ]; then
                docker compose -f "$COMPOSE_FILE" logs -f
            else
                docker compose -f "$COMPOSE_FILE" logs -f "$arg2"
            fi
            ;;
        backup)
            backup_data
            ;;
        help|--help|-h)
            show_help
            ;;
        *)
            log_error "未知命令: $mode"
            echo ""
            show_help
            exit 1
            ;;
    esac
}

main "$@"
