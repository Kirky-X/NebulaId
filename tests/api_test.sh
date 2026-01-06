#!/bin/bash

# Nebula ID API Comprehensive Test Suite V6
# Tests with all required parameters properly included
# Dynamic API key acquisition from workspace creation
# Support for salt-based authentication
#
# IMPORTANT: For production server, you must provide:
#   1. Admin API Key credentials (for setup):
#      export ADMIN_API_KEY_ID="your-admin-key-id"
#      export ADMIN_API_KEY_SECRET="your-admin-key-secret"
#
#   2. Salt value (for authentication):
#      export NEBULA_API_KEY_SALT="your-server-salt"
#
#   OR pre-configured user API key:
#      export USER_API_KEY_ID="your-user-key-id"
#      export USER_API_KEY_SECRET="your-user-key-secret"
#
# Environment setup:
#   BASE_URL - Server URL (default: production server)
#   For local development:
#     export BASE_URL="http://localhost:8080"

set -e

# ============================================
# API Configuration
# ============================================
# Server Configuration - defaults to production server
BASE_URL="${BASE_URL:-https://nebulaid.onrender.com}"
METRICS_URL="${METRICS_URL:-http://localhost:9091}"

# Admin API Key (Required for setup)
# SECURITY: These must be set via environment variables for security
# For local testing, export these before running the script:
#   export ADMIN_API_KEY_ID="your-admin-key-id"
#   export ADMIN_API_KEY_SECRET="your-admin-key-secret"
if [ -z "$ADMIN_API_KEY_ID" ] || [ -z "$ADMIN_API_KEY_SECRET" ]; then
    cat << 'EOF'
╔══════════════════════════════════════════════════════════════════════╗
║  ERROR: Admin API Key credentials not provided                       ║
╠══════════════════════════════════════════════════════════════════════╣
║  For testing, you need to provide admin credentials:                ║
║                                                                     ║
║    export ADMIN_API_KEY_ID="your-admin-key-id"                      ║
║    export ADMIN_API_KEY_SECRET="your-admin-key-secret"              ║
║                                                                     ║
║  Then run: ./api_test.sh                                            ║
║                                                                     ║
║  For local development with Docker:                                ║
║    make dev-up                                                      ║
║    export ADMIN_API_KEY_ID="..."  # Check docker logs              ║
║    export ADMIN_API_KEY_SECRET="..."                                ║
╚══════════════════════════════════════════════════════════════════════╝
EOF
    exit 1
fi

# Pre-configured User API Key (for production testing without admin key)
# Set these if you have a pre-configured user key in the database
USER_API_KEY_ID="${USER_API_KEY_ID:-}"
USER_API_KEY_SECRET="${USER_API_KEY_SECRET:-}"

# Use pre-configured user key if provided, otherwise use admin key to obtain dynamically
if [ -n "$USER_API_KEY_ID" ] && [ -n "$USER_API_KEY_SECRET" ]; then
    echo "[INFO] Using pre-configured user API key"
    API_KEY_ID="$USER_API_KEY_ID"
    API_KEY_SECRET="$USER_API_KEY_SECRET"
    API_AUTH_HEADER="Authorization: ApiKey ${API_KEY_ID}:${API_KEY_SECRET}"
    ADMIN_API_AUTH_HEADER="Authorization: ApiKey ${ADMIN_API_KEY_ID}:${ADMIN_API_KEY_SECRET}"
else
    ADMIN_API_AUTH_HEADER="Authorization: ApiKey ${ADMIN_API_KEY_ID}:${ADMIN_API_KEY_SECRET}"
fi

# Dynamic API Key (Will be obtained dynamically)
API_KEY_ID=""
API_KEY_SECRET=""
API_AUTH_HEADER=""

# Performance Test Configuration (reduced for production)
TEST_WORKSPACE="${TEST_WORKSPACE:-test-workspace-$(date +%s)}"
TEST_GROUP="${TEST_GROUP:-default}"
BATCH_SIZE_TEST="${BATCH_SIZE_TEST:-5}"
PERF_TEST_ITERATIONS="${PERF_TEST_ITERATIONS:-100}"
PERF_CONCURRENT_BATCHES="${PERF_CONCURRENT_BATCHES:-10}"
PERF_CONCURRENT_SIZE="${PERF_CONCURRENT_SIZE:-10}"

# Report Configuration
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="test_results_${TIMESTAMP}.txt"

# Test Counters
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
SKIPPED_TESTS=0

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_header() {
    echo -e "\n${BLUE}========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}========================================${NC}"
}

log_info() {
    echo -e "${YELLOW}[INFO]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Compute salted SHA256 hash for API key authentication
# Format: SHA256(salt:key_id:key_secret) - base64 encoded
compute_salted_hash() {
    local salt="$1"
    local key_id="$2"
    local key_secret="$3"
    echo -n "${salt}:${key_id}:${key_secret}" | sha256sum | awk '{print $1}' | xxd -r -p | base64
}

# Check if we have a salt value configured
has_salt_configured() {
    [ -n "$NEBULA_API_KEY_SALT" ]
}

log_header() {
    echo -e "\n${BLUE}========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}========================================${NC}"
}

log_test() {
    echo -e "${YELLOW}测试 $1: $2${NC}"
}

log_pass() {
    echo -e "  ${GREEN}[PASS]${NC} $1"
    PASSED_TESTS=$((PASSED_TESTS + 1))
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
}

log_fail() {
    echo -e "  ${RED}[FAIL]${NC} $1"
    FAILED_TESTS=$((FAILED_TESTS + 1))
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
}

log_skip() {
    echo -e "  ${YELLOW}[SKIP]${NC} $1"
    SKIPPED_TESTS=$((SKIPPED_TESTS + 1))
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
}

log_info() {
    echo -e "  ${YELLOW}[INFO]${NC} $1"
}

log_result() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [$2] $1: $3" >> "$REPORT_FILE"
}

