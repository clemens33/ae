#!/opt/homebrew/bin/bash
D="$(dirname "$0")"
for a in arm1 arm2 arm3; do
    echo "=== START $a $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
    "$D/$a.sh"
    echo "=== END $a rc=$? $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
    pkill -x aefake 2>/dev/null
    sleep 2
done
echo "ALL ARMS DONE"
