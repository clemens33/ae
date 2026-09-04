# What an upgrade does to your sessions

`ae upgrade` and `ae _install` are the same publish. Since the migration chain landed it is
not only a binary swap, and the reasoning behind that is here rather than in
[AGENTS.md](../AGENTS.md), which carries the rule and not the argument.

## The shape row

Every session meta carries `meta_version=<N>`, written at launch. Today N is 2 and the chain
holds no steps, so it is the version check and the refusals. The first real shape change is a
step with a fixture beside it, not a rewrite of every reader.

The row is younger than the sessions. On the machine this was written for, 28 of 28 live
sessions had none — what they had was `schema=2`, which is the same statement about the same
shape in the word ae used before the chain existed. So a meta with `schema=2` and no row is
PLACED at 2 and the row is stamped in silently on first touch. Only a meta that says neither
has told us nothing, and that one is refused. Getting this backwards would have made the
release that added the chain the release that made every running session unresumable.

`src/migrate.rs::placed` owns the rule, and both readers call it — the chain, which acts on
the answer, and `ae list`, which only reports it. One question, one answer.

## The sweep

Between the new version directory and the repointed command link, the publish asks EVERY
session whether the chain can place it, and only then writes. Per session: the chain, the
core rows rewritten as one locked document, all helper links re-rendered, and for a running
session the watchdog and the Telegram bridge restarted on the new core. Agent panes are never
touched — they run the agent tool, not ae.

Nothing is written until every session has been asked, so a session that cannot be migrated
aborts the publish by name with the old link intact. An abort later than that names the
sessions that did move rather than claiming none did.

`ae upgrade` hands the publish to the DOWNLOADED core, as the `install` bootstrap already
does. The steps for versions N..M belong to the core being installed; a publish run in-process
by the old core would migrate with the rules of the release it is replacing, and on the first
real schema change would have no step to run. One consequence is unavoidable and one-time:
upgrading FROM a core older than the chain runs that core's publish, which has no sweep, so
those sessions arrive unmigrated.

## The version sweep

After the journal is removed — not before, because the journal's `link_old` names the
directory a rollback would relink to — every `versions/<V>` no session records is deleted.
One installed version is the consequence, and so is the loss of relink-to-yesterday.

Three things guard it. A publisher lock is held from before the first mutation until after
the sweep: the journal is rollback state and is gone by then, so exclusion cannot rest on it,
and without the lock one publisher could prune another's freshly published version out from
under the live command link. The session census is TYPED as well as fallible: a state root
with no `sessions/` directory answers "none", every other failure — including one entry that
vanished mid-walk — is an error that skips the whole sweep with a warning, because a keep-set
built from a partial reading authorises deleting the core a session is running on.

And the sweep never deletes the directory the command link resolves into, read immediately
before the deletions rather than with the rest of the keep-set. That floor is there for the
one publisher the lock cannot exclude: a core older than the lock does not take it, so during
the release that introduces it a second, older publish could repoint the command in the
middle of this one. Everything else a stale keep-set gets wrong costs disk space. That would
cost the command.

## Why a live probe does not go in `ae-dev`

A publish is `$HOME`-pinned end to end: the version directories are `$HOME/.ae/versions`, the
command link is `$HOME/.local/bin/ae`, and the sessions it migrates are `$HOME/.ae/sessions`.
`AE_HOME` does not move any of that, and it cannot — the command link has to be the one on
`PATH`.

So there is no `ae-dev`-scoped publish, and a checkout build whose state root points
elsewhere now REFUSES `ae upgrade` before it downloads anything, naming both roots. Before
that refusal existed, `ae-dev upgrade` would have reached straight past its own namespace and
migrated, repointed and pruned the real fleet.

A live upgrade probe therefore runs against a sandboxed `$HOME` — its own `HOME`, its own
`AE_HOME` pointing at that home's `.ae`, and its own tmux socket. That is the same isolation
`ae-dev` gives for everything else, arranged so the one door `ae-dev` cannot close stays shut.