record_test() {
    local status=$1
    local test_name=$2
    local details=$3
    if [ "$status" = "PASS" ]; then
        log_pass "$details"
    elif [ "$status" = "FAIL" ]; then
        log_fail "$details"
    else
        log_skip "$details"
    fi
    log_result "$test_name" "$status" "$details"
}

init_report() {
    echo "Nebula ID API Test Report" > "$REPORT_FILE"
    echo "Generated: $(date)" >> "$REPORT_FILE"
    echo "Base URL: $BASE_URL" >> "$REPORT_FILE"
    echo "Test Workspace: $TEST_WORKSPACE" >> "$REPORT_FILE"
    echo "========================================" >> "$REPORT_FILE"
}

check_service() {
    echo "Checking service availability..."
    if curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/health" | grep -q "200\|degraded"; then
        echo "Service is available"
        return 0
    else
        echo "Service is not available"
        return 1
    fi
}

obtain_user_key_for_test() {
    # Try to get a user key from an existing workspace for ID generation tests
    log_info "Attempting to obtain user key for ID generation tests..."

    # List workspaces and get the first one
    WORKSPACES_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET "$BASE_URL/api/v1/workspaces" \
        -H "$ADMIN_API_AUTH_HEADER")

    WORKSPACES_CODE=$(echo "$WORKSPACES_RESPONSE" | tail -n1)
    WORKSPACES_BODY=$(echo "$WORKSPACES_RESPONSE" | sed '$d')

    if [ "$WORKSPACES_CODE" != "200" ]; then
        echo "    [FAIL] Cannot list workspaces"
        return 1
    fi

    # Get first workspace name
    FIRST_WS=$(echo "$WORKSPACES_BODY" | grep -o '"name":"[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -z "$FIRST_WS" ]; then
        echo "    [FAIL] No workspaces found"
        return 1
    fi

    echo "    [DEBUG] Using workspace: $FIRST_WS"

    # Regenerate user key
    USER_KEY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/workspaces/$FIRST_WS/regenerate-user-key" \
        -H "$ADMIN_API_AUTH_HEADER")

    USER_KEY_CODE=$(echo "$USER_KEY_RESPONSE" | tail -n1)
    USER_KEY_BODY=$(echo "$USER_KEY_RESPONSE" | sed '$d')

    if [ "$USER_KEY_CODE" != "200" ]; then
        echo "    [FAIL] Cannot regenerate user key: $USER_KEY_BODY"
        return 1
    fi

    # Extract user key details
    TEST_API_KEY_SECRET=$(echo "$USER_KEY_BODY" | grep -o '"key_secret":"[^"]*"' | cut -d'"' -f4)
    TEST_API_KEY_ID=$(echo "$USER_KEY_BODY" | grep -o '"key_id":"[^"]*"' | cut -d'"' -f4)
    TEST_API_KEY_PREFIX=$(echo "$USER_KEY_BODY" | grep -o '"key_prefix":"[^"]*"' | cut -d'"' -f4)

    if [ -n "$TEST_API_KEY_SECRET" ] && [ -n "$TEST_API_KEY_ID" ]; then
        # Note: key_id now includes prefix in database
        TEST_WORKSPACE="$FIRST_WS"

        # If salt is configured, use Basic auth with salted hash
        if has_salt_configured; then
            local salted_hash=$(compute_salted_hash "$NEBULA_API_KEY_SALT" "$TEST_API_KEY_ID" "$TEST_API_KEY_SECRET")
            API_KEY_ID="$TEST_API_KEY_ID"
            API_KEY_SECRET="$TEST_API_KEY_SECRET"
            API_AUTH_HEADER="Authorization: Basic ${salted_hash}"
        else
            # Use ApiKey format
            API_KEY_ID="$TEST_API_KEY_ID"
            API_KEY_SECRET="$TEST_API_KEY_SECRET"
            API_AUTH_HEADER="Authorization: ApiKey ${API_KEY_ID}:${API_KEY_SECRET}"
        fi

        echo "    [DEBUG] User key obtained: ${API_KEY_ID:0:15}..."
        return 0
    fi

    echo "    [FAIL] Failed to extract user key details"
    return 1
}

