# ae demo — scripted version with simulated agent output
#
# This version uses a pre-recorded tmux session for predictable output.
# It demonstrates ae's workflow without depending on live LLM responses.
#
# Prerequisites:
#   1. Install VHS: go install github.com/charmbracelet/vhs@latest
#   2. Run: vhs demo-scripted.tape
#
# Strategy: We show the real ae commands and config, but use a simple
# project where agents respond quickly (e.g., a ping-pong test).
# For the "hero" recording, do a live run with generous Sleep timers
# and edit the MP4 afterward to cut dead air.

Output demo.gif
Output demo.mp4

# --- Display ---
Set Shell "bash"
Set FontSize 16
Set FontFamily "JetBrains Mono"
Set Width 1400
Set Height 800
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 40ms
Set CursorBlink false

# ┌──────────────────────────────────────────────┐
# │  Act 1: What is ae? (5s)                     │
# └──────────────────────────────────────────────┘

Sleep 500ms
Type "# ae — multi-agent workspace in one command"
Enter
Sleep 1s
Type "# agents collaborate via tmux, survive reboots, zero deps"
Enter
Sleep 2s

# ┌──────────────────────────────────────────────┐
# │  Act 2: Show the config (5s)                 │
# └──────────────────────────────────────────────┘

Type "cat ~/.ae/config"
Enter
Sleep 4s

# ┌──────────────────────────────────────────────┐
# │  Act 3: Launch (8s)                          │
# └──────────────────────────────────────────────┘

Type "ae demo"
Sleep 300ms
Enter
Sleep 8s

# ┌──────────────────────────────────────────────┐
# │  Act 4: Work — give a task to lead (20s)     │
# └──────────────────────────────────────────────┘

Type@80ms "write a python function that checks if a number is prime, with tests"
Sleep 300ms
Enter
Sleep 20s

# ┌──────────────────────────────────────────────┐
# │  Act 5: Collaborate — lead asks coworker (20s)│
# └──────────────────────────────────────────────┘

Type@80ms "ask codex:coworker to review your implementation"
Sleep 300ms
Enter
Sleep 12s

# Peek at codex pane (tmux pane switch)
Ctrl+B
Sleep 200ms
Type "o"
Sleep 8s

# Back to lead
Ctrl+B
Sleep 200ms
Type "o"
Sleep 5s

# ┌──────────────────────────────────────────────┐
# │  Act 6: Session management (10s)             │
# └──────────────────────────────────────────────┘

# Detach
Ctrl+B
Sleep 200ms
Type "d"
Sleep 1s

# List sessions
Type "ae list"
Enter
Sleep 3s

# Re-attach (sessions survive)
Type "ae demo"
Enter
Sleep 5s

# Detach again
Ctrl+B
Sleep 200ms
Type "d"
Sleep 1s

# ┌──────────────────────────────────────────────┐
# │  Act 7: Clean exit (5s)                      │
# └──────────────────────────────────────────────┘

Type "ae end demo -f"
Enter
Sleep 3s

# ┌──────────────────────────────────────────────┐
# │  Outro (3s)                                  │
# └──────────────────────────────────────────────┘

Type "# github.com/clemens33/ae"
Enter
Sleep 3s
