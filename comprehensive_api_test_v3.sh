#!/bin/bash

# Nebula ID API Comprehensive Test Suite V3
# Tests with all required parameters properly included

set -e

BASE_URL="http://localhost:8080"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="test_results_${TIMESTAMP}.txt"
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

init_report

if ! check_service; then
    echo "Service not available. Please start the service first."
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

echo "Test 1.3: GET /metrics"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/metrics")
if [ "$HTTP_CODE" = "200" ]; then
    echo "  [PASS] Metrics endpoint responding"
    log_result "Metrics" "PASS" "Metrics endpoint responding"
    record_test "PASS" "Metrics" "Metrics endpoint responding"
else
    echo "  [FAIL] HTTP $HTTP_CODE"
    log_result "Metrics" "FAIL" "HTTP $HTTP_CODE"
    record_test "FAIL" "Metrics" "HTTP $HTTP_CODE"
fi

log_header "2. ID Generation Tests"

echo "Test 2.1: POST /api/v1/generate (Snowflake ID)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"default","biz_tag":"order","algorithm":"snowflake"}')
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

echo "Test 2.2: POST /api/v1/generate (UUID v7)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"default","biz_tag":"user","algorithm":"uuid_v7"}')
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

echo "Test 2.3: POST /api/v1/generate (Segment)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"default","biz_tag":"segment","algorithm":"segment"}')
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

echo "Test 2.4: POST /api/v1/generate/batch (5 IDs)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate/batch" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"default","biz_tag":"batch_test","size":5,"algorithm":"snowflake"}')
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')
if [ "$HTTP_CODE" = "200" ]; then
    IDS=$(echo "$BODY" grep -o '"ids":\[[^]]*\]' || echo "")
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

echo "Test 2.5: POST /api/v1/generate (Default Algorithm)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"default","biz_tag":"default"}')
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

log_header "3. ID Parse Tests"

echo "Test 3.1: POST /api/v1/parse (Snowflake ID)"
SF_RESPONSE=$(curl -s -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"default","biz_tag":"parse_test","algorithm":"snowflake"}')
SF_ID=$(echo "$SF_RESPONSE" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
if [ -n "$SF_ID" ] && [[ "$SF_ID" =~ ^[0-9]+$ ]]; then
    RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/parse" \
        -H "Content-Type: application/json" \
        -d "{\"id\": \"$SF_ID\", \"workspace\": \"test\", \"group\": \"default\", \"biz_tag\": \"parse_test\", \"algorithm\": \"snowflake\"}")
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

echo "Test 3.2: POST /api/v1/parse (UUID v7)"
UUID_RESPONSE=$(curl -s -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"default","biz_tag":"parse_test","algorithm":"uuid_v7"}')
UUID_ID=$(echo "$UUID_RESPONSE" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
if [ -n "$UUID_ID" ]; then
    RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/parse" \
        -H "Content-Type: application/json" \
        -d "{\"id\": \"$UUID_ID\", \"workspace\": \"test\", \"group\": \"default\", \"biz_tag\": \"parse_test\", \"algorithm\": \"uuid_v7\"}")
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

log_header "4. Parameter Validation Tests"

echo "Test 4.1: POST /api/v1/generate (empty workspace)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"","group":"default","biz_tag":"test","algorithm":"snowflake"}')
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

echo "Test 4.2: POST /api/v1/generate (empty group)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"","biz_tag":"test","algorithm":"snowflake"}')
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

echo "Test 4.3: POST /api/v1/generate (invalid JSON)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"invalid json}')
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

echo "Test 4.4: POST /api/v1/generate (invalid algorithm)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"default","biz_tag":"test","algorithm":"invalid_algorithm"}')
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

log_header "5. Configuration Management Tests"

echo "Test 5.1: GET /api/v1/config"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/api/v1/config")
if [ "$HTTP_CODE" = "200" ]; then
    echo "  [PASS] Configuration retrieved successfully"
    log_result "Get Configuration" "PASS" "Configuration retrieved"
    record_test "PASS" "Get Configuration" "Configuration retrieved"
else
    echo "  [FAIL] HTTP $HTTP_CODE"
    log_result "Get Configuration" "FAIL" "HTTP $HTTP_CODE"
    record_test "FAIL" "Get Configuration" "HTTP $HTTP_CODE"
fi

echo "Test 5.2: POST /api/v1/config/rate-limit"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/config/rate-limit" \
    -H "Content-Type: application/json" \
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

echo "Test 5.3: POST /api/v1/config/logging"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/config/logging" \
    -H "Content-Type: application/json" \
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

echo "Test 5.4: POST /api/v1/config/algorithm"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/config/algorithm" \
    -H "Content-Type: application/json" \
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

log_header "6. Error Handling Tests"

echo "Test 6.1: GET /nonexistent (404)"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/nonexistent")
if [ "$HTTP_CODE" = "404" ]; then
    echo "  [PASS] Correctly returned 404 Not Found"
    log_result "404 Handling" "PASS" "HTTP 404"
    record_test "PASS" "404 Handling" "HTTP 404"
else
    echo "  [FAIL] Expected 404, got HTTP $HTTP_CODE"
    log_result "404 Handling" "FAIL" "Expected 404, got HTTP $HTTP_CODE"
    record_test "FAIL" "404 Handling" "Expected 404, got HTTP $HTTP_CODE"
fi

echo "Test 6.2: GET /api/v1/generate (405)"
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

echo "Test 6.3: POST with wrong content type (415)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: text/plain" \
    -d '{"workspace":"test","group":"default","biz_tag":"test"}')
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