obtain_api_key() {
    # If using pre-configured user key, skip acquisition
    if [ -n "$USER_API_KEY_ID" ] && [ -n "$USER_API_KEY_SECRET" ]; then
        log_info "Using pre-configured user API key: ${USER_API_KEY_ID:0:15}..."

        # If salt is configured, compute the salted hash for Basic auth
        if has_salt_configured; then
            log_info "Using salt-based authentication"
            # Basic auth format: base64(key_id:key_secret) with salted hash
            local salted_hash=$(compute_salted_hash "$NEBULA_API_KEY_SALT" "$USER_API_KEY_ID" "$USER_API_KEY_SECRET")
            API_KEY_ID="$USER_API_KEY_ID"
            API_KEY_SECRET="$USER_API_KEY_SECRET"
            API_AUTH_HEADER="Authorization: Basic ${salted_hash}"
        else
            API_KEY_ID="$USER_API_KEY_ID"
            API_KEY_SECRET="$USER_API_KEY_SECRET"
            API_AUTH_HEADER="Authorization: ApiKey ${API_KEY_ID}:${API_KEY_SECRET}"
        fi
        return 0
    fi

    log_info "Obtaining API key dynamically..."

    echo "    [DEBUG] Using Admin API Key ID: $ADMIN_API_KEY_ID"
    echo "    [DEBUG] Using Admin API Key Secret: ${ADMIN_API_KEY_SECRET:0:8}..."
    echo "    [DEBUG] Target workspace: $TEST_WORKSPACE"

    local create_workspace_response
    create_workspace_response=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/workspaces" \
        -H "Content-Type: application/json" \
        -H "$ADMIN_API_AUTH_HEADER" \
        -d "{\"name\":\"$TEST_WORKSPACE\",\"description\":\"Test workspace\",\"max_groups\":10,\"max_biz_tags\":100}")

    local http_code
    http_code=$(echo "$create_workspace_response" | tail -n1)
    local body
    body=$(echo "$create_workspace_response" | sed '$d')

    echo "    [DEBUG] Create workspace response code: $http_code"
    echo "    [DEBUG] Create workspace response body: $body"

    if [ "$http_code" = "200" ]; then
        API_KEY_SECRET=$(echo "$body" | grep -o '"key_secret":"[^"]*"' | cut -d'"' -f4)
        API_KEY_ID=$(echo "$body" | grep -o '"key_id":"[^"]*"' | cut -d'"' -f4)
        API_KEY_PREFIX=$(echo "$body" | grep -o '"key_prefix":"[^"]*"' | cut -d'"' -f4)

        echo "    [DEBUG] Extracted key_secret: ${API_KEY_SECRET:0:10}..."
        echo "    [DEBUG] Extracted key_id: $API_KEY_ID"
        echo "    [DEBUG] Extracted key_prefix: $API_KEY_PREFIX"

        if [ -n "$API_KEY_SECRET" ] && [ -n "$API_KEY_ID" ]; then
            # Note: key_id now includes prefix in database
            API_AUTH_HEADER="Authorization: ApiKey ${API_KEY_ID}:${API_KEY_SECRET}"
            echo "    [DEBUG] Final API_AUTH_HEADER: $API_AUTH_HEADER"
            log_info "API key obtained successfully: ${API_KEY_ID:0:15}..."
            return 0
        else
            echo "    [ERROR] Failed to extract API key from response"
        fi
    else
        echo "    [WARN] Workspace creation failed (HTTP $http_code), trying to use existing workspace..."
    fi

    echo "    [DEBUG] Listing existing workspaces..."
    list_workspace_response=$(curl -s -w "\n%{http_code}" -X GET "$BASE_URL/api/v1/workspaces" \
        -H "$ADMIN_API_AUTH_HEADER")

    local list_http_code
    list_http_code=$(echo "$list_workspace_response" | tail -n1)
    local list_body
    list_body=$(echo "$list_workspace_response" | sed '$d')

    echo "    [DEBUG] List workspaces response code: $list_http_code"
    echo "    [DEBUG] List workspaces response body: $list_body"

    if [ "$list_http_code" = "200" ]; then
        local existing_workspace
        existing_workspace=$(echo "$list_body" | grep -o "\"name\":\"$TEST_WORKSPACE\"" | head -1)
        if [ -n "$existing_workspace" ]; then
            echo "    [INFO] Workspace exists, regenerating user API key..."
            regenerate_response=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/workspaces/$TEST_WORKSPACE/regenerate-user-key" \
                -H "$ADMIN_API_AUTH_HEADER")

            local regen_http_code
            regen_http_code=$(echo "$regenerate_response" | tail -n1)
            local regen_body
            regen_body=$(echo "$regenerate_response" | sed '$d')

            echo "    [DEBUG] Regenerate response code: $regen_http_code"
            echo "    [DEBUG] Regenerate response body: $regen_body"

            if [ "$regen_http_code" = "200" ]; then
                API_KEY_SECRET=$(echo "$regen_body" | grep -o '"key_secret":"[^"]*"' | cut -d'"' -f4)
                API_KEY_ID=$(echo "$regen_body" | grep -o '"key_id":"[^"]*"' | cut -d'"' -f4)
                API_KEY_PREFIX=$(echo "$regen_body" | grep -o '"key_prefix":"[^"]*"' | cut -d'"' -f4)

                if [ -n "$API_KEY_SECRET" ] && [ -n "$API_KEY_ID" ]; then
                    API_KEY_ID="${API_KEY_PREFIX}_${API_KEY_ID}"
                    API_AUTH_HEADER="Authorization: ApiKey ${API_KEY_ID}:${API_KEY_SECRET}"
                    log_info "API key obtained successfully: ${API_KEY_ID:0:15}..."
                    return 0
                fi
            fi
        fi
    fi

    echo "    [ERROR] Failed to obtain API key"
    echo "    [ERROR] Response: $body"
    return 1
}

init_report

if ! check_service; then
    echo "Service is not available. Please start the service first."
    exit 1
fi

log_header "1. Health Check Interface Tests"

echo "Test 1.1: GET /health"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/health")
if [ "$HTTP_CODE" = "200" ]; then
    RESPONSE=$(curl -s "$BASE_URL/health")
    STATUS=$(echo "$RESPONSE" | grep -o '"status":"[^"]*"' | cut -d'"' -f4)
    ALGO=$(echo "$RESPONSE" | grep -o '"algorithm":"[^"]*"' | cut -d'"' -f4)
    echo "  [PASS] Status: $STATUS, Algorithm: $ALGO"
    log_result "Health Check" "PASS" "Status: $STATUS, Algorithm: $ALGO"
    record_test "PASS" "Health Check" "Status: $STATUS, Algorithm: $ALGO"
else
    echo "  [FAIL] HTTP $HTTP_CODE"
    log_result "Health Check" "FAIL" "HTTP $HTTP_CODE"
    record_test "FAIL" "Health Check" "HTTP $HTTP_CODE"
fi

