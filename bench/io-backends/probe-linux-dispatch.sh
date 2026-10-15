#!/usr/bin/env bash
# Is Tokio's blocking pool creating a thread per call on this box, or is the
# per-call cost a pure handoff? Sample the process's thread count during a
# dispatch run, and price raw thread creation for comparison.
cd "$HOME/io-backends" || exit 1

./target/release/io-backends run dispatch --root "$HOME/dae-io-bench" \
  --blob-mib 4096 --big-mib 4096 --threads 1 --passes 1 --warmup 0 \
  --backend tokio --json > /tmp/tc.log 2>&1 &
pid=$!
max=0
for _ in $(seq 1 60); do
  n=$(awk '/^Threads/{print $2}' "/proc/$pid/status" 2>/dev/null)
  [ -z "$n" ] && break
  [ "$n" -gt "$max" ] && max=$n
  sleep 0.2
done
wait "$pid"
echo "tokio dispatch: peak threads = $max"
tail -2 /tmp/tc.log

python3 - <<'EOF'
import threading, time
t = time.perf_counter()
for _ in range(3000):
    threading.Thread(target=lambda: None).start()
ms = (time.perf_counter() - t) * 1000
print(f"3000 thread starts: {ms:.1f} ms -> {ms / 3000 * 1000:.1f} us per thread")
EOF
