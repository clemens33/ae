#!/opt/homebrew/bin/bash
set -euo pipefail
SB=/tmp/aeb0
{
  echo "# B0 Design 1 environment / tool record"
  echo "captured_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uname=$(uname -a)"
  echo "sw_vers=$(sw_vers -productVersion 2>/dev/null || echo na)"
  echo "frozen_commit=72c729343a0117af2968b66e1c43f89ad25fc0b2"
  echo "frozen_ae_sha256=$(cat "$SB/frozen/ae.sha256")"
  echo "instrumented_ae_sha256=$(cat "$SB/instr/ae.sha256")"
  echo "h507_patch_sha256=$(cat "$SB/instr/h507.patch.sha256")"
  echo "template_fingerprint_sha256=$(cat "$SB/template/fingerprint.sha256")"
  echo "run_shell=/opt/homebrew/bin/bash"
  echo "arm_PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
  for t in /opt/homebrew/bin/bash /bin/bash /opt/homebrew/bin/tmux /usr/bin/git /bin/date /usr/bin/awk /usr/bin/sed /usr/bin/grep /usr/bin/shasum /usr/bin/stat /usr/bin/find /usr/bin/sort /bin/cp /bin/mv; do
    if [[ -e "$t" ]]; then echo "tool $t $(shasum -a 256 "$t" | awk '{print $1}')"; fi
  done
  echo "bash_version=$(/opt/homebrew/bin/bash --version | head -1)"
  echo "tmux_version=$(/opt/homebrew/bin/tmux -V)"
  echo "git_version=$(git --version)"
} 
