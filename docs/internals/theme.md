# The session look

ae draws its own sessions: two status lines, a terminal title, named agents in
every window entry, a title on every pane border, and a style for the menu the picker opens. All of it is
**session-scoped**. A tmux server can hold ae sessions and your own side by
side, and yours keeps your theme.

`src/theme.rs` is pure — colours, glyphs and format strings in, format strings
out. Who writes them and when is the launch's and the watchdog's business.

## Three rules

**Opt-out, not opt-in.** `[workspace] theme = off` leaves `status-format`, the
pane borders and the menu styles exactly as your own tmux configuration left
them. ae still publishes every `@ae_*` value, so a hand-written `status-right`
can carry ae's facts in your own layout. `motion = off` freezes the working `●`
at its accent colour. Both knobs are session-scoped like everything else here.

**Session-scoped, never global.** `status`, `status-style`,
`window-status-separator`, `set-titles`, `set-titles-string` and the two
`status-format` indices are session options, so
ae writes them on its own sessions. The pane borders, the window entries and the menu and popup styles are
**window** options — measured on tmux 3.7b, `set-option -t <session>` on one of
those lands on that session's *current* window and silently leaves the others on
the global table — so ae stamps each window individually and never touches `-g`.
The launch also stamps a session-scoped `client-session-changed` hook that
selects the lead window and pane by pane id; it is a focus rule, not a look
option, and is present when `theme = off` too. One input rule cannot be
session-scoped: on a positively selected ae-owned server, launch replaces the
root `MouseDown1Status` binding. Window ranges use `select-window -t =`, so a
tab click does not fire the session hook and bounce back to the lead window;
session ranges keep tmux's default `switch-client -t =`, so the fleet strip
still switches sessions and the hook still focuses the destination's lead.
Launch never writes this server-global binding on an ambient server, where the
root key table belongs to the user. Every launch on an owned server reasserts
the same binding, so the write is idempotent.

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
| working | `●` | `*` | active within the watchdog's liveness window |
| done | `✓` | `+` | declared complete or paused |
| stale / unknown | `◌` | `?` | silent past the window, or a fact ae could not establish |
| idle | `·` | `-` | no agent, or no verdict yet |

Dead keeps its own mark because "this will never move again" is not the news
that "this is waiting for you" is, and a gone process must never be drawn like a
session that finished. Stale and unknown share a glyph and never a WORD: the
reason beside the mark says which of the two it was.

A ticker runs between the watchdog's 60-second verdict cycles. Every attached
pane whose latest verdict mark is working gets one pulsing `●` frame every
100 ms: its foreground breathes from near-background to bright working accent
over two seconds. One batched tmux invocation advances its pane border,
window entry and fleet strip together. Every fifth frame refreshes the pane and
fleet observations; the four frames between reuse that snapshot, keeping tmux
reads at 500 ms while animation runs at 10 fps. Detached sessions receive no
ticker writes; a last visible frame can remain cached until the next verdict
cycle republishes the static working glyph, but no client can see it. The
the pulse declares the cached verdict; it does not infer terminal motion.
Liveness belongs to the 60-second verdict
cycle, which changes an inactive pane to stale. Tmux redraws changed user
options for every attached client without waiting for `status-interval`, so no
format polls and no status-interval setting participates in animation.
A pane spawned after the last verdict cycle has no cached verdict yet, so it
starts animating from the next cycle.

The pulse is subordinate to the state: it stands in for the working glyph and
nothing else, so done, needs-you, dead, stale and idle panes stay still.
`[workspace] icons = off` selects the ASCII column. `motion = off` or
`theme = off` disables the ticker. The watchdog re-reads the look every cycle, so
flipping either knob on a live session takes effect on the next one.

## What goes when the bar runs out of room

One order, everywhere. The path first, because it is the one fact the reader's
own shell prompt already carries. Then the calmest fleet entries, never the ones
asking for something, and the strip says `+N` for what it dropped. A pane border
title is only ever the agent's name and its state; the profile behind the seat is
a pane fact (`@ae_profile`) for a reader that asks, never a drawn word.

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