echo "Test 1.2: GET /api/v1"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/api/v1")
if [ "$HTTP_CODE" = "200" ]; then
    echo "  [PASS] API info retrieved successfully"
    log_result "API Info" "PASS" "API info retrieved"
    record_test "PASS" "API Info" "API info retrieved"
else
    echo "  [FAIL] HTTP $HTTP_CODE"
    log_result "API Info" "FAIL" "HTTP $HTTP_CODE"
    record_test "FAIL" "API Info" "HTTP $HTTP_CODE"
fi

echo "Test 1.3: GET /metrics (port 8080)"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/metrics")
if [ "$HTTP_CODE" = "200" ]; then
    METRICS_DATA=$(curl -s "$BASE_URL/metrics" | head -10)
    echo "  [PASS] Metrics endpoint responding on port 8080"
    echo "         Sample: $(echo "$METRICS_DATA" | head -1)"
    log_result "Metrics (8080)" "PASS" "Metrics endpoint responding"
    record_test "PASS" "Metrics (8080)" "Metrics endpoint responding"
else
    echo "  [FAIL] HTTP $HTTP_CODE"
    log_result "Metrics (8080)" "FAIL" "HTTP $HTTP_CODE"
    record_test "FAIL" "Metrics (8080)" "HTTP $HTTP_CODE"
fi

echo "Test 1.4: Port 9091 (gRPC Server)"
GRPC_PORT_CHECK=$(ss -tlnp 2>/dev/null | grep ":9091" || netstat -tlnp 2>/dev/null | grep ":9091" || echo "")
if [ -n "$GRPC_PORT_CHECK" ]; then
    HTTP_CODE=$(curl -s --connect-timeout 2 --max-time 3 -o /dev/null -w "%{http_code}" "$METRICS_URL/" 2>/dev/null || echo "000")
    if [ "$HTTP_CODE" = "000" ]; then
        echo "  [PASS] Port 9091 is active (gRPC server, HTTP not expected)"
        log_result "gRPC Port (9091)" "PASS" "gRPC server listening on port 9091"
        record_test "PASS" "gRPC Port (9091)" "gRPC server listening on port 9091"
    elif [ "$HTTP_CODE" = "405" ]; then
        echo "  [PASS] Port 9091 is active (gRPC server, method not allowed)"
        log_result "gRPC Port (9091)" "PASS" "gRPC port active"
        record_test "PASS" "gRPC Port (9091)" "gRPC port active"
    else
        echo "  [INFO] Port 9091 responding with HTTP $HTTP_CODE"
        log_result "gRPC Port (9091)" "INFO" "HTTP $HTTP_CODE"
        record_test "PASS" "gRPC Port (9091)" "Port active (HTTP $HTTP_CODE)"
    fi
else
    echo "  [SKIP] Port 9091 not listening"
    log_result "gRPC Port (9091)" "SKIP" "Port not listening"
    record_test "SKIP" "gRPC Port (9091)" "Port not listening"
fi

log_header "2. API Key Setup"

if ! obtain_api_key; then
    echo ""
    echo "=============================================="
    echo "ERROR: Failed to obtain API key"
    echo "=============================================="
    echo "Your Admin API Key is invalid or missing."
    echo ""
    echo "Please set the following environment variables:"
    echo "  export ADMIN_API_KEY_ID=\"your-admin-key-id\""
    echo "  export ADMIN_API_KEY_SECRET=\"your-admin-key-secret\""
    echo ""
    echo "Then run the test again:"
    echo "  ./api_test.sh"
    echo "=============================================="
    exit 1
fi

echo "    [DEBUG] Verifying API key..."
# For user keys, test ID generation; for admin keys, test workspace listing
VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"verify\",\"biz_tag\":\"test\",\"algorithm\":\"snowflake\"}")

VERIFY_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
VERIFY_BODY=$(echo "$VERIFY_RESPONSE" | sed '$d')

if [ "$VERIFY_CODE" = "200" ]; then
    echo "    [PASS] API key is valid user key, can generate IDs"
elif echo "$VERIFY_BODY" | grep -q "Admin API key cannot generate IDs"; then
    echo "    [WARN] Using admin API key - cannot generate IDs, using admin-only tests"
    # Try to get a user key for ID generation tests
    echo "    [INFO] Attempting to obtain user key for ID generation tests..."
    if obtain_user_key_for_test; then
        echo "    [PASS] Obtained user key for ID generation tests"
    else
        echo "    [INFO] Will skip ID generation tests (admin key used)"
    fi
else
    echo "    [FAIL] API key verification failed (HTTP $VERIFY_CODE)"
    echo "    [FAIL] Response: $VERIFY_BODY"
    echo "    [FAIL] Aborting tests..."
    exit 1
fi

log_header "3. ID Generation Tests"

