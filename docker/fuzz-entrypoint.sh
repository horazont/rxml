#!/bin/sh
cd /app/fuzz
mkdir -p out
chown -R fuzzer:fuzzer out

exec setpriv --reuid fuzzer --regid fuzzer --init-groups --reset-env -- sh -c 'export PATH="/home/fuzzer/.cargo/bin:$PATH" AFL_SKIP_CPUFREQ=1; exec cargo afl fuzz -i in -o out "$@" -- target/debug/fuzz' -- "$@"