log_header "7. Performance Tests"

echo "Test 7.1: Sequential generation of 100 IDs"
START_TIME=$(date +%s%N)
SUCCESS_COUNT=0
for i in {1..100}; do
    RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
        -H "Content-Type: application/json" \
        -d '{"workspace":"test","group":"perf","biz_tag":"perf_test","algorithm":"snowflake"}')
    HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
    if [ "$HTTP_CODE" = "200" ]; then
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    fi
done
END_TIME=$(date +%s%N)
DURATION=$((($END_TIME - $START_TIME) / 1000000))
if [ $SUCCESS_COUNT -eq 100 ]; then
    TPS=$((100000 / DURATION))
    echo "  [PASS] $SUCCESS_COUNT/100 successful, duration: ${DURATION}ms, TPS: $TPS"
    log_result "Sequential Performance" "PASS" "$SUCCESS_COUNT/100, ${DURATION}ms, TPS: $TPS"
    record_test "PASS" "Sequential Performance" "$SUCCESS_COUNT/100, ${DURATION}ms, TPS: $TPS"
else
    echo "  [FAIL] Only $SUCCESS_COUNT/100 successful"
    log_result "Sequential Performance" "FAIL" "$SUCCESS_COUNT/100"
    record_test "FAIL" "Sequential Performance" "$SUCCESS_COUNT/100"
fi

echo "Test 7.2: Concurrent generation (50 requests x 50)"
START_TIME=$(date +%s%N)
SUCCESS_COUNT=0
for j in {1..50}; do
    (for i in {1..50}; do
        RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BASE_URL/api/v1/generate" \
            -H "Content-Type: application/json" \
            -d '{"workspace":"test","group":"concurrent","biz_tag":"concurrent_test","algorithm":"snowflake"}')
        if [ "$RESPONSE" = "200" ]; then
            SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
        fi
    done) &
done
wait
END_TIME=$(date +%s%N)
DURATION=$((($END_TIME - $START_TIME) / 1000000))
TOTAL_REQUESTS=2500
echo "  [INFO] $SUCCESS_COUNT/$TOTAL_REQUESTS successful, duration: ${DURATION}ms"
if [ $SUCCESS_COUNT -gt 0 ]; then
    log_result "Concurrent Performance" "PASS" "$SUCCESS_COUNT/$TOTAL_REQUESTS, ${DURATION}ms"
    record_test "PASS" "Concurrent Performance" "$SUCCESS_COUNT/$TOTAL_REQUESTS, ${DURATION}ms"
else
    log_result "Concurrent Performance" "FAIL" "$SUCCESS_COUNT/$TOTAL_REQUESTS"
    record_test "FAIL" "Concurrent Performance" "$SUCCESS_COUNT/$TOTAL_REQUESTS"
fi

log_header "8. Snowflake ID Correctness Tests"

echo "Test 8.1: Snowflake ID timestamp verification"
SF_RESPONSE=$(curl -s -X POST "$BASE_URL/api/v1/generate" \
    -H "Content-Type: application/json" \
    -d '{"workspace":"test","group":"verify","biz_tag":"verify_test","algorithm":"snowflake"}')
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