echo "Test 3.1: POST /api/v1/generate (Snowflake ID)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"order\",\"algorithm\":\"snowflake\"}")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')
if [ "$HTTP_CODE" = "200" ]; then
    ID=$(echo "$BODY" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    if [[ "$ID" =~ ^[0-9]+$ ]]; then
        echo "  [PASS] Snowflake ID generated: $ID"
        log_result "Snowflake ID Generation" "PASS" "ID: $ID"
        record_test "PASS" "Snowflake ID Generation" "ID: $ID"
    else
        echo "  [FAIL] Invalid ID format: $BODY"
        log_result "Snowflake ID Generation" "FAIL" "Invalid format: $BODY"
        record_test "FAIL" "Snowflake ID Generation" "Invalid format: $BODY"
    fi
else
    echo "  [FAIL] HTTP $HTTP_CODE: $BODY"
    log_result "Snowflake ID Generation" "FAIL" "HTTP $HTTP_CODE: $BODY"
    record_test "FAIL" "Snowflake ID Generation" "HTTP $HTTP_CODE: $BODY"
fi

echo "Test 3.2: POST /api/v1/generate (UUID v7)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"user\",\"algorithm\":\"uuid_v7\"}")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')
if [ "$HTTP_CODE" = "200" ]; then
    ID=$(echo "$BODY" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    if [[ "$ID" =~ ^\{?[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}\}?$ ]]; then
        echo "  [PASS] UUID v7 generated: $ID"
        log_result "UUID v7 Generation" "PASS" "ID: $ID"
        record_test "PASS" "UUID v7 Generation" "ID: $ID"
    else
        echo "  [FAIL] Invalid UUID format: $BODY"
        log_result "UUID v7 Generation" "FAIL" "Invalid format: $BODY"
        record_test "FAIL" "UUID v7 Generation" "Invalid format: $BODY"
    fi
else
    echo "  [FAIL] HTTP $HTTP_CODE: $BODY"
    log_result "UUID v7 Generation" "FAIL" "HTTP $HTTP_CODE: $BODY"
    record_test "FAIL" "UUID v7 Generation" "HTTP $HTTP_CODE: $BODY"
fi

echo "Test 3.3: POST /api/v1/generate (Segment)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"segment\",\"algorithm\":\"segment\"}")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')
if [ "$HTTP_CODE" = "200" ]; then
    ID=$(echo "$BODY" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    echo "  [PASS] Segment ID generated: $ID"
    log_result "Segment ID Generation" "PASS" "ID: $ID"
    record_test "PASS" "Segment ID Generation" "ID: $ID"
else
    echo "  [FAIL] HTTP $HTTP_CODE: $BODY"
    log_result "Segment ID Generation" "FAIL" "HTTP $HTTP_CODE: $BODY"
    record_test "FAIL" "Segment ID Generation" "HTTP $HTTP_CODE: $BODY"
fi

echo "Test 3.4: POST /api/v1/generate/batch (5 IDs)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate/batch" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"batch_test\",\"size\":$BATCH_SIZE_TEST,\"algorithm\":\"snowflake\"}")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')
if [ "$HTTP_CODE" = "200" ]; then
    IDS=$(echo "$BODY" | grep -o '"ids":\[[^]]*\]' || echo "")
    if [ -n "$IDS" ]; then
        echo "  [PASS] Batch generation successful"
        log_result "Batch ID Generation" "PASS" "Generated 5 IDs"
        record_test "PASS" "Batch ID Generation" "Generated 5 IDs"
    else
        echo "  [FAIL] Invalid response: $BODY"
        log_result "Batch ID Generation" "FAIL" "Invalid response: $BODY"
        record_test "FAIL" "Batch ID Generation" "Invalid response: $BODY"
    fi
else
    echo "  [FAIL] HTTP $HTTP_CODE: $BODY"
    log_result "Batch ID Generation" "FAIL" "HTTP $HTTP_CODE: $BODY"
    record_test "FAIL" "Batch ID Generation" "HTTP $HTTP_CODE: $BODY"
fi