`status-format[0]` — the session's attention glyph in its accent, then the
windows. Each window leads with its live mark and then names its agent
(`0:✓lead`) or separates multiple agents with their marks
(`0:✓lead ●colead`); a window with no agent panes falls back to its tmux name.
Adjacent windows have a two-space separator. `Z` stays because a zoomed pane
hides the rest of the window.
The selected window uses the palette's selection ground and ink. The right
side carries the branch, goal, shortened path and watch segment. The session
name is shown once in the fleet strip below.

`status-format[1]` — the **fleet strip**: every non-orchestrator ae session in
the order it was created, each with its live glyph. A session keeps its place
while its attention changes, so a click never moves the thing that was clicked;
the current session uses the palette's selection ground and ink, not a new
position. The orchestrator is rendered immediately before the version segment —
the ONE place the seat is drawn — with its own verdict mark and a tmux
`range=session` target for the canonical `orchestrator` session. When current,
selection colours mark it in place; it never jumps into the fleet strip. Three
spaces separate it from the fleet and one space separates it from `ae <version>`.
Each strip entry is a tmux `range=session` region, so
ae's root `MouseDown1Status` binding sends it through tmux's default
`switch-client -t =` action. The binding is the one server-global exception
described above and is installed only on an ae-owned server.
The bottom-right `ae <version>` segment is another session range when an
orchestrator exists, targeting its `$<n>` id; the orchestrator's own segment and
a fleet without one stay plain text.

The ticker refreshes the strip from one `list-sessions` call every 500 ms and
rewrites it only when a rank, name, order or working frame changed. Each
session's watchdog publishes its own `@ae_attn_rank`, and every other session
reads it back and draws the glyph in its OWN vocabulary — so no session walks
another session's state, and a session running the ASCII fallback never
inherits someone else's braille. Dead, needs-you and stale outrank a working
agent in the session rollup, so attention always stops that session's pulse.

One snapshot feeds every surface. The marks the agent strip draws, the mark the
session publishes for other sessions to sort on, and the words on the pane
borders all come from the same cycle's judgement, including the slots whose pane
has gone missing. A missing agent has no window entry to name it: it remains
visible only through the session mark (the fleet-strip glyph and terminal title)
and through `ae list` or the session brief.

### Terminal titles

When the look is on, ae enables tmux terminal titles and sets
`set-titles-string` to `#{@ae_attn_glyph} ae`. A terminal that exposes tmux's title
therefore shows the session's current attention glyph and the word `ae`, nothing
more: the fleet strip names the current session, and the tab is not asked to.
Titles are session-scoped and follow the same look-stamp repaint as the status
lines; `[workspace] theme = off` leaves the user's title settings untouched.

## The `@ae_*` interface

| Option | Scope | Written by |
|---|---|---|
| `@ae_palette`, `@ae_icons`, `@ae_look`, `@ae_motion` | session | launch, rename |
| `@ae_look_stamp`, `@ae_paths` | session | launch, rename, watchdog |
| `@ae_attn_glyph`, `@ae_attn_rank`, `@ae_attn_style` | session | launch seeds them once, watchdog owns them after |
| `@ae_fleet_strip`, `@ae_orchestrator_strip`, `@ae_watchdog_status` | session | watchdog |
| `@ae_goal_status` | session | watchdog |
| `@ae_version` | session | watchdog (the core it runs on, `ae <version>`) |
| `@ae_orchestrator_id` | session | watchdog (the local fleet's orchestrator target) |
| `@ae_branch_status`, `@ae_branch_name` | session | watchdog |
| `@ae_window_agents` | window | watchdog |
| `@ae_window_plumbing` | monitor window | launch, watchdog lifecycle |
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
other session on the server draws its fleet-strip glyphs from them and sheds on
them when the strip overflows.

`@ae_theme` carries the option set's version and the look the window was dressed
in, so a window dressed in another palette does not read as dressed. Nothing
stamps a window in a look ae did not actually read: a failed probe leaves the
window alone rather than standing the default in.

A tmux option **value** interpolates literally: `##` renders as two characters
and `#{…}` is not re-expanded. What the drawer *does* read out of a value is
`#[…]`, which is how the watchdog publishes colour. So values carry styles and
are never format-escaped, while text baked into a FORMAT — the session name in
the fleet strip — is escaped through `tmux::format_literal`.

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
