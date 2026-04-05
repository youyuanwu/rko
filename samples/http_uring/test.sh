#!/bin/sh
# Test the http_uring module with the userspace io_uring test binary.

# Handle pre-insmod phase (called with "pre" argument)
if [ "$1" = "pre" ]; then
    # Bring up loopback for TCP connections
    ifconfig lo 127.0.0.1 up
    echo "http_uring: loopback configured"
    exit 0
fi

# Post-insmod: run the test binary
echo "http_uring: checking loopback"
ifconfig lo 2>/dev/null || echo "http_uring: lo not found"
sleep 1
http_uring_test