echo "Test 3.5: POST /api/v1/generate (Default Algorithm)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"default\"}")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')
if [ "$HTTP_CODE" = "200" ]; then
    ID=$(echo "$BODY" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    echo "  [PASS] Default algorithm generated: $ID"
    log_result "Default Algorithm Generation" "PASS" "ID: $ID"
    record_test "PASS" "Default Algorithm Generation" "ID: $ID"
else
    echo "  [FAIL] HTTP $HTTP_CODE: $BODY"
    log_result "Default Algorithm Generation" "FAIL" "HTTP $HTTP_CODE: $BODY"
    record_test "FAIL" "Default Algorithm Generation" "HTTP $HTTP_CODE: $BODY"
fi

log_header "4. ID Parse Tests"

echo "Test 4.1: POST /api/v1/parse (Snowflake ID)"
SF_RESPONSE=$(curl -s -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"parse_test\",\"algorithm\":\"snowflake\"}")
SF_ID=$(echo "$SF_RESPONSE" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
if [ -n "$SF_ID" ] && [[ "$SF_ID" =~ ^[0-9]+$ ]]; then
    RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/parse" \
        -H "Content-Type: application/json" \
        -H "$API_AUTH_HEADER" \
        -d "{\"id\": \"$SF_ID\", \"workspace\": \"$TEST_WORKSPACE\", \"group\": \"$TEST_GROUP\", \"biz_tag\": \"parse_test\", \"algorithm\": \"snowflake\"}")
    HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
    BODY=$(echo "$RESPONSE" | sed '$d')
    if [ "$HTTP_CODE" = "200" ]; then
        echo "  [PASS] Parse successful: $BODY"
        log_result "Snowflake ID Parse" "PASS" "$BODY"
        record_test "PASS" "Snowflake ID Parse" "$BODY"
    else
        echo "  [FAIL] HTTP $HTTP_CODE: $BODY"
        log_result "Snowflake ID Parse" "FAIL" "HTTP $HTTP_CODE: $BODY"
        record_test "FAIL" "Snowflake ID Parse" "HTTP $HTTP_CODE: $BODY"
    fi
else
    echo "  [SKIP] Cannot generate Snowflake ID for test"
    log_result "Snowflake ID Parse" "SKIP" "Cannot generate test ID"
    record_test "SKIP" "Snowflake ID Parse" "Cannot generate test ID"
fi

echo "Test 4.2: POST /api/v1/parse (UUID v7)"
UUID_RESPONSE=$(curl -s -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"parse_test\",\"algorithm\":\"uuid_v7\"}")
UUID_ID=$(echo "$UUID_RESPONSE" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
if [ -n "$UUID_ID" ]; then
    RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/parse" \
        -H "Content-Type: application/json" \
        -H "$API_AUTH_HEADER" \
        -d "{\"id\": \"$UUID_ID\", \"workspace\": \"$TEST_WORKSPACE\", \"group\": \"$TEST_GROUP\", \"biz_tag\": \"parse_test\", \"algorithm\": \"uuid_v7\"}")
    HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
    BODY=$(echo "$RESPONSE" | sed '$d')
    if [ "$HTTP_CODE" = "200" ]; then
        echo "  [PASS] Parse successful: $BODY"
        log_result "UUID v7 Parse" "PASS" "$BODY"
        record_test "PASS" "UUID v7 Parse" "$BODY"
    else
        echo "  [FAIL] HTTP $HTTP_CODE: $BODY"
        log_result "UUID v7 Parse" "FAIL" "HTTP $HTTP_CODE: $BODY"
        record_test "FAIL" "UUID v7 Parse" "HTTP $HTTP_CODE: $BODY"
    fi
else
    echo "  [SKIP] Cannot generate UUID for test"
    log_result "UUID v7 Parse" "SKIP" "Cannot generate test ID"
    record_test "SKIP" "UUID v7 Parse" "Cannot generate test ID"
fi

log_header "5. Parameter Validation Tests"

echo "Test 5.1: POST /api/v1/generate (empty workspace)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"test\",\"algorithm\":\"snowflake\"}")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
if [ "$HTTP_CODE" = "422" ] || [ "$HTTP_CODE" = "400" ]; then
    echo "  [PASS] Correctly rejected empty workspace (HTTP $HTTP_CODE)"
    log_result "Empty Workspace Validation" "PASS" "HTTP $HTTP_CODE"
    record_test "PASS" "Empty Workspace Validation" "HTTP $HTTP_CODE"
else
    echo "  [FAIL] Expected 422/400, got HTTP $HTTP_CODE"
    log_result "Empty Workspace Validation" "FAIL" "Expected 422/400, got HTTP $HTTP_CODE"
    record_test "FAIL" "Empty Workspace Validation" "Expected 422/400, got HTTP $HTTP_CODE"
fi

echo "Test 5.2: POST /api/v1/generate (empty group)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"\",\"biz_tag\":\"test\",\"algorithm\":\"snowflake\"}")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
if [ "$HTTP_CODE" = "422" ] || [ "$HTTP_CODE" = "400" ]; then
    echo "  [PASS] Correctly rejected empty group (HTTP $HTTP_CODE)"
    log_result "Empty Group Validation" "PASS" "HTTP $HTTP_CODE"
    record_test "PASS" "Empty Group Validation" "HTTP $HTTP_CODE"
else
    echo "  [FAIL] Expected 422/400, got HTTP $HTTP_CODE"
    log_result "Empty Group Validation" "FAIL" "Expected 422/400, got HTTP $HTTP_CODE"
    record_test "FAIL" "Empty Group Validation" "Expected 422/400, got HTTP $HTTP_CODE"
fi

echo "Test 5.3: POST /api/v1/generate (invalid JSON)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d '{"invalid_json": }')
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
if [ "$HTTP_CODE" = "400" ] || [ "$HTTP_CODE" = "415" ]; then
    echo "  [PASS] Correctly rejected invalid JSON (HTTP $HTTP_CODE)"
    log_result "Invalid JSON Handling" "PASS" "HTTP $HTTP_CODE"
    record_test "PASS" "Invalid JSON Handling" "HTTP $HTTP_CODE"
else
    echo "  [FAIL] Expected 400/415, got HTTP $HTTP_CODE"
    log_result "Invalid JSON Handling" "FAIL" "Expected 400/415, got HTTP $HTTP_CODE"
    record_test "FAIL" "Invalid JSON Handling" "Expected 400/415, got HTTP $HTTP_CODE"
fi

echo "Test 5.4: POST /api/v1/generate (invalid algorithm)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"test\",\"algorithm\":\"invalid_algorithm\"}")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
if [ "$HTTP_CODE" = "400" ] || [ "$HTTP_CODE" = "500" ]; then
    echo "  [PASS] Correctly rejected invalid algorithm (HTTP $HTTP_CODE)"
    log_result "Invalid Algorithm Handling" "PASS" "HTTP $HTTP_CODE"
    record_test "PASS" "Invalid Algorithm Handling" "HTTP $HTTP_CODE"
else
    echo "  [FAIL] Expected 400/500, got HTTP $HTTP_CODE"
    log_result "Invalid Algorithm Handling" "FAIL" "Expected 400/500, got HTTP $HTTP_CODE"
    record_test "FAIL" "Invalid Algorithm Handling" "Expected 400/500, got HTTP $HTTP_CODE"
fi

log_header "6. Configuration Management Tests"

echo "Test 6.1: GET /api/v1/config"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/api/v1/config" \
    -H "$API_AUTH_HEADER")
if [ "$HTTP_CODE" = "200" ]; then
    echo "  [PASS] Configuration retrieved successfully"
    log_result "Get Configuration" "PASS" "Configuration retrieved"
    record_test "PASS" "Get Configuration" "Configuration retrieved"
else
    echo "  [FAIL] HTTP $HTTP_CODE"
    log_result "Get Configuration" "FAIL" "HTTP $HTTP_CODE"
    record_test "FAIL" "Get Configuration" "HTTP $HTTP_CODE"
fi

echo "Test 6.2: POST /api/v1/config/rate-limit"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/config/rate-limit" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d '{"default_rps": 200, "burst_size": 50}')
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
if [ "$HTTP_CODE" = "200" ]; then
    echo "  [PASS] Rate limit configuration updated"
    log_result "Rate Limit Update" "PASS" "Rate limit updated"
    record_test "PASS" "Rate Limit Update" "Rate limit updated"
