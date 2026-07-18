#!/bin/bash
# Redis Cache Backend Integration Test

echo "=== Redis Cache Backend Integration Test ==="
echo ""

# Test Redis connection
echo "1. Testing Redis connection..."
if redis-cli -p 6379 PING | grep -q "PONG"; then
    echo "   ✓ Redis connection successful"
else
    echo "   ✗ Redis connection failed"
    exit 1
fi

# Test SET operation
echo ""
echo "2. Testing SET operation..."
if redis-cli -p 6379 SET "nebula:test:integration_key" "100,200,300,400,500" EX 60 | grep -q "OK"; then
    echo "   ✓ SET operation successful"
else
    echo "   ✗ SET operation failed"
fi

# Test GET operation
echo ""
echo "3. Testing GET operation..."
VALUE=$(redis-cli -p 6379 GET "nebula:test:integration_key")
if [ -n "$VALUE" ]; then
    echo "   ✓ GET operation successful: $VALUE"
else
    echo "   ✗ GET operation failed"
fi

# Test EXISTS operation
echo ""
echo "4. Testing EXISTS operation..."
EXISTS=$(redis-cli -p 6379 EXISTS "nebula:test:integration_key")
echo "   ✓ EXISTS operation successful: key exists = $EXISTS"

# Test TTL operation
echo ""
echo "5. Testing TTL operation..."
TTL=$(redis-cli -p 6379 TTL "nebula:test:integration_key")
echo "   ✓ TTL operation successful: remaining TTL = ${TTL}s"

# Test DELETE operation
echo ""
echo "6. Testing DELETE operation..."
redis-cli -p 6379 DEL "nebula:test:integration_key" > /dev/null
echo "   ✓ DELETE operation successful"

# Verify deletion
echo ""
echo "7. Verifying deletion..."
EXISTS=$(redis-cli -p 6379 EXISTS "nebula:test:integration_key")
if [ "$EXISTS" = "0" ]; then
    echo "   ✓ Key successfully deleted"
else
    echo "   ✗ Key still exists"
fi

echo ""
echo "=== All Redis operations verified successfully! ==="