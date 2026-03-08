# ae demo — multi-agent coding workspace in one command
#
# This is a VHS tape file for generating a terminal demo.
# Run: vhs demo.tape
#
# NOTE: This tape is designed for a pre-configured environment where
# ae, claude, and codex are installed and authenticated. The actual
# LLM responses are live — adjust Sleep timers based on your network.
# For a predictable recording, consider a mock setup (see demo-mock.tape).

Output demo.gif
Output demo.mp4

# --- Display settings ---
Set Shell "bash"
Set FontSize 16
Set FontFamily "JetBrains Mono"
Set Width 1400
Set Height 800
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 50ms
Set CursorBlink false

# --- Scene 1: Show the config (3 seconds) ---
Sleep 500ms
Type "cat ~/.ae/config"
Sleep 300ms
Enter
Sleep 3s

# --- Scene 2: Launch ae (the money shot) ---
Type "ae demo"
Sleep 500ms
Enter

# Wait for tmux session to spin up and agents to launch
Sleep 8s

# --- Scene 3: Give the lead agent a task ---
Type "Add a fibonacci function to utils.py with type hints and a docstring"
Sleep 300ms
Enter

# Wait for the agent to work
Sleep 15s

# --- Scene 4: Show agent collaboration — ask codex to review ---
# The lead agent should have written code by now.
# We'll demonstrate the ask helper.
#
# (If the agent doesn't auto-delegate, we prompt it)
Type "Now ask codex:coworker to review your changes"
Sleep 300ms
Enter

# Wait for cross-agent communication
Sleep 20s

# --- Scene 5: Show the other agent's pane briefly ---
# Use tmux to switch panes
Ctrl+B
Sleep 200ms
Type "o"
Sleep 5s

# Switch back to main pane
Ctrl+B
Sleep 200ms
Type "o"
Sleep 3s

# --- Scene 6: Clean exit ---
# Detach from tmux first
Ctrl+B
Sleep 200ms
Type "d"
Sleep 1s

# Show session still exists
Type "ae list"
Sleep 300ms
Enter
Sleep 3s

# End the session
Type "ae end demo -f"
Sleep 300ms
Enter
Sleep 5s

# --- Fin ---
Type "# One command. Multiple AI agents. They talk to each other."
Sleep 1s
Enter
Sleep 3s
