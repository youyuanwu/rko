#!/bin/sh
# Pre-insmod: bring up loopback for networking tests.
# Called with "pre" arg before insmod, no arg after insmod.
if [ "$1" = "pre" ]; then
    ifconfig lo 127.0.0.1 up
fi
