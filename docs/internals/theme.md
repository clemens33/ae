# The session look

ae draws its own sessions: two status lines, a glyph on every window, a title on
every pane border, and a style for the menu the picker opens. All of it is
**session-scoped**. A tmux server can hold ae sessions and your own side by
side, and yours keeps your theme.

`src/theme.rs` is pure — colours, glyphs and format strings in, format strings
out. Who writes them and when is the launch's and the watchdog's business.

## Three rules

**Opt-out, not opt-in.** `[workspace] theme = off` leaves `status-format`, the
pane borders and the menu styles exactly as your own tmux configuration left
them. ae still publishes every `@ae_*` value, so a hand-written `status-right`
can carry ae's facts in your own layout. `motion = off` freezes the spinner on
its mark. Both knobs are session-scoped like everything else here.

**Session-scoped, never global.** `status`, `status-style` and the two
`status-format` indices are session options, so ae writes them on its own
sessions. The pane borders, the window entries and the menu and popup styles are
**window** options — measured on tmux 3.7b, `set-option -t <session>` on one of
those lands on that session's *current* window and silently leaves the others on
the global table — so ae stamps each window individually and never touches `-g`.

**Three writers, one job each.** A launch writes the layout, the look facts and
the attention seed. A rename rewrites the layout and the facts, and leaves every
verdict alone. The watchdog owns the verdicts, refreshing them each cycle, and
rewrites the layout only when the look stamp says the look changed underneath
it. Every format references `@ae_*` rather than a fact of its own, so nothing
polls and no format ever contains `#()`, which would run a shell command on
every redraw.

The stamp is what makes the knobs live. Each cycle compares it with the look the
session's own options now declare, so turning one on a running session rewrites
the bar instead of leaving half of it behind. Turning the look off UNSETS the
session options rather than blanking them, which is how tmux falls back to the
user's own global status line. The stamp only advances once every write in that
pass has landed, so a partial repaint is retried rather than remembered as done.

**Spaces, not commas, inside `#[…]`.** A `#{W:a,b}` or `#{?c,a,b}` splits on
commas, so a comma-separated style list inside one tears the format in half.
tmux reads a space-separated style list identically, and its own default
`status-format[0]` is written that way for exactly this reason.

## The six marks

One vocabulary, shared by the status bar, the window entries, the pane borders
and the `ae orchestrator` picker. Every mark has its own glyph as well as its
own accent, so a reader who cannot separate two of the colours still reads two
different characters.

| Mark | Glyph | ASCII | Means |
|---|---|---|---|
| dead | `✖` | `x` | the process behind the pane is gone |
| needs-you | `⚠` | `!` | waiting-user, blocked, throttled, unanswered |
| working | `●` | `*` | moving, or recently moved |
| done | `✓` | `+` | declared complete or paused |
| stale / unknown | `◌` | `?` | silent past the window, or a fact ae could not establish |
| idle | `·` | `-` | no agent, or no verdict yet |

Dead keeps its own mark because "this will never move again" is not the news
that "this is waiting for you" is, and a gone process must never be drawn like a
session that finished. Stale and unknown share a glyph and never a WORD: the
reason beside the mark says which of the two it was.

A pane that produced output since the watchdog's last capture shows a braille
spinner frame in place of the working glyph. It is not an animation: the frame
advances once per watchdog cycle, and the cycle is 60 seconds by default. So the
spinner reads as "this moved since I last looked", and a bar that is spinning
one frame per minute is telling you about the last minute rather than about now.
Live motion belongs to a surface that redraws for itself, not to a status line
one daemon repaints.

The spinner is subordinate to the state: it stands in for the working glyph and
nothing else, so a pane that needs a human keeps saying so while it prints.
`[workspace] icons = off` selects the ASCII column, and `motion = off` freezes
the frame on its mark. The watchdog re-reads the look every cycle, so flipping
either knob on a live session takes effect on the next one.

## What goes when the bar runs out of room

One order, everywhere. The path first, because it is the one fact the reader's
own shell prompt already carries. Then the profile in a pane border title, which
the roster also holds. Then the calmest fleet entries, never the ones asking for
something, and the strip says `+N` for what it dropped.

The bar is a hierarchy: the session, its windows and its agents come first, and
the watch counters step out of it while everything is healthy. The moment a pane
is dead or stale the counts come back in that mark's accent, because then they
are the news.

## Colour is never the only signal

