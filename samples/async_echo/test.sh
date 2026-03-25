#!/bin/sh
# Test the async TCP echo server by connecting with nc and verifying echo.

# Bring up loopback so we can connect to 127.0.0.1
ifconfig lo 127.0.0.1 up

# Give the accept loop a moment to start
sleep 1

# Send "hello" and read back the echo.
RESULT=$(printf 'hello' | nc -w 2 127.0.0.1 8080)

if [ "$RESULT" = "hello" ]; then
    echo "async_echo: echo test passed"
else
    echo "async_echo: echo test FAILED (got '$RESULT')"
fi