else
    echo "  [FAIL] HTTP $HTTP_CODE"
    log_result "Rate Limit Update" "FAIL" "HTTP $HTTP_CODE"
    record_test "FAIL" "Rate Limit Update" "HTTP $HTTP_CODE"
fi

echo "Test 6.3: POST /api/v1/config/logging"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/config/logging" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d '{"level": "debug", "format": "json"}')
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
if [ "$HTTP_CODE" = "200" ]; then
    echo "  [PASS] Logging configuration updated"
    log_result "Logging Update" "PASS" "Logging configuration updated"
    record_test "PASS" "Logging Update" "Logging configuration updated"
else
    echo "  [FAIL] HTTP $HTTP_CODE"
    log_result "Logging Update" "FAIL" "HTTP $HTTP_CODE"
    record_test "FAIL" "Logging Update" "HTTP $HTTP_CODE"
fi

echo "Test 6.4: POST /api/v1/config/algorithm"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/config/algorithm" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d '{"biz_tag": "order", "algorithm": "snowflake"}')
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')
if [ "$HTTP_CODE" = "200" ]; then
    echo "  [PASS] Algorithm setting successful"
    log_result "Algorithm Setting" "PASS" "$BODY"
    record_test "PASS" "Algorithm Setting" "$BODY"
else
    echo "  [FAIL] HTTP $HTTP_CODE: $BODY"
    log_result "Algorithm Setting" "FAIL" "HTTP $HTTP_CODE: $BODY"
    record_test "FAIL" "Algorithm Setting" "HTTP $HTTP_CODE: $BODY"
fi

log_header "7. Error Handling Tests"

echo "Test 7.1: GET /nonexistent (404)"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/nonexistent" \
    -H "$API_AUTH_HEADER")
if [ "$HTTP_CODE" = "404" ]; then
    echo "  [PASS] Correctly returned 404 Not Found"
    log_result "404 Handling" "PASS" "HTTP 404"
    record_test "PASS" "404 Handling" "HTTP 404"
else
    echo "  [FAIL] Expected 404, got HTTP $HTTP_CODE"
    log_result "404 Handling" "FAIL" "Expected 404, got HTTP $HTTP_CODE"
    record_test "FAIL" "404 Handling" "Expected 404, got HTTP $HTTP_CODE"
fi

echo "Test 7.2: GET /api/v1/generate (405)"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/api/v1/generate")
if [ "$HTTP_CODE" = "405" ]; then
    echo "  [PASS] Correctly returned 405 Method Not Allowed"
    log_result "405 Handling" "PASS" "HTTP 405"
    record_test "PASS" "405 Handling" "HTTP 405"
else
    echo "  [FAIL] Expected 405, got HTTP $HTTP_CODE"
    log_result "405 Handling" "FAIL" "Expected 405, got HTTP $HTTP_CODE"
    record_test "FAIL" "405 Handling" "Expected 405, got HTTP $HTTP_CODE"
fi

echo "Test 7.3: POST with wrong content type (415)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: text/plain" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"$TEST_GROUP\",\"biz_tag\":\"test\"}")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
if [ "$HTTP_CODE" = "415" ] || [ "$HTTP_CODE" = "400" ]; then
    echo "  [PASS] Correctly rejected invalid content type (HTTP $HTTP_CODE)"
    log_result "Content Type Handling" "PASS" "HTTP $HTTP_CODE"
    record_test "PASS" "Content Type Handling" "HTTP $HTTP_CODE"
else
    echo "  [FAIL] Expected 415/400, got HTTP $HTTP_CODE"
    log_result "Content Type Handling" "FAIL" "Expected 415/400, got HTTP $HTTP_CODE"
    record_test "FAIL" "Content Type Handling" "Expected 415/400, got HTTP $HTTP_CODE"
fi

log_header "8. Performance Tests"

echo "Test 8.1: Sequential generation of $PERF_TEST_ITERATIONS IDs"
START_TIME=$(date +%s%N)
SUCCESS_COUNT=0
for i in $(seq 1 $PERF_TEST_ITERATIONS); do
    RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
        -H "Content-Type: application/json" \
        -H "$API_AUTH_HEADER" \
        -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"perf\",\"biz_tag\":\"perf_test\",\"algorithm\":\"snowflake\"}")
    HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
    if [ "$HTTP_CODE" = "200" ]; then
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    fi
done
END_TIME=$(date +%s%N)
DURATION=$((($END_TIME - $START_TIME) / 1000000))
if [ $SUCCESS_COUNT -eq $PERF_TEST_ITERATIONS ]; then
    TPS=$((PERF_TEST_ITERATIONS * 1000 / DURATION))
    echo "  [PASS] $SUCCESS_COUNT/$PERF_TEST_ITERATIONS successful, duration: ${DURATION}ms, TPS: $TPS"
    log_result "Sequential Performance" "PASS" "$SUCCESS_COUNT/$PERF_TEST_ITERATIONS, ${DURATION}ms, TPS: $TPS"
    record_test "PASS" "Sequential Performance" "$SUCCESS_COUNT/$PERF_TEST_ITERATIONS, ${DURATION}ms, TPS: $TPS"
else
    echo "  [FAIL] Only $SUCCESS_COUNT/$PERF_TEST_ITERATIONS successful"
    log_result "Sequential Performance" "FAIL" "$SUCCESS_COUNT/$PERF_TEST_ITERATIONS"
    record_test "FAIL" "Sequential Performance" "$SUCCESS_COUNT/$PERF_TEST_ITERATIONS"
fi

echo "Test 8.2: Concurrent generation ($PERF_CONCURRENT_BATCHES batches x $PERF_CONCURRENT_SIZE requests)"
START_TIME=$(date +%s%N)

