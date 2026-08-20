#!/opt/homebrew/bin/bash
D="$(dirname "$0")"
chmod -R u+w /tmp/aecx/templates 2>/dev/null; rm -rf /tmp/aecx/templates; mkdir -p /tmp/aecx/templates
for s in tpl-stage1 tpl-stage2 tpl-stage3 tpl-stage4 tpl-a1 tpl-a1c tpl-a1e; do
    echo "=== $s $(date -u +%H:%M:%S) ==="
    "$D/$s.sh" >"$D/$s.rebuild.log" 2>&1
    echo "  rc=$? $(grep -c 'fingerprint' "$D/$s.rebuild.log" 2>/dev/null) fingerprint lines"
done
echo "REBUILD DONE"
