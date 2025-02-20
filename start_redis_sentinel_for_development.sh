#!/bin/bash

# Kill any existing Redis processes
redis-cli -p 6380 shutdown || true
redis-cli -p 6381 shutdown || true
redis-cli -p 6382 shutdown || true
redis-cli -p 26380 shutdown || true


# Clean up sentinel state
echo "Cleaning up sentinel state..."
cat > sentinel.conf << EOF
port 26380

# Monitor the master (name: mymaster) with 1 vote required for failover
sentinel monitor mymaster 127.0.0.1 6380 1

# If master is unresponsive for 2 seconds, consider it down
sentinel down-after-milliseconds mymaster 2000

# Failover timeout: 10 seconds
sentinel failover-timeout mymaster 10000

# Allow one replica to be promoted at a time
sentinel parallel-syncs mymaster 1

# Disable protected mode
protected-mode no
EOF

echo "Starting Redis master..."
redis-server --port 6380 \
    --appendonly yes \
    --appendfilename "appendonly-6380.aof" \
    --dbfilename dump-6380.rdb \
    --dir ./ &

# Wait for master to start
sleep 2

echo "Starting Redis replicas..."
redis-server --port 6381 \
    --replicaof 127.0.0.1 6380 \
    --appendonly yes \
    --appendfilename "appendonly-6381.aof" \
    --dbfilename dump-6381.rdb \
    --dir ./ &

redis-server --port 6382 \
    --replicaof 127.0.0.1 6380 \
    --appendonly yes \
    --appendfilename "appendonly-6382.aof" \
    --dbfilename dump-6382.rdb \
    --dir ./ &

# Wait for replicas to start
sleep 2

echo "Starting Redis Sentinel..."
redis-sentinel sentinel.conf &

echo "All processes started. Use these commands to check status:"
echo "redis-cli -p 26380 SENTINEL get-master-addr-by-name mymaster"
echo "redis-cli -p 6380 INFO replication"

# Optional: Show running Redis processes
#ps aux | grep redis 