# Industrial-grade concurrent solution: batch aggregation + independent file counting
# Using brace expansion to avoid C-style loop subshell compatibility issues
SUCCESS_COUNT=0
BATCH_SIZE=$PERF_CONCURRENT_SIZE
BATCHES=$PERF_CONCURRENT_BATCHES
TMPDIR="${TMPDIR:-/tmp}"

for batch in $(seq 1 $BATCHES); do
    (
        local_success=0
        for i in $(seq 1 $BATCH_SIZE); do
            HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
                -H "Content-Type: application/json" \
                -H "$API_AUTH_HEADER" \
                -H "X-Request-ID: batch${batch}-req${i}" \
                -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"concurrent\",\"biz_tag\":\"concurrent_test\",\"algorithm\":\"snowflake\"}")
            if [ "$HTTP_CODE" = "200" ]; then
                local_success=$((local_success + 1))
            fi
        done
        echo "$local_success" > "$TMPDIR/nebula_batch_${batch}_$$.tmp"
    ) &
done

wait

for batch in $(seq 1 $BATCHES); do
    if [ -f "$TMPDIR/nebula_batch_${batch}_$$.tmp" ]; then
        batch_count=$(cat "$TMPDIR/nebula_batch_${batch}_$$.tmp")
        SUCCESS_COUNT=$((SUCCESS_COUNT + batch_count))
        rm -f "$TMPDIR/nebula_batch_${batch}_$$.tmp"
    fi
done

END_TIME=$(date +%s%N)
DURATION=$((($END_TIME - $START_TIME) / 1000000))
TOTAL_REQUESTS=$((BATCH_SIZE * BATCHES))
echo "  [INFO] $SUCCESS_COUNT/$TOTAL_REQUESTS successful, duration: ${DURATION}ms"
if [ $SUCCESS_COUNT -gt 0 ]; then
    log_result "Concurrent Performance" "PASS" "$SUCCESS_COUNT/$TOTAL_REQUESTS, ${DURATION}ms"
    record_test "PASS" "Concurrent Performance" "$SUCCESS_COUNT/$TOTAL_REQUESTS, ${DURATION}ms"
else
    log_result "Concurrent Performance" "FAIL" "$SUCCESS_COUNT/$TOTAL_REQUESTS"
    record_test "FAIL" "Concurrent Performance" "$SUCCESS_COUNT/$TOTAL_REQUESTS"
fi

log_header "9. Snowflake ID Correctness Tests"

echo "Test 9.1: Snowflake ID timestamp verification"
SF_RESPONSE=$(curl -s -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -H "$API_AUTH_HEADER" \
    -d "{\"workspace\":\"$TEST_WORKSPACE\",\"group\":\"verify\",\"biz_tag\":\"verify_test\",\"algorithm\":\"snowflake\"}")
SF_ID=$(echo "$SF_RESPONSE" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
if [ -n "$SF_ID" ] && [[ "$SF_ID" =~ ^[0-9]+$ ]]; then
    DECIMAL_ID=$(echo "$SF_ID" | sed 's/^0*//')
    if [ -z "$DECIMAL_ID" ]; then
        DECIMAL_ID="0"
    fi
    
    TIMESTAMP_BITS=$((DECIMAL_ID >> 21))
    # Snowflake uses milliseconds since epoch (1704067200000 = Jan 1, 2025 00:00:00 UTC)
    CURRENT_TIMESTAMP_MS=$(( $(date +%s) * 1000 - 1704067200000 ))
    
    DIFF=$((TIMESTAMP_BITS - CURRENT_TIMESTAMP_MS))
    if [ $DIFF -lt 0 ]; then
        DIFF=$((-DIFF))
    fi
    
    # Allow up to 5 seconds (5000ms) difference for test execution time
    if [ $DIFF -lt 5000 ]; then
        echo "  [PASS] Timestamp verification successful (difference: ${DIFF}s)"
        log_result "Timestamp Verification" "PASS" "Difference: ${DIFF}s"
        record_test "PASS" "Timestamp Verification" "Difference: ${DIFF}s"
    else
        echo "  [FAIL] Timestamp difference too large: ${DIFF}s"
        log_result "Timestamp Verification" "FAIL" "Difference: ${DIFF}s"
        record_test "FAIL" "Timestamp Verification" "Difference: ${DIFF}s"
    fi
else
    echo "  [SKIP] Cannot verify - invalid Snowflake ID"
    log_result "Timestamp Verification" "SKIP" "Invalid Snowflake ID"
    record_test "SKIP" "Timestamp Verification" "Invalid Snowflake ID"
fi

log_header "Test Summary"
echo ""
echo "Total Tests: $TOTAL_TESTS"
echo -e "Passed: ${GREEN}$PASSED_TESTS${NC}"
echo -e "Failed: ${RED}$FAILED_TESTS${NC}"
echo -e "Skipped: ${YELLOW}$SKIPPED_TESTS${NC}"
PASS_RATE=$((PASSED_TESTS * 100 / TOTAL_TESTS))
echo -e "Pass Rate: ${BLUE}${PASS_RATE}%${NC}"
echo ""
echo "Test report saved to: $REPORT_FILE"
echo "========================================"

echo "" >> "$REPORT_FILE"
echo "========================================" >> "$REPORT_FILE"
echo "Summary" >> "$REPORT_FILE"
echo "Total Tests: $TOTAL_TESTS" >> "$REPORT_FILE"
echo "Passed: $PASSED_TESTS" >> "$REPORT_FILE"
echo "Failed: $FAILED_TESTS" >> "$REPORT_FILE"
echo "Skipped: $SKIPPED_TESTS" >> "$REPORT_FILE"
echo "Pass Rate: ${PASS_RATE}%" >> "$REPORT_FILE"