Measured against these palettes, dark text on an accent clears 4.25:1 at best.
That is fine for a glyph and thin for a word, so the accent carries the badge
and every essential WORD stays as text on a neutral ground. Each accent is
paired with a glyph, and each glyph with a reason word on the pane border.

## The two status lines

`status-format[0]` — the session's attention glyph in its accent and its name as
plain text, then the windows (a mark instead of tmux's `*` and `-` flags, with
`Z` kept because a zoomed pane hides the rest of the window), then on the right
the branch, the goal, the shortened path and the watch segment.

`status-format[1]` — the **fleet strip**: every ae session on this server, most
actionable first, then this session's own agents with their live marks. Each
strip entry is a tmux `range=session` region, so tmux's own default
`MouseDown1Status` binding (`switch-client -t =`) makes it clickable: ae adds no
key binding, which would be a server-global write on your key table.

The strip is one `list-sessions` call. Each session's watchdog publishes its own
`@ae_attn_rank`, and every other session reads it back and draws the glyph in
its OWN vocabulary — so no session walks another session's state, and a session
running the ASCII fallback never inherits someone else's braille.

One snapshot feeds every surface. The marks the agent strip draws, the mark the
session publishes for other sessions to sort on, and the words on the pane
borders all come from the same cycle's judgement, including the slots whose pane
has gone missing.

## The `@ae_*` interface

| Option | Scope | Written by |
|---|---|---|
| `@ae_palette`, `@ae_icons`, `@ae_look`, `@ae_motion` | session | launch, rename |
| `@ae_look_stamp`, `@ae_paths` | session | launch, rename, watchdog |
| `@ae_attn_glyph`, `@ae_attn_rank`, `@ae_attn_style` | session | launch seeds them once, watchdog owns them after |
| `@ae_fleet_strip`, `@ae_agents_status`, `@ae_watchdog_status` | session | watchdog |
| `@ae_goal_status` | session | watchdog |
| `@ae_branch_status`, `@ae_branch_name` | session | watchdog |
| `@ae_window_status` | window | watchdog |
| `@ae_theme` | window | launch, spawn, watchdog |
| `@ae_agent`, `@ae_slot`, `@ae_profile` | pane | launch, spawn |
| `@ae_agent_label` | pane | launch, spawn, watchdog |
| `@ae_pane_state`, `@ae_pane_accent` | pane | watchdog |

`@ae_agent` is the pane's IDENTITY — the roster, the monitor's own names and
every pane lookup match on it, so it is stored exactly as it was given.
`@ae_agent_label` is the same name as the border DRAWS it, and it is the one the
format reads. The watchdog rewrites the label every cycle from the identity
beside it, which is how a session upgraded in place gets one without being
relaunched.

The attention trio is the one place where "launch writes it" and "the watchdog
owns it" meet. A launch SEEDS it, so a session says something true in the
seconds before the first cycle, and nothing writes it again — a rename
re-renders the layout and the facts and leaves the verdicts alone, because every
other session on the server sorts its fleet strip on them.

`@ae_theme` carries the option set's version and the look the window was dressed
in, so a window dressed in another palette does not read as dressed. Nothing
stamps a window in a look ae did not actually read: a failed probe leaves the
window alone rather than standing the default in.

A tmux option **value** interpolates literally: `##` renders as two characters
and `#{…}` is not re-expanded. What the drawer *does* read out of a value is
`#[…]`, which is how the watchdog publishes colour. So values carry styles and
are never format-escaped, while text baked into a FORMAT — the session name in
`status-format[0]` — is escaped through `tmux::format_literal`.

That leaves free text nowhere to hide, so it is dropped rather than escaped:
the goal, an agent's name on the strip, a pane's profile and the shortened path
all lose their `#` on the way into their option. A fleet row is proven before it
is drawn — the name against the session grammar, the id against `$<digits>`, the
rank against the marks that exist — so a session ae did not create cannot
restyle another session's strip.

## Colour

Every colour is a six-digit hex, and tmux down-converts RGB to the nearest
256-colour entry when the terminal does not report `RGB`. So one spelling serves
both, and `terminal-features` — a **server** option, and therefore yours — is
never touched.

**Darcula** is the default, and every one of its tokens is the JetBrains IDE's
own: the accents are its syntax colours, so a mark reads the way the code in the
pane below it already does. Two further variants, `a` (neutral dark) and `b`
(warmer), differ from each other only in their neutrals.

`[workspace] palette = darcula | a | b`.
