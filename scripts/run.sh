#!/usr/bin/env bash
# Nebula ID — Unified Script Entry
# Dispatches to _impl files in this directory.
# Usage: scripts/run.sh <subcommand> [args...]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
    cat <<'EOF'
Usage: scripts/run.sh <subcommand> [args...]

Subcommands:
    deploy          Deploy Nebula ID via docker-compose
    lint            Alias for pre-commit (run local CI checks)
    redis-test      Run Redis integration tests
    api-test        Run API endpoint tests (optional: server_url)
    install-hooks   Install git pre-commit hooks
    pre-commit      Run local CI pre-checks (fmt + clippy + test)
    help            Show this help message

Examples:
    scripts/run.sh deploy
    scripts/run.sh pre-commit
    scripts/run.sh api-test http://localhost:8080
    scripts/run.sh install-hooks
EOF
}

case "${1:-help}" in
    deploy)
        shift
        exec bash "$SCRIPT_DIR/_deploy_impl.sh" "$@"
        ;;
    lint|pre-commit)
        shift
        exec bash "$SCRIPT_DIR/_pre_commit_impl.sh" "$@"
        ;;
    redis-test)
        shift
        exec bash "$SCRIPT_DIR/_redis_test_impl.sh" "$@"
        ;;
    api-test)
        shift
        exec bash "$SCRIPT_DIR/_api_test_impl.sh" "$@"
        ;;
    install-hooks)
        shift
        exec bash "$SCRIPT_DIR/_install_hooks_impl.sh" "$@"
        ;;
    help|--help|-h)
        usage
        ;;
    *)
        echo "Error: unknown subcommand '$1'" >&2
        echo ""
        usage
        exit 1
        ;;
esac
