set dotenv-load
set positional-arguments

CI := env("CI", "false")
GIT_REMOTE := env("GIT_REMOTE", "origin")
default_branch := "main"

# ── Development ──────────────────────────────────────────────────────

# Run all quality checks
check: lint format-check

# THE SUBJECT IS THE BASH THAT STILL EXISTS, and after slice Z4 that is ONE file.
# The bash suites are retired into `tests/it` and the contrib sidecars' runners
# went with the sidecars, so what is left to lint is `install` — the bootstrap
# that has to run before there is a core to run.
#
# `< /dev/null` IS THE FIX FOR #67, not tidiness. shellcheck reads stdin when fd
# 0 is open, and an agent harness hands its tool calls a UNIX SOCKET. If that
# socket's peer has not closed by the time the read happens, it never returns
# EOF and the process blocks FOREVER at 0.0% CPU — wedges observed at 4h40m,
# 8h40m, 16h52m and 18h33m beside successful runs of the same command, with
# nothing in between: a race on the peer's close, not slowness. Every input is
# already named on the argv, so this costs nothing and removes the race.
# Reproduce on demand with a fifo no one ever closes; a plain rerun passes by
# luck most of the time. `tests/it/gate.rs` pins the redirect structurally,
# because it reads like line-noise to the next person tidying the recipe.

# Enforce the linter pin. Kept OUT of `lint` deliberately: the #67 gate requires the
# lint recipe to contain exactly ONE `shellcheck` token, so that the `< /dev/null` on
# that line provably protects the invocation's stdin. A version probe inside the body
# would be a second token and would defeat the check that exists to stop the wedge.
_shellcheck-pin:
    #!/usr/bin/env bash
    set -euo pipefail
    want="0.11.0"
    # AVAILABILITY BEFORE VERSION. Under `set -e` with pipefail, probing the version of a
    # binary that is not installed aborts this recipe at rc 127 with the probe's stderr
    # already redirected, so `just` prints an exit code and NONE of the guidance below —
    # exactly the fresh machine that needs it most. Ask whether the tool exists first, and
    # let a probe that runs but prints nothing recognisable fall through to the same
    # message instead of killing the shell.
    if command -v shellcheck >/dev/null 2>&1; then
        have="$(shellcheck --version 2>/dev/null | awk '/^version:/ {print $2}' || true)"
    else
        have=""
    fi
    if [ "${have:-}" != "$want" ]; then
        echo "Error: the bash lint lane is pinned to shellcheck $want, found ${have:-no shellcheck on PATH}" >&2
        echo "       Its finding set moves between releases, so an unpinned linter is a gate" >&2
        echo "       that changes without a commit. Install $want (macOS: brew install shellcheck)" >&2
        echo "       and confirm with: shellcheck --version" >&2
        exit 1
    fi

# `exec` (#342): dash keeps the one command of `sh -cu` as its child, so without it a TERM from
# `just` ends only dash and leaves shellcheck running. `format-check` execs shfmt for the same reason.
# Lint with shellcheck.
# SEVERITY FLOOR: warning and error are the gate; everything BELOW warning — info and
# style — is advisory and is NOT enforced. State it plainly: a green run here does NOT
# mean the raw linter reports nothing. The SC2016 and SC2329 sites carry their own reasoned comments
# instead. Full rationale: docs/development.md.
lint: _shellcheck-pin
    exec shellcheck --severity=warning -x install < /dev/null

# `< /dev/null` here is insurance, not a fix: shfmt reads stdin only when given
# no paths, and it is given two. Deliberately NOT pinned by a test — pinning
# insurance as contract makes the next reader treat it as load-bearing.

# Check formatting (shfmt, diff mode)
format-check:
    exec shfmt -d -i 4 -ci install < /dev/null

# Auto-format
format:
    shfmt -w -i 4 -ci install

# ── Testing ──────────────────────────────────────────────────────────

# THE GATE, and it is one command. Slice Z4 retired the bash suites into
# `tests/it` and the contrib sidecars moved into the core, so ae's whole test
# surface is Rust: `just rust-check` is format, lint and every test together.
#
# Minutes, not the half hour the serial bash integration suite cost. There is no
# inner-loop/gate split any more because there is nothing left to narrow — the
# fast lane and the gate are the same command, which is the point.

# Run all tests — THE GATE
test: rust-check

# ADVISORY pre-check, NOT the gate and never citable as it: a red gate costs
# minutes to learn that a format, lint or guard check failed, and this learns
# the same three in a fraction of that. It is `rust-fmt-check` + `rust-lint` + the
# unit tests (library and bin) + the guard modules of the integration target,
# `doors` (capability boundary), `floor` (tmux floor boundary) and `gate`
# (justfile/install hazards). A green `just pre` says nothing about the rest of
# `tests/it`, the fuzz compile or the doctests; only `just test` does. No retries.
# `-E` is nextest's filterset; the lane skips doctests for a filtered run.

# Advisory fast pre-check (fmt + clippy + unit + guard tests). NOT THE GATE
pre: rust-fmt-check rust-lint _pre-test

_pre-test:
    exec just _tmux-isolated test -E 'kind(lib) | kind(bin) | (binary(it) & test(/^(doors|floor|gate)::/))'

# THE MAC BOUNDARY SUBSET, what a release runs on the Mac while the hp runs the whole suite
# (`just release-proof`, docs/development.md "Release proof"): every unit test, plus the
# integration modules and the exact tests `.config/macos-subset` names. Those are the tests
# that touch what differs on a Mac: OS conditionals, the process table, real tmux clients,
# justfile recipes, fsync, the clipboard and archive tools. The guard
# `tests/it/gate/macos_subset.rs` derives that boundary from the source and refuses a
# manifest that misses part of it. This recipe only reads the manifest, refuses a malformed
# line, and renders the same nextest filterset the guard renders. A filtered run: nextest
# only, no doctests, no receipt. `print=yes` prints the filterset and the selected tests and
# runs nothing.

# Run the Mac boundary subset (`.config/macos-subset`); print=yes only prints it
test-macos *args:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ NEST }}
    print=no
    for arg in "$@"; do
        case "$arg" in
            print=yes) print=yes ;;
            *) echo "usage: just test-macos [print=yes]" >&2; exit 2 ;;
        esac
    done
    {{ SUBSET }}
    subset_render .config/macos-subset || { echo "test-macos: refused: $subset_why" >&2; exit 1; }
    # The guard that keeps the manifest complete runs in this lane whatever the manifest says: a valid
    # sparse manifest that drops `module gate` must not drop the guard from its own lane. The
    # manifest's filterset is OR terms only, so one more term needs no parentheses.
    subset_run="$subset_filterset | (binary_id(ae::it) & test(/^gate::macos_subset/))"
    if [[ "$print" == yes ]]; then
        printf '%s\n' "$subset_run"
        cargo nextest list --locked --all-features --message-format oneline -E "$subset_run"
        exit 0
    fi
    echo "test-macos: $subset_run" >&2
    nest just _tmux-isolated test -E "$subset_run"

# THE NESTED STEP (#340). just passes a TERM only to the recipe shell it started, and a shell that a
# signal ends leaves its foreground child running: a nested just, or the supervisor in `detached`,
# then went on alone with its cargo. `nest <command...>` runs one step as this shell's child and
# waits for it. A trap records the first INT, TERM or HUP and sends TERM to the child on every one;
# the wait is re-entered while the child lives, so the step always ends before this shell does.
# After a signal the result is 128 + the first signal, even when the child exits 0, and every later
# `nest` in the same shell refuses to start (a stopped gate starts no further leg). The traps live
# only while a step runs. Bash 3.2 syntax on purpose: `release-proof` has no bash floor.
#
# `nest_capture <var> <command...>` is `var=$(command...)` for a long step (#345). A command
# substitution is a subshell that the shell waits for outside any trap, so a TERM ends the shell
# and leaves the step running; a `nest` inside it is out of reach, because just signals only the
# recipe shell. Here the step is nest's child with its stdout in a temporary file under TMPDIR,
# which `$(<file)` reads back with the same trailing-newline rule as `$( )`. The text is data,
# never a printf format. The file is removed on every return, and the status is nest's, unchanged.
NEST := '''
    nest_signal=0
    nest_child=""
    nest_on() {
        ((nest_signal)) || nest_signal=$1
        [[ -z "$nest_child" ]] || kill -TERM "$nest_child" 2>/dev/null || true
    }
    nest() {
        local status
        ((nest_signal == 0)) || return $((128 + nest_signal))
        trap 'nest_on 2' INT
        trap 'nest_on 15' TERM
        trap 'nest_on 1' HUP
        "$@" 0<&0 &
        nest_child=$!
        ((nest_signal == 0)) || kill -TERM "$nest_child" 2>/dev/null || true
        while :; do
            status=0
            wait "$nest_child" || status=$?
            kill -0 "$nest_child" 2>/dev/null || break
        done
        nest_child=""
        trap - INT TERM HUP
        ((nest_signal == 0)) || return $((128 + nest_signal))
        return "$status"
    }
    nest_capture() {
        local nest_var=$1 nest_out nest_status=0
        shift
        nest_out="$(mktemp "${TMPDIR:-/tmp}/nest-capture.XXXXXX")" || return 1
        nest "$@" >"$nest_out" || nest_status=$?
        printf -v "$nest_var" '%s' "$(<"$nest_out")"
        rm -f "$nest_out"
        return "$nest_status"
    }
'''

# THE DISK FLOOR of `release` and `lane-lock claim`. A full disk (errno 28 in target/debug/deps)
# once stopped a release after its claim and left a suspect lock, so both refuse BEFORE any
# effect when a volume they are about to fill is short of space. `disk_floor <recipe> [path]`
# checks the volume of the checkout's toplevel, of the optional extra path (a claim passes the
# shared `.local/locks` of the MAIN checkout, which a linked worktree may not share a mount with)
# and of the cargo `target_directory` and `build_directory` that `cargo metadata --no-deps
# --locked` reports (so CARGO_TARGET_DIR, CARGO_BUILD_TARGET_DIR and the `build.target-dir` and
# `build.build-dir` config keys count, as they do for cargo), each volume once. Both directory
# fields are read STRUCTURALLY, as the top-level keys of the one JSON object: a string of the same
# name inside a package or the `metadata` tables never counts, and cargo emits those after its own
# directories. The pinned toolchain's cargo emits both fields, so a missing one refuses. The floor
# is AE_LANE_MIN_FREE_GIB whole GiB, default 20; 0 turns the check off before any df or cargo
# call; anything else but whole GiB refuses with exit 2. A refusal exits 1 and names the volume,
# its free space and the floor. Anything it cannot read refuses too (fail closed): cargo
# metadata, a path with a backslash, a df line it cannot parse. df is parsed by the first `size
# used available capacity` run, so a filesystem or mount point with spaces parses; a path that
# does not exist yet is probed through its nearest existing ancestor and never created. Bash 3.2
# safe.
DISKFREE := '''
    disk_floor() {
        local who="$1" floor="${AE_LANE_MIN_FREE_GIB:-20}" top meta fields key path probe row avail mount say dup i
        local -a paths mounts=()
        shift
        if [[ ! "$floor" =~ ^(0|[1-9][0-9]{0,5})$ ]]; then
            echo "$who: refused: AE_LANE_MIN_FREE_GIB is whole GiB without a leading zero (0 turns the floor off), got '$floor'" >&2
            exit 2
        fi
        ((floor > 0)) || return 0
        top="$(git rev-parse --show-toplevel 2>/dev/null)" && top="$(cd -P "$top" && pwd -P)" || {
            echo "$who: refused: not inside a git checkout, so the volume to check is unknown" >&2
            exit 1
        }
        meta="$(cargo metadata --no-deps --locked --offline --format-version 1 --manifest-path "$top/Cargo.toml" 2>/dev/null)" || {
            echo "$who: refused: cargo metadata failed, so the build volume is unknown (AE_LANE_MIN_FREE_GIB=0 turns the floor off)" >&2
            exit 1
        }
        # The top-level string fields of the one JSON object: depth 1, a key then a string value.
        fields="$(printf '%s\n' "$meta" | awk '
            {
                n = length($0)
                for (i = 1; i <= n; i++) {
                    c = substr($0, i, 1)
                    if (instr) {
                        if (esc) { buf = buf c; esc = 0 }
                        else if (c == "\\") { buf = buf c; esc = 1 }
                        else if (c == "\"") {
                            instr = 0
                            if (depth == 1) {
                                if (st == 0) { key = buf; st = 1 }
                                else if (st == 2) { val[key] = buf; st = 0 }
                            }
                        } else buf = buf c
                        continue
                    }
                    if (c == "\"") { instr = 1; buf = ""; continue }
                    if (c == "{" || c == "[") { if (depth == 1 && st == 2) st = 3; depth++; continue }
                    if (c == "}" || c == "]") { depth--; if (depth == 1 && st == 3) st = 0; continue }
                    if (c == ":" && depth == 1 && st == 1) { st = 2; continue }
                    if (c == "," && depth == 1 && st == 2) { st = 0; continue }
                }
            }
            END {
                if (depth == 0 && !instr && ("target_directory" in val) && ("build_directory" in val)) {
                    print "target_directory\t" val["target_directory"]
                    print "build_directory\t" val["build_directory"]
                }
            }')" || fields=""
        paths=("$top" "$@")
        for key in target_directory build_directory; do
            path="$(printf '%s\n' "$fields" | awk -F '\t' -v k="$key" '$1 == k { print substr($0, length(k) + 2); exit }')"
            if [[ -z "$path" ]]; then
                echo "$who: refused: cargo metadata names no top-level $key, so the build volume is unknown" >&2
                exit 1
            fi
            if [[ "$path" != /* || "$path" == *\\* ]]; then
                echo "$who: refused: cargo's $key is not a plain absolute path: $path" >&2
                exit 1
            fi
            paths+=("$path")
        done
        for path in "${paths[@]}"; do
            probe="$path"
            while [[ -n "$probe" && ! -e "$probe" ]]; do probe="${probe%/*}"; done
            [[ -n "$probe" ]] || probe=/
            row="$(LC_ALL=C df -Pk "$probe" 2>/dev/null | awk 'NR == 2 { if (match($0, /[0-9]+ +[0-9]+ +[0-9]+ +([0-9]+%|-) +/)) { split(substr($0, RSTART, RLENGTH), f, / +/); print f[3] "\t" substr($0, RSTART + RLENGTH) } }')" || row=""
            avail="${row%%$'\t'*}"
            mount="${row#*$'\t'}"
            if [[ -z "$row" || ! "$avail" =~ ^[0-9]+$ || -z "$mount" ]]; then
                echo "$who: refused: df could not read the volume of $path (probed $probe)" >&2
                exit 1
            fi
            dup=0
            for ((i = 0; i < ${#mounts[@]}; i++)); do
                [[ "${mounts[i]}" != "$mount" ]] || dup=1
            done
            ((dup == 0)) || continue
            mounts+=("$mount")
            if ((avail < floor * 1048576)); then
                say="$((avail / 1048576)).$((avail % 1048576 * 10 / 1048576))"
                echo "$who: refused: $mount has $say GiB free, below the $floor GiB floor (AE_LANE_MIN_FREE_GIB); checked for $path" >&2
                exit 1
            fi
        done
    }
'''

# THE GATE SUPERVISOR (#340, #149), the perl text `detached` runs and `lane-lock release` checks
# with. `run <command...>`: cargo runs in a session of its own with no controlling terminal (#149);
# this process supervises its group G, records the run in `<main checkout>/.local/locks/groups/`
# before cargo can exist under the claim it serves (AE_LANE_CLAIM, else the worktree binding,
# #343), and does not end while a process of the run is left. `check <dir>
# <owner line>`: the release census of every entry. The registry grammar, the census and the
# residual are in docs/development.md ("The gate process registry"). No single quote may appear
# in this text: the recipes pass it inside one.
REAP := '''
    use strict;
    use warnings;
    use POSIX qw(:sys_wait_h :signal_h setsid);
    use Time::HiRes qw(sleep time);
    use File::Path qw(make_path);
    my %signum = (INT => 2, TERM => 15, HUP => 1);
    my $mode = shift @ARGV // "";
    # The census seam: one ps read of the whole table, C locale. A row is pid, ppid, pgid and the
    # five lstart words; anything else, or a ps that fails, is a failed census (undef).
    sub census {
        local $ENV{LC_ALL} = "C";
        open(my $ps, "-|", "ps", "-A", "-o", "pid=", "-o", "ppid=", "-o", "pgid=", "-o", "lstart=") or return undef;
        my (%row, $bad);
        while (my $line = <$ps>) {
            my @f = split " ", $line;
            if (@f != 8 || "@f[0..2]" !~ /^[0-9]+ [0-9]+ [0-9]+$/) { $bad = 1; next }
            (my $id = join("", @f[3..7])) =~ s/[^A-Za-z0-9]//g;
            $row{$f[0]} = [$f[1], $f[2], $id];
        }
        close($ps) or return undef;
        return ($bad || !%row) ? undef : \%row;
    }
    # A start identity as #339 reads it: lstart, letters and digits only; empty when the pid is gone.
    sub identity {
        local $ENV{LC_ALL} = "C";
        open(my $ps, "-|", "ps", "-o", "lstart=", "-p", $_[0]) or return "";
        my $id = join("", <$ps>);
        close($ps);
        $id =~ s/[^A-Za-z0-9]//g;
        return $id;
    }
    # A registry entry, parsed (plan-p0-r2.md section 1). Returns a hash; {bad} names why it is malformed,
    # {claim} is undef when even the header cannot be read.
    sub parse_entry {
        my ($path) = @_;
        my %e = (members => {});
        open(my $fh, "<", $path) or return { bad => "unreadable", header => 0 };
        my $n = read($fh, my $text, 65537);
        close($fh);
        return { bad => "unreadable", header => 0 } unless defined $n;
        $e{text} = $text;
        return { %e, bad => "larger than 64 KiB", header => 0 } if length($text) > 65536;
        my @lines = split /\n/, $text, -1;
        my $tail = pop @lines;
        my $id = qr/[A-Za-z0-9]{1,64}/;
        my $pid = qr/[1-9][0-9]{0,9}/;
        if (@lines < 3 || $lines[0] ne "ae-gate-group 1" || $lines[1] !~ /^run ($pid) ($id)$/) {
            return { %e, bad => "no readable header", header => 0 };
        }
        @e{qw(spid sid)} = ($1, $2);
        return { %e, bad => "no readable claim line", header => 0 } unless $lines[2] =~ /^claim (.+)$/;
        $e{claim} = $1;
        $e{header} = 1;
        return { %e, bad => "the last line has no newline" } if $tail ne "";
        for my $i (3 .. $#lines) {
            if ($i == 3 && $lines[$i] =~ /^leader ($pid) ($id)$/) { @e{qw(g gid)} = ($1, $2); next }
            if ($i > 3 && defined $e{g} && $lines[$i] =~ /^member ($pid) ($id)$/) { $e{members}{$1} = $2; next }
            if ($i > 3 && $i == $#lines && defined $e{g} && $lines[$i] eq "lost") { $e{lost} = 1; next }
            return { %e, bad => "line " . ($i + 1) . " is not part of the grammar" };
        }
        $e{state} = defined $e{g} ? "running" : "pending";
        return \%e;
    }
    # The run set of an entry in one census: recorded pids alive with the same identity, the rows of
    # group G unless pid G now belongs to another process (G was reused), and every descendant of
    # those, to a fixed point. The supervisor itself is never part of it.
    sub run_set {
        my ($row, $e, $self) = @_;
        my %set;
        my %kept = (%{ $e->{members} }, $e->{g} => $e->{gid});
        for my $p (keys %kept) { $set{$p} = 1 if $row->{$p} && $row->{$p}[2] eq $kept{$p} }
        unless ($row->{ $e->{g} } && $row->{ $e->{g} }[2] ne $e->{gid}) {
            for my $p (keys %$row) { $set{$p} = 1 if $row->{$p}[1] == $e->{g} }
        }
        for (my $grew = 1; $grew;) {
            $grew = 0;
            for my $p (keys %$row) {
                next if $set{$p} || !$set{ $row->{$p}[0] };
                $set{$p} = 1;
                $grew = 1;
            }
        }
        delete $set{$self} if defined $self;
        return \%set;
    }

    if ($mode eq "check") {
        # lane-lock release: one census for every entry. Prints one suspect line per entry that keeps
        # the claim and exits 1 when there is any; removes the entries it proved empty.
        my ($dir, $owner) = @ARGV;
        if (-l $dir || !-d $dir) { print "suspect: groups: the registry $dir is not a plain directory\n"; exit 1 }
        opendir(my $dh, $dir) or do { print "suspect: groups: the registry $dir cannot be read\n"; exit 1 };
        my @names = sort grep { !/^\./ } readdir($dh);
        closedir($dh) or do { print "suspect: groups: the registry $dir cannot be read\n"; exit 1 };
        exit 0 unless @names;
        my $row = census();
        my $refused = 0;
        my $say = sub { print "suspect: $_[0]: $_[1]\n"; $refused = 1 };
        for my $name (@names) {
            my $path = "$dir/$name";
            my $e = parse_entry($path);
            my $mine = !$e->{header} || $e->{claim} eq $owner;
            if ($e->{bad}) { $say->($name, "malformed entry: $e->{bad}") if $mine; next }
            if ($e->{state} eq "pending") { $say->($name, "pending: its supervisor never confirmed a start") if $mine; next }
            if ($e->{lost}) { $say->($name, "lost: its supervisor could not record a member") if $mine; next }
            if (!$row) { $say->($name, "the process census failed") if $mine; next }
            my $set = run_set($row, $e, undef);
            $set->{ $e->{spid} } = 1 if $row->{ $e->{spid} } && $row->{ $e->{spid} }[2] eq $e->{sid};
            if (%$set) { $say->($name, "still running: " . join(" ", sort { $a <=> $b } keys %$set)) if $mine; next }
            # Proved empty, so no writer is left: remove it if it is still the text that was read.
            my $again = parse_entry($path);
            unlink($path) if defined $again->{text} && $again->{text} eq $e->{text};
        }
        exit($refused ? 1 : 0);
    }

    # Run mode: `run <command...>`. Cargo runs in a session of its own with no controlling terminal, so
    # no test can take the terminal of the lane (#149). This process stays in the process group of the caller as
    # the supervisor of the cargo group G: it records the run in the registry before cargo can exist, and
    # it does not end while any process of the run is left (#340).
    die "detached: usage: run <command...>\n" unless $mode eq "run" && @ARGV;
    my $groups = $ENV{AE_GATE_GROUPS} // "";
    my $clamp = sub { my ($v, $d) = @_; (defined $v && $v =~ /^[1-9][0-9]*$/ && $v <= $d) ? $v : $d };
    my $term_secs = $clamp->($ENV{AE_GATE_REAP_TERM_SECS}, 15);
    my $kill_secs = $clamp->($ENV{AE_GATE_REAP_KILL_SECS}, 5);
    my $self = $$;
    my $sid = identity($self);
    my ($entry, $tmp, $header);
    # Writes the entry by temp file and rename, so a reader sees a whole old or a whole new text.
    my $publish = sub {
        my ($text) = @_;
        open(my $fh, ">", $tmp) or return 0;
        my $ok = print {$fh} $text;
        return (close($fh) && $ok && rename($tmp, $entry)) ? 1 : 0;
    };
    # Step 1, before any fork: the pending entry with the claim this run serves (#343).
    # AE_LANE_CLAIM=<name> joins the claim of that name or does not start. Without it, the run
    # serves the holder unless a binding of the current owner line names another worktree:
    # then it serves no claim. Missing proof gives the run to the holder, never frees it. A
    # set AE_LANE_CLAIM, also an empty one, is a claim the caller means: it is never ignored.
    my $want = $ENV{AE_LANE_CLAIM};
    my $refuse = sub { print STDERR "detached: refused: $_[0]\n"; exit 1 };
    if (defined $want && $want !~ /\A[A-Za-z0-9][A-Za-z0-9_.-]{0,63}\z/) {
        # The refusal stays one line: control characters are shown escaped, never raw.
        (my $shown = $want) =~ s/([\x00-\x1f\x7f])/sprintf("\\x%02x", ord($1))/ge;
        $refuse->("AE_LANE_CLAIM is not a lock name: \"$shown\"");
    }
    $refuse->("this run serves lane claim $want, but this checkout has no lane lock") if defined $want && $groups eq "";
    if ($groups ne "") {
        die "detached: no start identity for pid $self\n" if $sid eq "";
        # Only an owner file that is provably absent means an unlocked run. One that exists but
        # cannot be read, or holds no owner line, stops the run here, before any entry or child.
        my ($claim, $owner, $line) = ("free", $ENV{AE_GATE_OWNER} // "");
        if ($owner ne "" && open(my $fh, "<", $owner)) {
            $line = <$fh>;
            close($fh);
            chomp($line) if defined $line;
            die "detached: the lane lock owner $owner holds no owner line\n" unless defined $line && $line =~ /^[^\x00-\x1f]+$/;
        } elsif ($owner ne "" && !$!{ENOENT}) {
            die "detached: the lane lock owner $owner cannot be read: $!\n";
        }
        if (defined $want) {
            $refuse->("this run serves lane claim $want, but the lane is free") unless defined $line;
            (my $holder = $line) =~ s/ .*//s;
            $refuse->("this run serves lane claim $want, but the lane is held by $line") unless $holder eq $want;
            $claim = $line;
        } elsif (defined $line) {
            $claim = $line;
            my ($here, $bind) = ($ENV{AE_GATE_WORKTREE} // "", $ENV{AE_GATE_BINDING} // "");
            if ($here ne "" && $bind ne "" && !-l $bind && -f $bind && open(my $bh, "<", $bind)) {
                my $n = read($bh, my $text, 8193);
                close($bh);
                if (defined $n && $n <= 8192 && $text =~ m{\A([^\x00-\x1f]+)\n(/[^\x00-\x1f]*)\n\z} && $1 eq $line && $2 ne $here) {
                    print STDERR "detached: note: the lane is held by $line from $2; this run in $here is not part of that claim (AE_LANE_CLAIM=<name> makes it one)\n";
                    $claim = "free";
                }
            }
        }
        make_path($groups, { error => \my $ignored });
        ($entry, $tmp) = ("$groups/$self.$sid", "$groups/.$self.$sid.tmp");
        $header = "ae-gate-group 1\nrun $self $sid\nclaim $claim\n";
        -d $groups && !-l $groups && $publish->($header) or die "detached: cannot register the run in $groups: $!\n";
    }
    # The claim proof is spent here: cargo and every test it runs start without it, so a scratch
    # recipe inside a test never inherits the claim of the gate that runs it.
    delete @ENV{qw(AE_LANE_CLAIM AE_GATE_BINDING AE_GATE_WORKTREE)};
    # Step 2: fork with the signals blocked. The child takes a session of its own, then waits for the
    # go byte: it never execs before the running entry is durable, and it exits on EOF.
    my ($set, $old) = (POSIX::SigSet->new(SIGINT, SIGTERM, SIGHUP), POSIX::SigSet->new);
    sigprocmask(SIG_BLOCK, $set, $old) or die "detached: sigprocmask: $!\n";
    pipe(my $go_r, my $go_w) or die "detached: pipe: $!\n";
    defined(my $g = fork) or die "detached: fork: $!\n";
    if (!$g) {
        close($go_w);
        setsid() or POSIX::_exit(127);
        $SIG{$_} = "DEFAULT" for qw(INT TERM HUP QUIT);
        POSIX::_exit(127) unless (sysread($go_r, my $byte, 1) // 0) == 1;
        close($go_r);
        sigprocmask(SIG_SETMASK, $old);
        exec { $ARGV[0] } @ARGV or do { print STDERR "detached: $ARGV[0]: $!\n"; POSIX::_exit(127) };
    }
    close($go_r);
    # Step 3: the running entry (leader G), then the go byte. A failure here reaps the blocked child
    # first, then removes the entry: cargo never starts.
    my $gid = identity($g);
    my (%kept, $marked);
    my $entry_text = sub { $header . "leader $g $gid\n" . join("", map { "member $_ $kept{$_}\n" } sort { $a <=> $b } keys %kept) . ($marked ? "lost\n" : "") };
    if ($groups ne "" && ($gid eq "" || !$publish->($entry_text->()))) {
        kill("KILL", $g);
        waitpid($g, 0);
        unlink($entry);
        die "detached: cannot record the run in $entry\n";
    }
    syswrite($go_w, "g", 1) == 1 or do { kill("KILL", $g); waitpid($g, 0); unlink($entry) if $entry; die "detached: cannot start $ARGV[0]\n" };
    close($go_w);
    # Step 4: a signal is passed to the cargo group at once and starts the cleanup; later ones only pass on.
    my ($first, $status, $reaped);
    for my $name (qw(INT TERM HUP)) {
        $SIG{$name} = sub { $first //= $name; kill($name, -$g) unless $reaped };
    }
    sigprocmask(SIG_SETMASK, $old);
    # Signals one pid, or group G, only while it is still the process the census saw.
    my $signal = sub {
        my ($name, $p, $id) = @_;
        kill($name, $p) if identity($p) eq $id;
    };
    # Step 5: watch, then reap. One census a second while cargo runs keeps every member it ever saw,
    # with its identity, also one that later leaves group G. Cleanup starts on the first signal or
    # when cargo ends with members left: TERM, the TERM grace, KILL, the KILL wait. A member set the
    # entry could not record starts the cleanup too, is retried every pass and ends the run as a suspect.
    my $e = { g => $g, gid => $gid, members => \%kept };
    my ($phase, $deadline, $last, $why, $dirty, $lost) = ("watch", 0, {});
    # The lost mark: a member the entry never recorded may have left group G, so a census of the stale
    # entry could prove nothing. The whole text with a last `lost` line, else that line appended in
    # place; release refuses a lost entry. True once one write landed.
    my $mark = sub {
        return 1 if $marked;
        $marked = 1;
        return 1 if $publish->($entry_text->());
        $marked = 0;
        if (open(my $fh, ">>", $entry)) {
            my $ok = print {$fh} "lost\n";
            $marked = (close($fh) && $ok) ? 1 : 0;
        }
        return $marked;
    };
    while (1) {
        if (!$reaped && waitpid($g, WNOHANG) == $g) { ($status, $reaped) = ($?, 1) }
        my $row = census();
        if ($row) {
            my $run = run_set($row, $e, $self);
            $last = { map { $_ => $row->{$_}[2] } keys %$run };
            my $changed = 0;
            for my $p (keys %kept) { if (!$run->{$p} && !($row->{$p} && $row->{$p}[2] eq $kept{$p})) { delete $kept{$p}; $changed = 1 } }
            for my $p (keys %$run) { next if $p == $g || exists $kept{$p}; $kept{$p} = $row->{$p}[2]; $changed = 1 }
            $dirty = 1 if $changed;
            if ($dirty && $groups ne "") { if ($publish->($entry_text->())) { $dirty = 0 } else { $lost = 1 } }
            last if $reaped && !%$run;
        }
        if ($phase eq "watch" && ($first || $reaped || $lost)) {
            ($phase, $deadline) = ("term", time + $term_secs);
            $signal->("TERM", $_, $last->{$_}) for keys %$last;
            kill("TERM", -$g) unless $row && $row->{$g} && $row->{$g}[2] ne $gid;
        } elsif ($phase eq "term" && time >= $deadline) {
            ($phase, $deadline) = ("kill", time + $kill_secs);
            $signal->("KILL", $_, $last->{$_}) for keys %$last;
            kill("KILL", -$g) unless $row && $row->{$g} && $row->{$g}[2] ne $gid;
        } elsif ($phase eq "kill" && time >= $deadline && $lost && !$mark->()) {
            # Neither the entry nor the mark holds the run, so only this live process keeps release
            # refused: it stays and reaps again (TERM, grace, KILL) until its own census proves empty.
            ($phase, $deadline) = ("term", time + $term_secs);
            $signal->("TERM", $_, $last->{$_}) for keys %$last;
            kill("TERM", -$g) unless $row && $row->{$g} && $row->{$g}[2] ne $gid;
        } elsif ($phase eq "kill" && time >= $deadline) {
            $why = $row ? "still running after KILL: " . join(" ", sort { $a <=> $b } keys %$last) : "the process census failed";
            last;
        }
        sleep($phase eq "watch" ? 1 : 0.1);
    }
    # Step 6: the end. Proved empty: remove the entry (still ours) and end as cargo did. Otherwise the
    # entry stays for lane-lock release, which then keeps the lock with a suspect note.
    if (!$reaped && waitpid($g, defined $why ? WNOHANG : 0) == $g) { ($status, $reaped) = ($?, 1) }
    $status //= 0;
    $SIG{$_} = "DEFAULT" for qw(INT TERM HUP);
    # A lost run ends red and keeps its entry, marked when a write still lands. Unmarked, it got here
    # only with its census proved empty, so release may then remove the stale entry.
    if ($lost) {
        $why //= "a member could not be recorded";
        $why .= " (the lost mark could not be written)" unless $mark->();
    }
    if (defined $why) {
        print STDERR "detached: suspect: " . ($entry // "group $g") . ": $why\n";
        exit($first ? 128 + $signum{$first} : 1);
    }
    if ($groups ne "") {
        my $now = parse_entry($entry);
        unlink($entry) if defined $now->{spid} && $now->{spid} == $self && $now->{sid} eq $sid;
    }
    exit(128 + $signum{$first}) if $first;
    my $sig = $status & 127;
    exit($status >> 8) unless $sig;
    kill($sig, $$);
    exit(128 + $sig);
'''

# THE DETACHED STEP (#340, #342, #345), one copy for every recipe that starts cargo in a gate or a
# release: the test lane (`_tmux-isolated`), `rust-fmt-check`, `rust-lint`, `rust-fuzz-check`, the
# selector and the doctests of `test-affected`, `rust-deny`, and the cargo steps of `release` and
# `bundles`. `detached <command...>` runs one cargo step under the REAP supervisor, through `nest`:
# cargo gets a session of its own with no controlling terminal (#149), the run is recorded
# in the main checkout's registry under the claim it serves (#343) before cargo can exist, and the
# step ends only when no process of the run is left (#340). `nest` alone does not stop a cargo
# step: a TERM to the cargo pid leaves `cargo check` and its compilers running (#342, measured on
# the hp). A recipe includes `{{ NEST }}` before this text. The perl text is kept in `gate_reap`,
# so the supervisor gets the same argv as when `REAP` is written inline. Where `.local` or
# `.local/locks` cannot be a directory, `lane-lock claim` cannot make a lock either, so no claim
# exists to protect: the step runs without a registry, as outside a checkout. A registry that fails
# anywhere else stops cargo from starting.
DETACHED := '''
    gate_groups=""
    gate_owner=""
    gate_binding=""
    gate_worktree=""
    if gate_common="$(git rev-parse --git-common-dir 2>/dev/null)" && gate_common="$(cd -P "$gate_common" && pwd -P)" && [[ "${gate_common##*/}" == .git ]]; then
        gate_root="${gate_common%/.git}"
        if [[ ( -e "$gate_root/.local" && ! -d "$gate_root/.local" ) || ( -e "$gate_root/.local/locks" && ! -d "$gate_root/.local/locks" ) ]]; then
            echo "note: no gate process registry: $gate_root/.local/locks cannot be a directory, so no lane lock can be held for this checkout" >&2
        else
            gate_groups="$gate_root/.local/locks/groups"
            gate_owner="$gate_root/.local/locks/gate/owner"
            # Which claim the run serves (#343): the binding `lane-lock claim` wrote, and this
            # worktree's root computed exactly as the claim computes it (`pwd -P` of the toplevel).
            gate_binding="$gate_root/.local/locks/binding"
            if gate_top="$(git rev-parse --show-toplevel 2>/dev/null)" && gate_top="$(cd -P "$gate_top" && pwd -P)"; then
                gate_worktree="$gate_top"
            fi
        fi
    fi
    detached() {
        nest env AE_GATE_GROUPS="$gate_groups" AE_GATE_OWNER="$gate_owner" AE_GATE_BINDING="$gate_binding" \
            AE_GATE_WORKTREE="$gate_worktree" perl -e "$gate_reap" run "$@"
    }
''' + "gate_reap='" + REAP + "'\n"

# THE BASH FLOOR of a recipe that uses bash 4 syntax (#337). A recipe that opens with this refuses a
# bash older than 4.4 with a message and exit 1, before bash reads any later line. Without it, macOS
# /bin/bash 3.2 stops at the first construct it cannot parse (`[[ -v`, `exec {fd}<`, `local -A`,
# `mapfile`) and then returns the status of the recipe's `trap ... EXIT` command, so a gate that ran
# nothing exits 0; the trap sees `$?` as 0 there, so no trap can repair it. The 4.4 floor is the empty
# `"${a[@]}"` under `set -u` that older bash rejects. It is POSIX shell on purpose and sits BEFORE
# `set -euo pipefail`: a shell that does not know `pipefail` (dash) reaches it too, and it is one
# complete command, which a bash runs before it parses the rest of the script. tests/it/gate/bashfloor.rs
# pins the position, the version patterns, and that every recipe holding bash 4 syntax opens with it.
BASH_FLOOR := '''
    case "${BASH_VERSION-}" in
        4.[4-9].* | 4.[1-9][0-9].* | [5-9].* | [1-9][0-9].*) ;;
        *)
            echo "${0##*/}: refused: needs bash 4.4 or newer, found ${BASH_VERSION:-a shell that is not bash}; put a newer bash first on PATH (macOS: /opt/homebrew/bin) and run it again" >&2
            exit 1
            ;;
    esac
'''

# THE MERGE GATE (M1, one hop), what a slice passes before it lands, while a release still runs
# the whole suite on the hp (`just release-proof`, docs/development.md "The merge gate"). It runs
# the three checks of `just test` (fmt, lint, fuzz compile) and then only the tests the change
# reaches: every unit test, the guard modules `doors`, `floor` and `gate`, the integration
# modules the selector names, and the doctests when a `src/` file changed. The selector is
# `examples/affected.rs` (tests/it/gate/affected.rs): this recipe writes it the change as framed
# records, reads its answer strictly and prints why it chose the whole suite or how many
# integration tests the subset leaves out (the M1 gap). A change it cannot place runs the whole
# lane, and so does any failure of the producer or the selector. The checkout must be clean, and
# HEAD and its tree must not move from the start to the end, or the recipe refuses. It never
# writes a gate receipt: only a whole `just test` vouches for a tree. `base=<ref>` (default main)
# measures the change from the merge-base with that ref. `print=yes` measures the working tree
# instead, dirt included, prints the selector's answer with every reason and runs nothing else.

# Run the merge gate: the checks, then the tests the change reaches; print=yes only prints the selection
test-affected *args:
    #!/usr/bin/env bash
    {{ BASH_FLOOR }}
    set -euo pipefail
    {{ NEST }}
    {{ DETACHED }}
    {{ SIZING }}
    base=main
    print=no
    for arg in "$@"; do
        case "$arg" in
            base=?*) base="${arg#base=}" ;;
            print=yes) print=yes ;;
            *) echo "usage: just test-affected [base=<ref>] [print=yes]" >&2; exit 2 ;;
        esac
    done
    refuse() { echo "test-affected: refused: $*" >&2; exit 1; }
    scratch="$(mktemp -d)"
    doc_dir=""
    trap 'status=$?; rm -rf "$scratch" ${doc_dir:+"$doc_dir"}; exit "$status"' EXIT
    mode=gate
    [[ "$print" == no ]] || mode=preview

    # HEAD, its tree and a clean checkout. Untracked files count: the tests would see them.
    identity() {
        local names dirt
        names="$(git rev-parse HEAD 'HEAD^{tree}' 2>/dev/null)" || { why="git cannot name HEAD and its tree"; return 1; }
        dirt="$(git --no-optional-locks status --porcelain --untracked-files=all 2>/dev/null)" || { why="git cannot read the status of the working tree"; return 1; }
        [[ -z "$dirt" ]] || { why="the working tree is not clean (untracked files count)"; return 1; }
        now="${names//$'\n'/ }"
    }
    steady() {
        identity || refuse "$why $1"
        [[ "$now" == "$start" ]] || refuse "HEAD or its tree moved $1 (${start:0:12} became ${now:0:12})"
    }

    # THE PRODUCER: the change as framed records, `<tag> <bytes>\n<payload>\n`, in the order
    # tests/it/gate/affected.rs `decode` reads them. It runs as an `if` condition, where `set -e`
    # stops nothing, so every git, cat and wc call is checked here. LC_ALL=C makes `${#x}` count bytes.
    record() { printf '%s %s\n%s\n' "$1" "${#2}" "$2" && records=$((records + 1)); }
    change() {
        if (($# == 3)); then
            printf 'change %s\n%s\0%s\0%s\n' "$((${#1} + ${#3} + ${#2} + 2))" "$1" "$3" "$2"
        else
            printf 'change %s\n%s\0%s\n' "$((${#1} + ${#2} + 1))" "$1" "$2"
        fi && records=$((records + 1))
    }
    # A file's bytes as a `new` or `old` record, read without a child process: a few hundred
    # files cost seconds when each needs its own `wc` and `cat`. `read -d ''` stops early only
    # at a NUL byte, so a file that holds one fails here instead of being cut short.
    content() {
        local fd text
        exec {fd}<"$3" || return 1
        if IFS= read -r -d '' text <&"$fd"; then
            exec {fd}<&-
            return 1
        fi
        exec {fd}<&-
        # A read error assigns nothing, not even the text read before it: `text` stays unset.
        [[ -v text ]] || return 1
        printf '%s %s\n%s\0%s\n' "$1" "$((${#2} + 1 + ${#text}))" "$2" "$text" && records=$((records + 1))
    }
    # A `.rs` file under `src/` or `tests/`, as the selector's `is_source` reads the name.
    source_path() { [[ ("$1" == src/* || "$1" == tests/*) && "$1" == *.rs && "${1##*/}" != .rs ]]; }
    produce() {
        local LC_ALL=C merge_base head status from path
        local -a sources=() modified=()
        local -A seen=()
        records=0
        merge_base="$(git merge-base HEAD "$base" 2>/dev/null)" || { why="git finds no merge-base of HEAD and $base"; return 1; }
        head="$(git rev-parse HEAD 2>/dev/null)" || { why="git cannot name HEAD"; return 1; }
        if [[ "$mode" == gate ]]; then
            git diff -z --no-color --no-ext-diff --name-status -M "$merge_base" HEAD >"$scratch/diff" 2>/dev/null &&
                git ls-tree -r -z --name-only HEAD >"$scratch/tracked" 2>/dev/null &&
                : >"$scratch/untracked" || { why="git cannot list the change or the tree"; return 1; }
        else
            git diff -z --no-color --no-ext-diff --name-status -M "$merge_base" >"$scratch/diff" 2>/dev/null &&
                git ls-files -z --others --exclude-standard >"$scratch/untracked" 2>/dev/null &&
                git ls-files -z --cached --others --exclude-standard >"$scratch/tracked" 2>/dev/null || { why="git cannot list the change or the tree"; return 1; }
        fi
        record version 1 && record mode "$mode" && record base "$base" &&
            record merge-base "$merge_base" && record head "$head" || { why="the header could not be written"; return 1; }
        while IFS= read -r -d '' status; do
            case "$status" in
                R* | C*)
                    IFS= read -r -d '' from && IFS= read -r -d '' path || { why="git diff ended inside a rename"; return 1; }
                    change "$status" "$path" "$from" || return 1
                    ;;
                *)
                    IFS= read -r -d '' path || { why="git diff ended inside a record"; return 1; }
                    change "$status" "$path" || return 1
                    if [[ "$status" == [MT] ]] && source_path "$path"; then modified+=("$path"); fi
                    ;;
            esac
        done <"$scratch/diff"
        while IFS= read -r -d '' path; do change U "$path" || return 1; done <"$scratch/untracked"
        while IFS= read -r -d '' path; do
            [[ -z "${seen[$path]:-}" ]] || continue
            seen[$path]=1
            # A preview skips an indexed path the working tree no longer has.
            [[ "$mode" == gate || -e "$path" || -L "$path" ]] || continue
            record tracked "$path" || return 1
            if source_path "$path"; then sources+=("$path"); fi
        done <"$scratch/tracked"
        for path in "${sources[@]}"; do
            # Reader selection depends on the complete source inventory. Reject linked sources
            # and non-regular files before opening them.
            [[ -f "$path" && ! -L "$path" ]] || { why="$path is not a regular source file"; return 1; }
            content new "$path" "$path" || { why="$path cannot be read"; return 1; }
        done
        for path in "${modified[@]}"; do
            git cat-file blob "$merge_base:$path" >"$scratch/blob" 2>/dev/null &&
                content old "$path" "$scratch/blob" || { why="git cannot read $path at the merge-base"; return 1; }
        done
        record end "$records" || { why="the end record could not be written"; return 1; }
    }

    # THE CONSUMER: the selector's answer, line by line in its one order. Anything else is
    # malformed, and the gate then runs the whole lane.
    listed() {
        local want="$1" item
        shift
        for item in "$@"; do [[ "$item" != "$want" ]] || return 0; done
        return 1
    }
    parse_answer() {
        local -a lines terms
        local at=2 term reason_re='^reason [a-z][a-z0-9-]* .' counter_re='^counter (0|[1-9][0-9]{0,8}) (0|[1-9][0-9]{0,8})$'
        local module='[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*'
        local filter_re="^kind\\(lib\\) \\| kind\\(bin\\) \\| \\(binary_id\\(ae::it\\) & test\\(/\\^\\(($module(\\|$module)*)\\)::/\\)\\)\$"
        verdict="" doctests="" check="" not_selected="" total="" filterset=""
        reasons=() owners=()
        [[ -s "$1" && -z "$(tail -c 1 "$1")" ]] || { why="the answer is empty or does not end with a newline"; return 1; }
        mapfile -t lines <"$1"
        [[ "${lines[0]-}" == "version 1" ]] || { why="line 1 is not 'version 1'"; return 1; }
        case "${lines[1]-}" in
            "verdict whole") verdict=whole ;;
            "verdict subset") verdict=subset ;;
            *) why="line 2 is not a verdict"; return 1 ;;
        esac
        while [[ "${lines[at]-}" =~ $reason_re ]]; do reasons+=("${lines[at]#reason }"); at=$((at + 1)); done
        while [[ "${lines[at]-}" == "owner "?* ]]; do owners+=("${lines[at]#owner }"); at=$((at + 1)); done
        while [[ "${lines[at]-}" == "why "?* ]]; do at=$((at + 1)); done
        case "${lines[at]-}" in
            "doctests yes" | "doctests no") doctests="${lines[at]#doctests }" ;;
            *) why="line $((at + 1)) is not 'doctests yes|no'"; return 1 ;;
        esac
        at=$((at + 1))
        case "${lines[at]-}" in
            "check yes" | "check no") check="${lines[at]#check }" ;;
            *) why="line $((at + 1)) is not 'check yes|no'"; return 1 ;;
        esac
        at=$((at + 1))
        if [[ "$verdict" == subset ]]; then
            [[ "${lines[at]-}" =~ $counter_re ]] && ((BASH_REMATCH[1] <= BASH_REMATCH[2])) || { why="line $((at + 1)) is not 'counter <N> <M>' with N <= M"; return 1; }
            not_selected="${BASH_REMATCH[1]}"
            total="${BASH_REMATCH[2]}"
            at=$((at + 1))
            [[ "${lines[at]-}" == "filterset "?* ]] || { why="line $((at + 1)) is not 'filterset <filterset>'"; return 1; }
            filterset="${lines[at]#filterset }"
            # Every unit test, then the selected owners: the three guards and only owner lines.
            [[ "$filterset" =~ $filter_re ]] || { why="the filterset is not 'kind(lib) | kind(bin) | (binary_id(ae::it) & test(/^(<owner>|…)::/))'"; return 1; }
            IFS='|' read -r -a terms <<<"${BASH_REMATCH[1]}"
            for term in doors floor gate; do
                listed "$term" "${terms[@]}" || { why="the filterset leaves out the guard $term"; return 1; }
            done
            for term in "${terms[@]}"; do
                listed "$term" "${owners[@]}" || { why="the filterset names $term, which no owner line selects"; return 1; }
            done
            at=$((at + 1))
            ((${#reasons[@]} == 0)) || { why="a subset names a reason"; return 1; }
        else
            ((${#reasons[@]} > 0 && ${#owners[@]} == 0)) || { why="a whole verdict names no reason, or names owners"; return 1; }
        fi
        [[ "${lines[at]-}" == end ]] || { why="line $((at + 1)) is not 'end'"; return 1; }
        ((at + 1 == ${#lines[@]})) || { why="lines follow 'end'"; return 1; }
    }
    selector() {
        selector_status=0
        detached nice -n 10 cargo run --locked --quiet --all-features --example affected <"$scratch/stream" >"$scratch/answer" || selector_status=$?
    }

    if [[ "$mode" == preview ]]; then
        size_counts
        produce >"$scratch/stream" || refuse "$why"
        selector
        ((selector_status == 0)) || refuse "the selector exited with status $selector_status"
        parse_answer "$scratch/answer" || refuse "the selector's answer is malformed: $why"
        cat "$scratch/answer"
        exit 0
    fi

    identity || refuse "$why"
    start="$now"
    nest just rust-fmt-check
    nest just rust-lint
    nest just rust-fuzz-check
    size_counts
    fallback=""
    if produce >"$scratch/stream"; then
        selector
        if ((selector_status != 0)); then
            fallback="selector-exit: the selector exited with status $selector_status"
        elif ! parse_answer "$scratch/answer"; then
            fallback="selector-output: $why"
        fi
    else
        fallback="producer: $why"
    fi
    steady "during the selection"
    if [[ -n "$fallback" ]]; then
        # Without an answer nothing is known about the change: the whole lane, and the install
        # checks too, because `install` may be part of it.
        verdict=whole check=yes
        reasons=()
        echo "test-affected: whole: $fallback" >&2
    fi
    if [[ "$verdict" == whole ]]; then
        for reason in "${reasons[@]}"; do
            echo "test-affected: whole: ${reason%% *}: ${reason#* }" >&2
        done
        nest just _tmux-isolated test
    else
        echo "test-affected: subset: ${owners[*]}" >&2
        echo "test-affected: not selected: $not_selected of $total integration tests (M1 gap)" >&2
        echo "test-affected: $filterset" >&2
        nest just _tmux-isolated test -E "$filterset"
        if [[ "$doctests" == yes ]]; then
            # The doctests a filtered lane skips, sized and isolated as the release runs them.
            doc_dir="$(mktemp -d)"
            size_say "test-affected doctests"
            detached env -u TMUX -u TMUX_PANE TMPDIR="$doc_dir" TMUX_TMPDIR="$doc_dir" nice -n 10 cargo test --doc --locked --all-features -- --test-threads "$NEXTEST_TEST_THREADS"
        fi
    fi
    if [[ "$check" == yes ]]; then
        nest just check
    fi
    steady "while the tests ran"
    echo "test-affected: green ($verdict)" >&2

# ── Lane lock ────────────────────────────────────────────────────────

# One machine runs one gate at a time, so a slice takes the LANE LOCK before it
# runs `just test` and gives it back after. This recipe is that protocol, byte
# for byte what the hand-typed version did, so a holder on one spelling blocks
# a claimant on the other: the lock is the directory `<main checkout>/.local/locks/gate`
# recording its holder as one `owner` line, `<name> <UTC timestamp>`, which the recipe
# creates EXCLUSIVELY (noclobber, O_EXCL): that create is the claim, because `mkdir` is not
# atomic everywhere (uutils 0.10.0, Ubuntu 26.04, reports success for a directory that exists,
# so two claimants both "win" it). The hand protocol has no owner proof on such a host, the
# recipe has. `<main checkout>/.local/locks/next` is the queue, a file naming
# the one slice that may claim next (empty = anyone). The main checkout is
# resolved from `git rev-parse --git-common-dir`, so a linked worktree meets the
# same lock. `.local` is untracked: claim creates what it needs.
#
# WHICH RUNS A CLAIM OWNS (#343). After its owner create wins, a claim writes
# `<main checkout>/.local/locks/binding`: its exact owner line, then the canonical root
# (`pwd -P` of the toplevel) of the worktree it ran in. A run started without
# AE_LANE_CLAIM belongs to the claim only from that worktree; from another worktree it
# registers under no claim and never blocks the release. A run in another worktree
# that the claim did start names it: `AE_LANE_CLAIM=<name> just <recipe>`. A binding
# is proof only while its first line equals the current owner line; without one,
# every run counts as the holder's. Release removes the binding of its own claim.
#
# claim <name> [wait-secs]  take it, or with a wait poll every 30 s up to that
#                           many seconds, printing the holder; 0 = try once. Exit 1 when not taken.
# release <name>            give it back. Refused, lock untouched, when the owner line
#                           is not <name> (or not readable, or the directory holds anything
#                           but `owner`). It never removes a lock it cannot prove is yours.
#                           It works inside the directory it checked, so a release that stalls
#                           cannot remove the lock of whoever claims after it.
# cancel-wait <name>        stop a claim of that name that is still WAITING and that this
#                           machine's `claim` recorded when it began to wait. Never a holder, never
#                           a pid set, never a signal: see CANCEL-WAIT below. Exit 0 cancelled, 1 refused.
# status                    who holds it and how long; exit 0 free, 1 held.
#
# A `claim` and the `release` RECIPE (`just release`, not the `release` verb here, which never checks
# the disk, so a full disk cannot stop a holder giving its lock back) refuse, before any effect, when
# the checkout's, the shared lock directory's or cargo's volume is short of AE_LANE_MIN_FREE_GIB
# (default 20): a claim refused so takes no lock (DISKFREE).
#
# CANCEL-WAIT (ae-feedback 65). A waiting claim registers an incarnation directory
# `<main checkout>/.local/locks/waiting/w.XXXXXX` (mktemp: a path no later claim can recreate)
# holding exactly one state file, `waiting`, `acquiring` or `cancelled`, of one line
# `<name> <pid> <identity> <host>` (pid of the recipe shell, its `ps -o lstart=` letters and digits,
# `hostname`). The claim leaves WAITING with `mv waiting acquiring` before every attempt after the
# first; `cancel-wait` leaves it with `mv waiting cancelled`. One rename path, so exactly one
# of them wins: after a cancel wins, that claim can never reach `acquiring`, and a claim that has
# reached it can no longer be cancelled. Either rename refuses, moving nothing, when its source is
# not a plain file or its destination exists in any form. The claim sleeps in whole seconds and looks
# for `cancelled` after each, so a cancel ends it within about a second while the attempt, and the
# printed `waiting` line, keep the 30 s poll. A claim that cannot publish its record, or publish it
# again after an attempt, stops with a refusal rather than wait where nothing can cancel it.
# `cancel-wait` validates the one record naming <name> BEFORE it moves anything: the incarnation
# holds exactly one plain state file of exactly one well formed line, this host recorded it, its
# pid is alive with the recorded identity, the gate holder is another name, the state is `waiting`.
# Any doubt refuses and leaves it alone. A record whose process is provably gone, in any state, is
# stale: `cancel-wait` removes it and never counts it. Concurrent claims of the same name may
# coexist (the owner line still decides acquisition); `cancel-wait` refuses when more than one live
# record names it. A SIGKILLed claim leaves such a stale directory.
#
# NEVER AUTO-STEAL, NEVER AUTO-BREAK. A lock whose holder is gone is REPORTED
# ("suspect") and left standing: a human or the lead clears it, by hand, after
# asking the holder. A stale-lock cleanup once removed an ACTIVE gate. Age alone
# cannot prove a holder dead, so nothing here acts on it. The queue check and the
# claim are two steps, as they were by hand: `next` is a courtesy, the owner create is the lock.

# Claim, release or inspect the gate lane lock: claim NAME [WAIT-SECS] | release NAME | cancel-wait NAME | status
[no-exit-message]
lane-lock verb name="" wait="0":
    #!/usr/bin/env bash
    set -euo pipefail
    {{ DISKFREE }}
    verb="$1"
    name="${2:-}"
    wait="${3:-0}"
    poll=30
    suspect_secs=3600
    usage() { echo "usage: just lane-lock claim <name> [wait-secs] | release <name> | cancel-wait <name> | status" >&2; exit 2; }
    refuse() { echo "lane-lock: refused: $*" >&2; exit 1; }
    case "$verb" in
        claim | release | cancel-wait) [[ -n "$name" ]] || usage ;;
        status) ;;
        *) usage ;;
    esac
    if [[ -n "$name" && ! "$name" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$ ]]; then
        echo "lane-lock: refused: a lock name is [A-Za-z0-9][A-Za-z0-9_.-]{0,63}, got '$name'" >&2
        exit 2
    fi
    # Whole seconds without a leading zero: bash would read 08 as an invalid octal number.
    [[ "$wait" =~ ^(0|[1-9][0-9]{0,5})$ ]] || { echo "lane-lock: refused: wait is whole seconds without a leading zero, got '$wait'" >&2; exit 2; }
    common="$(git rev-parse --git-common-dir 2>/dev/null)" || refuse "not inside a git checkout"
    common="$(cd -P "$common" && pwd -P)"
    [[ "${common##*/}" == .git ]] || refuse "the git common dir $common is not <checkout>/.git"
    root="${common%/.git}"
    locks="$root/.local/locks"
    gate="$locks/gate"
    owner_file="$gate/owner"
    next_file="$locks/next"
    # Seconds since the UTC timestamp $1, or nothing when it does not parse. BSD date first, then GNU.
    age_of() {
        local since
        since="$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$1" +%s 2>/dev/null || date -u -d "$1" +%s 2>/dev/null || true)"
        [[ "$since" =~ ^[0-9]+$ ]] || return 0
        echo $(( $(date +%s) - since ))
    }
    # The inode of directory $1. `ls -di` is the one spelling BSD and GNU share (`stat` differs).
    # shellcheck disable=SC2012
    inode_of() { ls -di "$1" | awk '{print $1}'; }
    # The claim: the owner line created EXCLUSIVELY. Noclobber makes bash open it with O_EXCL; the
    # subshell keeps it from reaching anything else in the recipe. Fails when the line exists.
    claim_owner() {
        claim_line="$name $(date -u +%FT%TZ)"
        ( set -C; printf '%s\n' "$claim_line" >"$owner_file" ) 2>/dev/null
    }
    binding_file="$locks/binding"
    # The binding of the claim just won (#343): its owner line and the canonical root of this
    # worktree, by temp file and rename. Run only by the holder, so no other claim can race it.
    # A binding that cannot be written must not outlive this claim with an older owner line:
    # without one, every run counts as this claim's, never as free.
    bind_claim() {
        local top tmp="$locks/.binding.$$"
        if top="$(git rev-parse --show-toplevel 2>/dev/null)" && top="$(cd -P "$top" && pwd -P)" &&
            [[ "$top" == /* && "$top" != *$'\n'* && ! -d "$binding_file" ]] &&
            printf '%s\n%s\n' "$claim_line" "$top" >"$tmp" 2>/dev/null && mv "$tmp" "$binding_file" 2>/dev/null; then
            return 0
        fi
        rm "$tmp" 2>/dev/null || true
        [[ ! -f "$binding_file" || -L "$binding_file" ]] || rm "$binding_file" 2>/dev/null || true
        echo "note: $binding_file could not be written: every run while $name holds the lane counts as its own"
    }
    span() {
        local s=$1
        (( s >= 0 )) || s=0
        if (( s >= 3600 )); then printf '%dh%02dm' $((s / 3600)) $((s % 3600 / 60))
        elif (( s >= 60 )); then printf '%dm%02ds' $((s / 60)) $((s % 60))
        else printf '%ds' "$s"; fi
    }
    # Fills h_line (the owner line, "" when unrecorded), h_name, h_age (seconds, "" when unknown).
    read_holder() {
        local ts
        h_line=""
        h_name=""
        h_age=""
        if [[ -f "$owner_file" && ! -L "$owner_file" ]]; then
            h_line="$(head -n 1 "$owner_file" 2>/dev/null || true)"
        fi
        h_name="${h_line%% *}"
        if [[ "$h_line" == *" "* ]]; then
            ts="${h_line#* }"
            h_age="$(age_of "${ts%% *}")"
        fi
    }
    # One line naming the holder. Needs read_holder first.
    holder_words() {
        if [[ -z "$h_name" ]]; then
            echo "held, but no owner recorded (a claim is mid-write, or one died before writing it)"
        elif [[ -n "$h_age" ]]; then
            echo "held by $h_line (age $(span "$h_age"))"
        else
            echo "held by $h_line (age unknown)"
        fi
    }
    waiting="$locks/waiting"
    # A process's start identity: its `ps -o lstart=` start time, keeping the ASCII letters and digits
    # (the same reading `_tmux-isolated` and tests/it/parity.rs make).
    proc_identity() { LC_ALL=C ps -o lstart= -p "$1" 2>/dev/null | LC_ALL=C tr -cd 'A-Za-z0-9'; }
    # claimant_state PID IDENTITY: alive when the pid is the recorded process, gone when no such
    # process exists or the pid now has another identity, unsure when it exists but cannot be read.
    claimant_state() {
        local pid="$1" ident="$2" error current
        if error="$(LC_ALL=C kill -0 "$pid" 2>&1)" || [[ "$error" != *"No such process"* ]]; then
            current="$(proc_identity "$pid" || true)"
            if [[ -z "$current" ]]; then echo unsure; elif [[ "$current" == "$ident" ]]; then echo alive; else echo gone; fi
        else
            echo gone
        fi
    }
    # The incarnation of a claim that is waiting (see CANCEL-WAIT above): its directory, empty until
    # the claim first waits, and the state this claim last published in it, `waiting` or `acquiring`.
    inc=""
    inc_state=""
    inc_remove() {
        [[ -n "${1:-}" ]] || return 0
        rm -f "$1/waiting" "$1/acquiring" "$1/cancelled" "$1/.new" 2>/dev/null || true
        rmdir "$1" 2>/dev/null || true
    }
    # state_move DIR FROM TO: the one rename of the protocol, on a state file of DIR. It moves nothing
    # unless FROM is a plain file and TO does not exist in any form (a directory named TO would
    # otherwise take the file in), so a conflicting state refuses, never half-moves.
    state_move() {
        [[ -f "$1/$2" && ! -L "$1/$2" && ! -e "$1/$3" && ! -L "$1/$3" ]] || return 1
        mv "$1/$2" "$1/$3" 2>/dev/null
    }
    # Enter or re-enter WAITING. The first time the claim creates its incarnation (state file written
    # whole, then renamed into place); after an attempt that did not win, `acquiring` goes back to
    # `waiting`. A claim that cannot publish that state does not wait: it stops, since nothing could
    # cancel it while it sleeps and then takes the lock.
    wait_register() {
        local ident host
        if [[ -n "$inc" ]]; then
            state_move "$inc" acquiring waiting || refuse "this claim could not publish its waiting record again under $inc; it stops rather than wait where nothing can cancel it"
            inc_state=waiting
            return 0
        fi
        ident="$(proc_identity $$ || true)"
        host="$(hostname 2>/dev/null || true)"
        if [[ -n "$ident" && "$host" =~ ^[A-Za-z0-9._-]+$ ]] &&
            mkdir -p "$waiting" 2>/dev/null &&
            [[ -d "$waiting" && ! -L "$waiting" ]] &&
            inc="$(mktemp -d "$waiting/w.XXXXXX" 2>/dev/null)" &&
            printf '%s %s %s %s\n' "$name" "$$" "$ident" "$host" >"$inc/.new" 2>/dev/null &&
            mv "$inc/.new" "$inc/waiting" 2>/dev/null; then
            inc_state=waiting
            return 0
        fi
        inc_remove "$inc"
        inc=""
        refuse "this claim could not register under $waiting (its process identity or the host name cannot be read, or the directory cannot be written); it stops rather than wait where nothing can cancel it"
    }
    # WAITING to ACQUIRING before an attempt. A failed rename means the record was taken (a cancel won)
    # or is gone or conflicts: either way no attempt is made.
    wait_leave() {
        [[ -n "$inc" && "$inc_state" == waiting ]] || return 0
        if ! state_move "$inc" waiting acquiring; then
            if [[ -f "$inc/cancelled" && ! -L "$inc/cancelled" ]]; then
                echo "lane-lock: cancelled: the waiting claim of $name was cancelled; nothing was claimed" >&2
                exit 1
            fi
            refuse "the waiting record of this claim is gone or conflicts under $inc; nothing was claimed"
        fi
        inc_state=acquiring
    }
    # Sleep whole seconds, looking for a cancel after each.
    nap() {
        local left="$1"
        while ((left > 0)); do
            sleep 1
            left=$((left - 1))
            if [[ -n "$inc" && -f "$inc/cancelled" && ! -L "$inc/cancelled" ]]; then
                echo "lane-lock: cancelled: the waiting claim of $name was cancelled; nothing was claimed" >&2
                exit 1
            fi
        done
    }
    # read_record DIR: DIR must be a plain directory holding exactly one entry, a plain file named
    # `waiting`, `acquiring` or `cancelled`, whose whole content is exactly one line
    # `<name> <pid> <identity> <host>`. Returns 0 with r_state, r_name, r_pid, r_ident and r_host set;
    # otherwise 1 with the reason in r_why. Nothing is moved or removed.
    read_record() {
        local dir="$1" entry count=0 line
        local re='^([A-Za-z0-9][A-Za-z0-9_.-]{0,63}) ([0-9]+) ([A-Za-z0-9]+) ([A-Za-z0-9._-]+)$'
        r_why=""
        r_state=""
        r_name=""
        r_pid=""
        r_ident=""
        r_host=""
        [[ -d "$dir" && ! -L "$dir" ]] || { r_why="it is not a plain directory"; return 1; }
        for entry in "$dir"/* "$dir"/.[!.]* "$dir"/..?*; do
            [[ -e "$entry" || -L "$entry" ]] || continue
            count=$((count + 1))
            r_state="${entry##*/}"
        done
        ((count == 1)) || { r_why="it holds $count entries, not exactly one state file"; return 1; }
        case "$r_state" in
            waiting | acquiring | cancelled) ;;
            *) r_why="its one entry is not a state file"; return 1 ;;
        esac
        [[ -f "$dir/$r_state" && ! -L "$dir/$r_state" ]] || { r_why="its state is not a plain file"; return 1; }
        line="$(cat "$dir/$r_state" 2>/dev/null)" || { r_why="its state cannot be read"; return 1; }
        if [[ "$line" == *$'\n'* ]] || ! printf '%s\n' "$line" | cmp -s - "$dir/$r_state"; then
            r_why="its state is not exactly one line"
            return 1
        fi
        [[ "$line" =~ $re ]] || { r_why="its line is not well formed"; return 1; }
        r_name="${BASH_REMATCH[1]}"
        r_pid="${BASH_REMATCH[2]}"
        r_ident="${BASH_REMATCH[3]}"
        r_host="${BASH_REMATCH[4]}"
    }
    case "$verb" in
        status)
            if [[ ! -e "$gate" && ! -L "$gate" ]]; then
                echo "gate: free"
                state=0
            else
                if [[ -d "$gate" && ! -L "$gate" ]]; then
                    read_holder
                    echo "gate: $(holder_words)"
                    if [[ -z "$h_name" ]]; then
                        echo "suspect: nobody is recorded as holding it"
                    elif [[ -n "$h_age" ]] && (( h_age > suspect_secs )); then
                        echo "suspect: held longer than $(span "$suspect_secs"); the holder may be gone"
                    fi
                    if [[ -f "$gate/suspect" && ! -L "$gate/suspect" ]]; then cat "$gate/suspect"; fi
                else
                    echo "gate: $gate exists and is not a directory"
                    echo "suspect: not a lock this protocol made"
                fi
                echo "lane-lock does not clear a lock: ask the holder, then the lead or a human removes it by hand"
                state=1
            fi
            if [[ -s "$next_file" ]]; then echo "next: $(cat "$next_file")"; else echo "next: none"; fi
            exit "$state"
            ;;
        release)
            [[ -d "$gate" && ! -L "$gate" ]] || refuse "no gate lock is held, nothing to release"
            # ANCHORED TO THE DIRECTORY ENTERED HERE, never to the path. A release can stall between
            # its owner check and its removal while the lock is released (by this recipe or by the
            # hand-typed protocol, which takes no part in anything below) and another slice claims
            # the same path. Standing inside the directory it checked, the stalled release meets a
            # directory that is gone: its relative `rm owner` finds nothing and it stops before the
            # path `rmdir`. A tiny window remains and is accepted: between the last identity check
            # and `rmdir`, a gate could be removed and re-made empty, and the claim writes its
            # owner line only after its own `mkdir`.
            cd -P "$gate" 2>/dev/null || refuse "could not enter $gate"
            entered="$(inode_of .)"
            owner_file=owner
            read_holder
            [[ -n "$h_name" ]] || refuse "the gate has no owner line, so it is not provably $name's; lock left intact"
            [[ "$h_name" == "$name" ]] || refuse "the gate is $(holder_words), not $name's; lock left intact"
            extra=""
            for entry in ./* ./.[!.]* ./..?*; do
                [[ -e "$entry" || -L "$entry" ]] || continue
                [[ "${entry##*/}" == owner ]] || extra="$extra ${entry##*/}"
            done
            [[ -z "$extra" ]] || refuse "the gate holds more than its owner line ($extra ); lock left intact"
            # THE RUN CENSUS (#340): every process a gate started under this claim must have ended. An
            # entry of this claim that is pending, malformed or still running, or a census that fails,
            # keeps the lock: the reasons go to <gate>/suspect, and from then on every release refuses
            # on its own until a human clears it (docs/development.md, "The gate process registry").
            # Only an absent registry skips the census: one that is a link, not a directory or not
            # readable is checked, and the check refuses it.
            if [[ -e "$locks/groups" || -L "$locks/groups" ]]; then
                census=0
                verdict="$(perl -e '{{ REAP }}' check "$locks/groups" "$h_line" 2>&1)" || census=$?
                if ((census != 0)); then
                    [[ -n "$verdict" ]] || verdict="suspect: groups: the run census exited with status $census"
                    printf '%s\n' "$verdict" >>suspect
                    refuse "a process this claim started may still run; the lock stays held: $(head -n 1 suspect)"
                fi
            fi
            # The binding of this claim goes while the lock is still held, so no later claim can race
            # it, and a re-claim of the same name in the same second never meets it (#343).
            if [[ -f "$binding_file" && ! -L "$binding_file" && "$(head -n 1 "$binding_file" 2>/dev/null)" == "$h_line" ]]; then
                rm "$binding_file" 2>/dev/null || echo "note: $binding_file could not be removed"
            fi
            err="$(rm owner 2>&1)" || refuse "the owner line could not be removed ($err); the gate this release checked is gone or changed, so nothing else was removed"
            [[ "$(inode_of "$gate" 2>/dev/null)" == "$entered" ]] || refuse "the owner line is removed, but $gate is no longer the directory this release entered; left alone"
            rmdir "$gate" || refuse "the owner line is removed, but $gate could not be removed"
            echo "lane-lock: released by $name"
            ;;
        cancel-wait)
            # Validate first, move second (CANCEL-WAIT above). No signal is sent and no process table is read.
            here="$(hostname 2>/dev/null || true)"
            found=0
            pruned=0
            rec=""
            if [[ -d "$waiting" && ! -L "$waiting" ]]; then
                for dir in "$waiting"/w.??????; do
                    [[ -d "$dir" && ! -L "$dir" ]] || continue
                    # A directory belongs to <name> when a state file of it opens with that name.
                    named=0
                    for state in waiting acquiring cancelled; do
                        [[ -f "$dir/$state" && ! -L "$dir/$state" ]] || continue
                        line="$(head -n 1 "$dir/$state" 2>/dev/null || true)"
                        [[ "${line%% *}" != "$name" ]] || named=1
                    done
                    ((named)) || continue
                    # A well formed record of this host whose process is provably gone is stale in any
                    # state: it is removed and never counted, so it cannot make a live claim ambiguous.
                    if read_record "$dir" && [[ "$r_host" == "$here" && "$(claimant_state "$r_pid" "$r_ident")" == gone ]]; then
                        inc_remove "$dir"
                        pruned=$((pruned + 1))
                        continue
                    fi
                    found=$((found + 1))
                    rec="$dir"
                done
            fi
            if ((found == 0)); then
                if ((pruned > 0)); then
                    echo "lane-lock: the claim of $name is gone ($pruned stale record(s) removed); nothing was cancelled" >&2
                    exit 1
                fi
                refuse "no claim named $name is waiting"
            fi
            ((found == 1)) || refuse "$found claims named $name are waiting; cancel-wait cannot tell which one to stop, and moved nothing"
            read_record "$rec" || refuse "the record of $name in $rec cannot be used ($r_why); left alone"
            [[ "$r_name" == "$name" ]] || refuse "the record in $rec names $r_name, not $name; left alone"
            [[ "$r_host" == "$here" ]] || refuse "the claim of $name was recorded on host $r_host, not on this machine ($here); left alone"
            case "$(claimant_state "$r_pid" "$r_ident")" in
                alive) ;;
                gone)
                    inc_remove "$rec"
                    echo "lane-lock: the claim of $name (pid $r_pid) is gone; its stale record is removed, nothing was cancelled" >&2
                    exit 1
                    ;;
                *) refuse "pid $r_pid of the claim of $name cannot be read, so it is not provably that claim; left alone" ;;
            esac
            read_holder
            [[ "$h_name" != "$name" ]] || refuse "$name holds the gate; cancel-wait never touches a holder"
            case "$r_state" in
                acquiring) refuse "the claim of $name (pid $r_pid) is acquiring now and cannot be cancelled; nothing moved" ;;
                cancelled) refuse "the claim of $name (pid $r_pid) is already cancelled and ending; nothing moved" ;;
            esac
            # THE DECISION: one rename. It succeeds only while the claim is still WAITING.
            state_move "$rec" waiting cancelled || refuse "the claim of $name is no longer waiting, or its state conflicts (it began to acquire, or ended); nothing was cancelled"
            tries=10
            while [[ -d "$rec" ]] && ((tries-- > 0)); do
                if [[ "$(claimant_state "$r_pid" "$r_ident")" == gone ]]; then
                    inc_remove "$rec"
                    break
                fi
                sleep 1
            done
            if [[ -d "$rec" ]]; then
                echo "lane-lock: cancel decided for $name, but pid $r_pid has not ended after 10 s; it cannot take the lock" >&2
                exit 1
            fi
            echo "lane-lock: cancelled the waiting claim of $name (pid $r_pid); it will not claim"
            ;;
        claim)
            disk_floor lane-lock "$locks"
            mkdir -p "$locks"
            start="$(date +%s)"
            vanished=0
            attempt=0
            # The incarnation of a waiting claim goes with the claim, on every way out; TERM, INT and HUP
            # are exits so that it does (SIGKILL leaves a stale one, which cancel-wait consumes).
            claim_cleanup() {
                local status=$?
                inc_remove "$inc"
                exit "$status"
            }
            trap claim_cleanup EXIT
            trap 'exit 143' TERM
            trap 'exit 130' INT
            trap 'exit 129' HUP
            while :; do
                if ((attempt++ > 0)); then
                    disk_floor lane-lock "$locks"
                    wait_leave
                fi
                if [[ ! -s "$next_file" || "$(cat "$next_file")" == "$name" ]]; then
                    # A standing gate is held and is not asked about: a `mkdir` that is not atomic reports
                    # success for it. Two claimants can still both pass this look and both `mkdir`; the
                    # exclusive owner create picks one, and the other reads it like any holder.
                    made=skip
                    if [[ ! -e "$gate" && ! -L "$gate" ]]; then
                        if err="$(mkdir "$gate" 2>&1)"; then made=yes; else made=no; fi
                    fi
                    if [[ "$made" == yes ]] && claim_owner; then
                        echo "lane-lock: claimed by $name"
                        bind_claim
                        [[ ! -s "$next_file" ]] || echo "note: $next_file still names $(cat "$next_file"); the lead clears it"
                        exit 0
                    fi
                    if [[ ! -e "$gate" && ! -L "$gate" ]]; then
                        [[ "$made" != no ]] || refuse "could not create $gate: $err"
                        # Released between the look or `mkdir` and the owner create: the lock is free, try again.
                        (( ++vanished <= 5 )) || refuse "$gate keeps vanishing under this claim"
                        continue
                    fi
                    if [[ -d "$gate" && ! -L "$gate" ]]; then
                        read_holder
                        [[ "$h_name" != "$name" ]] || refuse "$name already holds the gate ($h_line); release it first, or this is another session on the same name"
                        why="gate $(holder_words)"
                    else
                        why="$gate exists and is not a directory"
                    fi
                else
                    why="queue: next is $(cat "$next_file"), not $name"
                fi
                left=$(( wait - ($(date +%s) - start) ))
                if (( left <= 0 )); then
                    echo "lane-lock: not claimed: $why" >&2
                    exit 1
                fi
                wait_register
                echo "lane-lock: waiting, ${left}s left: $why" >&2
                if (( left < poll )); then nap "$left"; else nap "$poll"; fi
            done
            ;;
    esac

# ── Version ──────────────────────────────────────────────────────────

# Show current version. Since slice Z3 the crate is the ONLY place a version
# word lives — the wrapper that held the other half of the pair is deleted.
version:
    @awk '/^\[/ { in_package = ($0 == "[package]") } in_package && /^version = "/ { gsub(/^version = "|"$/, ""); print; exit }' Cargo.toml

# Restore a previously interrupted CalVer bump from durable backups.
bump-recover:
    #!/usr/bin/env bash
    set -euo pipefail
    RECOVERY_DIR=".ae-bump-recovery"
    paths=(Cargo.toml Cargo.lock)
    staged=()
    if [[ ! -d "$RECOVERY_DIR" ]]; then
        echo "Error: ${RECOVERY_DIR} is not present; nothing to recover" >&2
        exit 1
    fi
    if [[ ! -f "$RECOVERY_DIR/backups-ready" ]]; then
        echo "Error: ${RECOVERY_DIR}/backups-ready is missing; marker retained and live files unchanged" >&2
        exit 1
    fi

    cleanup() {
        local status="$?"
        local restore_tmp
        for restore_tmp in "${staged[@]}"; do
            if [[ -n "$restore_tmp" && -e "$restore_tmp" ]] && ! rm -f "$restore_tmp"; then
                status=1
            fi
        done
        return "$status"
    }
    trap cleanup EXIT

    for index in "${!paths[@]}"; do
        path="${paths[$index]}"
        name="${path##*/}"
        backup="$RECOVERY_DIR/${name}.orig"
        if [[ ! -f "$backup" ]]; then
            echo "Error: missing ${backup}; marker retained and live files unchanged" >&2
            exit 1
        fi
        if ! restore_tmp="$(mktemp "${path}.bump-recover.XXXXXX")"; then
            echo "Error: could not stage ${path}; marker retained and live files unchanged" >&2
            exit 1
        fi
        staged+=("$restore_tmp")
        if ! cp -p "$backup" "$restore_tmp"; then
            echo "Error: could not stage ${path}; marker retained and live files unchanged" >&2
            exit 1
        fi
        if ! cmp -s "$restore_tmp" "$backup"; then
            echo "Error: staged restore verification failed for ${path}; marker retained and live files unchanged" >&2
            exit 1
        fi
    done

    for index in "${!paths[@]}"; do
        path="${paths[$index]}"
        if ! mv "${staged[$index]}" "$path"; then
            echo "Error: could not restore ${path}; marker retained" >&2
            exit 1
        fi
    done

    if ! rm -rf "$RECOVERY_DIR"; then
        echo "Error: restored files but could not remove ${RECOVERY_DIR}; marker retained" >&2
        exit 1
    fi
    trap - EXIT

# Compute next release version using SemVer-compatible CalVer: YYYY.M.N.
# The sequence is tag-derived, so a stale working tree version cannot cause a
# duplicate publication. Version files use durable backups and recover-or-refuse
# publication; stdout remains the VERSION-only contract for just release.
#
# Z3: it owns Cargo.toml and Cargo.lock, and nothing else. There is no second
# version word to move in step — `ae-entry` was deleted with the rest of the
# product's bash, and with it the whole class of "the pair disagrees" failure.
bump:
    #!/usr/bin/env bash
    set -euo pipefail
    RECOVERY_DIR=".ae-bump-recovery"
    if [[ -e "$RECOVERY_DIR" ]]; then
        echo "Error: stale ${RECOVERY_DIR} exists; recover before starting another bump." >&2
        echo "run just bump-recover." >&2
        exit 1
    fi

    YEAR_MONTH="$(date -u +%Y).$((10#$(date -u +%m)))"
    TAG_PREFIX="v${YEAR_MONTH}."
    TAG_RE="^v${YEAR_MONTH//./\\.}\.([0-9]+)$"
    MAX=0
    while IFS= read -r tag; do
        if [[ "$tag" =~ $TAG_RE ]]; then
            sequence="${BASH_REMATCH[1]}"
            if ((10#$sequence > MAX)); then
                MAX=$((10#$sequence))
            fi
        fi
    done < <(git tag --list "${TAG_PREFIX}*")

    VERSION="${YEAR_MONTH}.$((MAX + 1))"
    TAG="v${VERSION}"
    if git show-ref --verify --quiet "refs/tags/${TAG}"; then
        echo "Error: release tag ${TAG} already exists" >&2
        exit 1
    fi

    TMP_DIR="$(mktemp -d .ae-bump.XXXXXX)"
    if ! mkdir "$RECOVERY_DIR"; then
        rm -rf "$TMP_DIR"
        echo "Error: could not create ${RECOVERY_DIR}" >&2
        exit 1
    fi

    recover() {
        local path name restore_tmp recovery_rc=0
        if [[ ! -f "$RECOVERY_DIR/backups-ready" ]]; then
            rm -f "$RECOVERY_DIR"/*.orig "$RECOVERY_DIR"/*.orig.tmp.* "$RECOVERY_DIR"/backups-ready.tmp.* || true
            return 0
        fi
        for path in Cargo.toml Cargo.lock; do
            name="${path##*/}"
            if [[ ! -f "$RECOVERY_DIR/${name}.orig" ]]; then
                recovery_rc=1
                continue
            fi
            restore_tmp="$RECOVERY_DIR/${name}.restore.$$"
            if ! cp -p "$RECOVERY_DIR/${name}.orig" "$restore_tmp" ||
                ! mv "$restore_tmp" "$path"; then
                rm -f "$restore_tmp" || true
                recovery_rc=1
            fi
        done
        return "$recovery_rc"
    }
    cleanup() {
        local status="$?"
        local recovery_status=0
        # An interrupted or failed publication restores every backed-up file
        # before either temporary data or the durable recovery marker is removed.
        if [[ "${BUMP_PUBLISHED:-0}" != 1 ]]; then
            recover || recovery_status="$?"
        fi
        rm -rf "$TMP_DIR" || recovery_status=1
        if ((recovery_status == 0)); then
            rm -rf "$RECOVERY_DIR" || recovery_status=1
        fi
        if ((recovery_status != 0)); then
            echo "Error: bump recovery failed; ${RECOVERY_DIR} retained for manual recovery" >&2
            return "$recovery_status"
        fi
        return "$status"
    }
    trap cleanup EXIT
    backup_files() {
        local path name backup_tmp
        for path in Cargo.toml Cargo.lock; do
            name="${path##*/}"
            backup_tmp="$RECOVERY_DIR/${name}.orig.tmp.$$"
            cp -p "$path" "$backup_tmp"
            mv "$backup_tmp" "$RECOVERY_DIR/${name}.orig"
        done
        for path in Cargo.toml Cargo.lock; do
            name="${path##*/}"
            if ! cmp -s "$RECOVERY_DIR/${name}.orig" "$path"; then
                echo "Error: backup verification failed for ${path}" >&2
                return 1
            fi
        done
        : >"$RECOVERY_DIR/backups-ready.tmp.$$"
        mv "$RECOVERY_DIR/backups-ready.tmp.$$" "$RECOVERY_DIR/backups-ready"
    }
    backup_files
    for path in Cargo.toml Cargo.lock; do
        name="${path##*/}"
        cp -p "$path" "$TMP_DIR/$name"
    done

    cp -p "$TMP_DIR/Cargo.toml" "$TMP_DIR/Cargo.toml.next"
    awk -v version="$VERSION" '
        !done && /^version = "/ { print "version = \"" version "\""; done=1; next }
        { print }
    ' "$TMP_DIR/Cargo.toml" >"$TMP_DIR/Cargo.toml.next"
    mv "$TMP_DIR/Cargo.toml.next" "$TMP_DIR/Cargo.toml"
    cp -p "$TMP_DIR/Cargo.lock" "$TMP_DIR/Cargo.lock.next"
    awk -v version="$VERSION" '
        function flush(    i, name, source, versions, replaced) {
            if (n == 0) return
            name = ""
            source = 0
            versions = 0
            for (i = 1; i <= n; i++) {
                if (lines[i] == "name = \"ae\"") name = "ae"
                if (lines[i] ~ /^source = /) source = 1
                if (lines[i] ~ /^version = "/) versions++
            }
            if (name == "ae" && !source) {
                roots++
                if (versions != 1) invalid = 1
                if (roots == 1) {
                    replaced = 0
                    for (i = 1; i <= n; i++) {
                        if (!replaced && lines[i] ~ /^version = "/) {
                            print "version = \"" version "\""
                            replaced = 1
                        } else {
                            print lines[i]
                        }
                    }
                    if (!replaced) invalid = 1
                } else {
                    for (i = 1; i <= n; i++) print lines[i]
                }
            } else {
                for (i = 1; i <= n; i++) print lines[i]
            }
            delete lines
            n = 0
        }
        /^\[\[package\]\]$/ {
            flush()
            lines[++n] = $0
            next
        }
        { lines[++n] = $0 }
        END {
            flush()
            if (roots != 1 || invalid) exit 1
        }
    ' "$TMP_DIR/Cargo.lock" >"$TMP_DIR/Cargo.lock.next"
    mv "$TMP_DIR/Cargo.lock.next" "$TMP_DIR/Cargo.lock"

    grep -q '^version = "'"$VERSION"'"$' "$TMP_DIR/Cargo.toml"
    awk -v version="$VERSION" '
        function flush(    i, name, source) {
            if (n == 0) return
            name = ""
            source = 0
            for (i = 1; i <= n; i++) {
                if (lines[i] == "name = \"ae\"") name = "ae"
                if (lines[i] ~ /^source = /) source = 1
            }
            if (name == "ae" && !source) {
                roots++
                for (i = 1; i <= n; i++) {
                    if (lines[i] == "version = \"" version "\"") matches++
                }
            }
            delete lines
            n = 0
        }
        /^\[\[package\]\]$/ {
            flush()
            lines[++n] = $0
            next
        }
        { lines[++n] = $0 }
        END {
            flush()
            exit !(roots == 1 && matches == 1)
        }
    ' "$TMP_DIR/Cargo.lock"

    for path in Cargo.toml Cargo.lock; do
        name="${path##*/}"
        if ! mv "$TMP_DIR/$name" "$path"; then
            echo "Error: could not publish ${path}; recover-or-refuse marker retained if recovery fails" >&2
            exit 1
        fi
    done
    BUMP_PUBLISHED=1
    trap - EXIT
    rm -rf "$TMP_DIR" "$RECOVERY_DIR"
    printf '%s\n' "$VERSION"

# ── Changelog ────────────────────────────────────────────────────────

# The commits git-cliff must not see: every commit reachable from HEAD that is NOT on the
# first-parent chain — the work commits a merge absorbed. WHY THIS EXISTS: git-cliff
# cannot express a first-parent walk. No `--first-parent` flag and no `[git]` key in
# v2.13.1 or current main, source grep `first_parent|first-parent|simplify_first` = 0 hits
# in both tarballs, and libgit2's simplify_first_parent is never called. The exact
# complement is computed here with git and handed to git-cliff through its own supported
# `--skip-commit`, which matches FULL 40-char ids. Re-verify the negative with:
#   curl -fsSL https://github.com/orhun/git-cliff/archive/refs/tags/v2.13.1.tar.gz | tar -xz
#   grep -rniE 'first_parent|first-parent|simplify_first' git-cliff-2.13.1/   # 0 hits
#
# The anchor is HEAD, the same universe a range-less git-cliff walk uses. NEVER `--all`:
# that reaches unmerged branches, so the output would depend on which worktree branches
# happen to exist on the machine and would differ run to run.
#
# Failure is LOUD, never short. A missing sha silently turns absorbed work back into
# changelog entries that still look normal, so the list is reconciled against two
# independent rev-list counts, every token must be a full-length lowercase hex sha, and
# the list is bounded before anything is printed.
_cliff-skip:
    #!/usr/bin/env bash
    set -euo pipefail

    # comm is the SET difference, not a rev-list reachability exclusion: `--not <chain>`
    # excludes the chain's whole ancestry, which already contains every absorbed commit.
    all=$(git rev-list HEAD)
    chain=$(git rev-list --first-parent HEAD)
    skip=$(comm -23 <(printf '%s\n' "$all" | sort) <(printf '%s\n' "$chain" | sort) | tr '\n' ' ')

    expected=$(( $(git rev-list HEAD | wc -l) - $(git rev-list --first-parent HEAD | wc -l) ))
    have=$(printf '%s' "$skip" | wc -w | tr -d '[:space:]')
    if [ "$have" -ne "$expected" ]; then
        echo "error: changelog skip list is short: $have of $expected off-chain commits" >&2
        exit 1
    fi
    for sha in $skip; do
        if [ "${#sha}" -ne 40 ]; then
            echo "error: changelog skip list holds a non-40-char sha: $sha" >&2
            exit 1
        fi
        case "$sha" in
            *[!0-9a-f]*) echo "error: changelog skip list holds a non-hex sha: $sha" >&2; exit 1 ;;
        esac
    done
    if [ "${#skip}" -gt 65536 ]; then
        echo "error: changelog skip list is ${#skip} bytes, past the 64KiB bound" >&2
        exit 1
    fi
    printf '%s\n' "$skip"

# The ONE owner of the text-to-argv boundary, and the ONE place `git-cliff` is invoked in
# this file — tests/it/gate.rs asserts both structurally, so a direct call added anywhere
# else fails by construction. `set --` prepends the skip flag and its UNQUOTED values to
# the caller's own argv: N shas become N argv entries, and an empty list adds no flag at
# all. A quoted crossing (one joined argument, silently ignored by git-cliff) is the
# failure the boundary test executes these recipes to refuse. Why the list exists and what
# keeps it complete: `_cliff-skip` above.
#
# The skip list comes through `nest_capture` and git-cliff is exec'd, so a TERM from just reaches
# whichever of the two runs (#345).
_cliff-run +args:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ NEST }}
    nest_capture skip just _cliff-skip
    if [ -n "$skip" ]; then
        set -- --skip-commit $skip "$@"
    fi
    exec git-cliff "$@"

# Generate full CHANGELOG.md from git history (skip list: `_cliff-skip`; boundary: `_cliff-run`)
changelog:
    just _cliff-run -o CHANGELOG.md

# ── Release ──────────────────────────────────────────────────────────

# THE WHOLE RELEASE HAPPENS HERE, on this machine. GitHub Actions is no longer
# on the critical path: `just bundles` builds and proves both platform halves
# locally, and `gh release create` attaches them. The dispatch-only workflow is
# retained as a proof lane with artifact uploads, not as the publisher.
#
# THE ORDER IS THE SAFETY PROPERTY. Everything that can refuse — a dirty tree,
# a gh account without push rights, a missing cross toolchain — refuses in the
# pre-flight, before the bump writes a version file and long before a tag
# exists. Everything that can fail expensively — the gates, the bundles —
# happens before the tag too. A release that dies after `git push --tags` has
# published a version with no assets behind it, which is the half-done state
# this ordering exists to prevent.

# Full release pipeline: check → release-proof (hp whole suite + Mac subset) → rust-deny → bump → bundles → smoke → changelog → tag → gh release
# Usage: just release
release:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ NEST }}
    {{ DETACHED }}
    {{ DISKFREE }}

    # THE DISK FLOOR comes first, before anything is claimed, fetched or written: a release that
    # runs out of disk mid-way stops after its claim and leaves a suspect lane lock. It refuses when
    # the checkout's or cargo's volume has less than AE_LANE_MIN_FREE_GIB (default 20) free.
    disk_floor release

    # Pre-flight: clean working tree (staged, unstaged, AND untracked)
    if ! git diff --quiet HEAD || [ -n "$(git ls-files --others --exclude-standard)" ]; then
        echo "Error: uncommitted or untracked changes" >&2; exit 1
    fi

    # THE BRANCH IS A PRE-FLIGHT QUESTION, not a late one. It used to be asked
    # after the bump, the compile, the bundles, the badge rewrite and the
    # changelog — so a release started on a feature branch mutated Cargo.toml,
    # Cargo.lock, README.md, docs/index.md and CHANGELOG.md, built two bundles,
    # and only then refused, leaving a half-mutated worktree to clean up by
    # hand. Nothing above this line has written anything.
    BRANCH=$(git rev-parse --abbrev-ref HEAD)
    if [ "$BRANCH" != "{{default_branch}}" ]; then
        echo "Error: releases must be from {{default_branch}} (currently on $BRANCH)" >&2; exit 1
    fi

    # PUBLICATION RIGHTS, PROVED BEFORE ANYTHING IS IRREVERSIBLE. Now that the
    # release is built and attached from here, `gh` is a hard prerequisite
    # rather than the best-effort afterthought it was when a runner published.
    #
    # `.permissions.push` is ASKED OF THE API, never inferred from a successful
    # login: an account can be authenticated, hold the `repo` scope, and still
    # have pull-only rights on this repository — which is the exact state this
    # laptop is in when the wrong one of two logged-in accounts is active.
    command -v gh >/dev/null 2>&1 || { echo "Error: gh is required to publish a release" >&2; exit 1; }
    gh auth status >/dev/null 2>&1 || { echo "Error: gh is not authenticated — gh auth login" >&2; exit 1; }
    REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
    [ -n "$REPO" ] || { echo "Error: gh cannot resolve this repository" >&2; exit 1; }
    PUSH=$(gh api "repos/$REPO" --jq .permissions.push)
    if [ "$PUSH" != "true" ]; then
        echo "Error: the active gh account cannot push to $REPO (permissions.push=$PUSH)" >&2
        echo "       gh auth status            # which account is active" >&2
        echo "       gh auth switch --hostname github.com --user <account>" >&2
        exit 1
    fi

    # The cross toolchain `just bundles` links the Linux half with, checked in
    # the same breath and for the same reason: it is present or it is not, and
    # discovering that after the tag is pushed helps nobody.
    command -v {{ RUST_MUSL_CC }} >/dev/null 2>&1 || {
        echo "Error: {{ RUST_MUSL_CC }} not found — the linux-x86_64-musl bundle cannot be linked." >&2
        echo "       brew install messense/macos-cross-toolchains/x86_64-unknown-linux-musl" >&2
        exit 1
    }

    # The smoke of the built release binary below is bounded, so the bound must
    # exist before anything is built: a hung `ae` must not hang the release.
    smoke_timeout="$(command -v timeout || command -v gtimeout || true)"
    [ -n "$smoke_timeout" ] || {
        echo "Error: the release smoke needs timeout (GNU coreutils; gtimeout on a Mac with brew coreutils)" >&2
        exit 1
    }

    # The hp that vouches for the whole suite is a pre-flight question too: without a usable
    # AE_RELEASE_HP and AE_RELEASE_HP_ROOT the release refuses here, before the fetch.
    {{ PROOF }}
    proof_destination || proof_refuse "$proof_why"

    git fetch {{GIT_REMOTE}} --tags
    git pull {{GIT_REMOTE}} {{default_branch}} --rebase

    # Gate 1: Quality
    nest just check

    # Gate 2: Tests on two machines. The hp runs the WHOLE suite on exactly this commit and
    # writes its receipt; this Mac runs only the boundary subset. The proof recipe checks the
    # receipt over ssh and runs the subset; with hp=run and no receipt yet, it starts the hp
    # whole run and runs the subset at the same time. It refuses on any doubt and never falls
    # back to the whole suite here (docs/development.md, "Release proof"). The identity it
    # proves is snapshotted now, after the rebase, and named to it.
    proof_identity || proof_refuse "$proof_why"
    proof_head="$id_head"
    proof_tree="$id_tree"
    nest just release-proof "expect=$proof_head:$proof_tree" hp=run

    # Gate 3: Supply chain on every release; advisory DB needs network, so offline refuses before the version is written.
    nest just rust-deny

    # The last identity recheck: the tree the proof vouched for must still be the clean
    # checkout, on the same HEAD, when the version is written.
    proof_same "$proof_head" "$proof_tree" || proof_refuse "$proof_why after rust-deny; the release stops before bump"

    # Version
    nest_capture VERSION just bump
    echo "Releasing v$VERSION"
    # Re-parse and compile after bump before any changelog or tag publication.
    detached cargo check --locked

    # The separate fuzz lock must carry the bump too, and it must before any gate step
    # below reads it: the bump changed the ae version that lock records, and
    # `rust-fuzz-check` runs `--locked`, so it refuses a lock that still records the
    # replaced version. Any refresh failure stops here under errexit, even if cargo
    # wrote a valid-looking partial update.
    detached cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null

    fuzz_delta="$(git diff --no-ext-diff --no-color --numstat -- fuzz/Cargo.lock)"
    # Compare all bytes, allowing only this ae version. A failed git show also
    # yields a mismatch and refuses; process-substitution errors cannot pass.
    if [ "$fuzz_delta" != $'1\t1\tfuzz/Cargo.lock' ] || ! cmp -s fuzz/Cargo.lock <(git show HEAD:fuzz/Cargo.lock | awk -v version="$VERSION" '
        /^\[\[package\]\]$/ { ae = 0 }
        /^name = "ae"$/ { ae = 1 }
        ae && /^version = / { $0 = "version = \"" version "\""; ae = 0 }
        { print }
    '); then
        echo "Error: fuzz/Cargo.lock must change only the ae version to $VERSION" >&2
        exit 1
    fi

    # The release proof vouched for nextest on the pre-bump tree and for nothing else. Every
    # other step of the gate runs here, on the tree the release will publish: the bump changed
    # the version every crate and doc test sees, so a break only the bump causes shows now,
    # before the bundles, not after the tag.
    nest just rust-fmt-check
    nest just rust-lint
    nest just rust-fuzz-check
    # The doctests, sized as the lane sizes them. A private TMPDIR and TMUX_TMPDIR
    # and no inherited TMUX keep them off the live tmux server, as the lane does.
    # `nest` runs the function in a forked shell, as the `( )` it replaces did, so the
    # sizing exports never reach the bundles below; a `( )` would leave its cargo running
    # after a TERM (#345).
    doc_dir="$(mktemp -d)"
    release_doctests() {
        {{ SIZING }}
        size_counts
        size_say "release doctests"
        detached env -u TMUX -u TMUX_PANE TMPDIR="$doc_dir" TMUX_TMPDIR="$doc_dir" nice -n 10 cargo test --doc --locked --all-features -- --test-threads "$NEXTEST_TEST_THREADS"
    }
    nest release_doctests || {
        doc_status=$?
        rm -rf "$doc_dir"
        ((nest_signal == 0)) || exit "$doc_status"
        echo "Error: the doctests failed on the bumped tree" >&2
        exit 1
    }
    rm -rf "$doc_dir"

    # Gate 4: the artifacts themselves. Both platform halves and the
    # release-level SHA256SUMS, built and proven HERE — this is what the release
    # gate guards. It runs BEFORE the tag deliberately: a failed
    # cross build or a musl binary that is not static costs a `git checkout` of
    # two version files, not an orphan tag with nothing behind it.
    nest just bundles

    # THE SMOKE of the binary the bundle carries (the native one, the half this
    # machine can run): it answers --version with the version just bumped to, and
    # `doctor` runs to its end, each inside a 10 s bound. `doctor` runs in a scratch
    # home with no config, so it exits 1 with a finding of its own; the smoke asks
    # only that it RAN: status 0 or 1. A timeout (124), a signal or a missing
    # binary refuses the release before the tag.
    binary="target/{{ RUST_NATIVE_TARGET }}/release/ae"
    smoke_dir="$(mktemp -d)"
    smoke_out="$("$smoke_timeout" 10 "$binary" --version 2>&1)" || smoke_out="(no answer within 10 s or a failure)"
    smoke_line="${smoke_out%%$'\n'*}"
    if [ "$smoke_line" != "ae $VERSION" ]; then
        rm -rf "$smoke_dir"
        echo "Error: smoke: the version probe of $binary printed '$smoke_line', want 'ae $VERSION'" >&2
        exit 1
    fi
    smoke_status=0
    nest env -u TMUX -u TMUX_PANE HOME="$smoke_dir" AE_HOME="$smoke_dir/state" CONFIG_FILE="$smoke_dir/config" TMUX_TMPDIR="$smoke_dir" \
        "$smoke_timeout" 10 "$binary" doctor >"$smoke_dir/health.out" 2>&1 || smoke_status=$?
    if ((nest_signal)); then
        rm -rf "$smoke_dir"
        exit "$smoke_status"
    fi
    if [ "$smoke_status" -gt 1 ]; then
        cat "$smoke_dir/health.out" >&2
        rm -rf "$smoke_dir"
        echo "Error: smoke: the health probe of $binary ended with status $smoke_status (124 is the 10 s bound), want 0 or 1" >&2
        exit 1
    fi
    rm -rf "$smoke_dir"
    echo "==> smoke: $binary answered both probes within 10 s"

    # Update the release badges.
    sed_i() {
        local f="$1"; shift
        cp -p "$f" "$f.tmp.$$" || return 1
        sed "$@" "$f" > "$f.tmp.$$" && mv "$f.tmp.$$" "$f" || { rm -f "$f.tmp.$$"; return 1; }
    }
    replace_badge_line() {
        local file="$1" pre_pattern="$2" prior_pattern="$3" post_line="$4" label="$5" source_pattern
        if grep -Eq "$pre_pattern" "$file"; then
            source_pattern="$pre_pattern"
        elif grep -Eq "$prior_pattern" "$file"; then
            source_pattern="$prior_pattern"
        else
            echo "Error: ${label} has no known pre-release or prior-release badge line" >&2
            exit 1
        fi
        sed_i "$file" -E -e "s@${source_pattern}@${post_line}@"
        if ! grep -Fxq "$post_line" "$file"; then
            echo "Error: ${label} did not become its expected release badge line" >&2
            exit 1
        fi
    }
    README_VERSION_PRE='^!\[Version: [0-9]+\.[0-9]+\.[0-9]+ untagged/pre-release\]\(https://img\.shields\.io/badge/version-[0-9]+\.[0-9]+\.[0-9]+%20untagged%2Fpre--release-blue\.svg\)$'
    README_VERSION_PRIOR='^\[!\[Release: [0-9]+\.[0-9]+\.[0-9]+\]\(https://img\.shields\.io/badge/release-[0-9]+\.[0-9]+\.[0-9]+-blue\.svg\)\]\(https://github\.com/clemens33/ae/releases\)$'
    README_VERSION_POST="[![Release: $VERSION](https://img.shields.io/badge/release-$VERSION-blue.svg)](https://github.com/clemens33/ae/releases)"
    README_INSTALL_PRE='^\[!\[Install\]\(https://img\.shields\.io/badge/install-checkout%20install-orange\.svg\)\]\(#install\)$'
    README_INSTALL_PRIOR='^\[!\[Install: curl \| bash\]\(https://img\.shields\.io/badge/install-curl%20%7C%20bash-orange\.svg\)\]\(#install\)$'
    README_INSTALL_POST='[![Install: curl | bash](https://img.shields.io/badge/install-curl%20%7C%20bash-orange.svg)](#install)'
    INDEX_VERSION_PRE="$README_VERSION_PRE"
    INDEX_VERSION_PRIOR="$README_VERSION_PRIOR"
    INDEX_VERSION_POST="$README_VERSION_POST"
    replace_badge_line README.md "$README_VERSION_PRE" "$README_VERSION_PRIOR" "$README_VERSION_POST" "README version"
    replace_badge_line README.md "$README_INSTALL_PRE" "$README_INSTALL_PRIOR" "$README_INSTALL_POST" "README Install"
    replace_badge_line docs/index.md "$INDEX_VERSION_PRE" "$INDEX_VERSION_PRIOR" "$INDEX_VERSION_POST" "docs/index version"
    if grep -En 'untagged/pre-release|checkout%20install|checkout install' README.md docs/index.md >&2; then
        echo "Error: pre-release badge or checkout-install prose remains; edit it deliberately before tagging" >&2
        exit 1
    fi
    # Generate changelog. The skip list, its WHY and its guards: `_cliff-skip`; the
    # text-to-argv boundary owner (and the proofs over it): `_cliff-run`.
    TAG="v$VERSION"
    nest just _cliff-run --tag "$TAG" -o CHANGELOG.md
    nest_capture RELEASE_BODY just _cliff-run --tag "$TAG" --unreleased --strip header
    RELEASE_BODY="${RELEASE_BODY:-Release $TAG}"

    # THE BUMP PROOF. The release proof is evidence about the PRE-BUMP tree, so the
    # release must show the bump left it alone: every tracked path that differs from
    # HEAD, and so every path the commit below would carry, is one of the six
    # version paths. A build script, a formatter or a changelog tool that touched
    # anything else stops the release here, by name, before the commit and the tag.
    version_paths=(Cargo.toml Cargo.lock fuzz/Cargo.lock README.md docs/index.md CHANGELOG.md)
    stray=""
    while IFS= read -r -d '' changed; do
        allowed=0
        for path in "${version_paths[@]}"; do
            [ "$changed" != "$path" ] || allowed=1
        done
        [ "$allowed" = 1 ] || stray="$stray $changed"
    done < <(git diff --name-only -z HEAD)
    if [ -n "$stray" ]; then
        echo "Error: the release changed paths outside the version files:$stray" >&2
        exit 1
    fi

    # Commit
    git add CHANGELOG.md
    git add -u
    git diff --cached --quiet || git commit -m "chore(release): $TAG"

    # EVERYTHING THE PUBLICATION NEEDS IS PREPARED BEFORE ANYTHING IS PUSHED.
    # A missing asset or an unwritable notes file must fail while the only state
    # is local. The notes go beside the assets rather than into a mktemp: `dist`
    # already exists and is writable at this point, and leaving the file there
    # means a manual retry has the exact body this run would have published.
    NOTES_FILE="dist/RELEASE_NOTES.md"
    printf '%s\n' "$RELEASE_BODY" > "$NOTES_FILE"
    ASSETS=(
        "dist/ae-$VERSION-darwin-arm64.tar.gz"
        "dist/ae-$VERSION-linux-x86_64-musl.tar.gz"
        dist/SHA256SUMS
    )
    for asset in "${ASSETS[@]}"; do
        [ -f "$asset" ] || { echo "Error: $asset is missing — just bundles did not produce it" >&2; exit 1; }
    done

    # THE REMOTE TAG IS CREATED BY THE RELEASE, NEVER PUSHED AHEAD OF IT. A
    # `git push <tag>` that succeeds and a `gh release create` that then fails
    # publishes a version with nothing behind it — `install` resolves the latest
    # release, finds no SHA256SUMS, and the advertised one-liner is broken for
    # everyone until a human notices. `gh release create --target` creates the
    # tag object and the release in ONE API call, so the failure mode is "no
    # remote tag" instead of "tag with no assets". The local tag is still made
    # first: `just bump` derives the next sequence from local tags and git-cliff
    # is asked for `--tag "$TAG"`, so it has to exist here either way.
    #
    # The BRANCH is pushed first because the tag has to name a commit the remote
    # already has. If the publication then fails, main carries a release commit
    # with no release — visible, recoverable, and the retry line is printed.
    git tag "$TAG"
    git push {{GIT_REMOTE}} {{default_branch}}
    SHA=$(git rev-parse "$TAG^{commit}")

    # A re-run after a partial failure meets an existing release object. Upload
    # into it rather than refusing — the shape the publish job used, for the
    # same reason.
    publish() {
        if gh release view "$TAG" -R "$REPO" >/dev/null 2>&1; then
            gh release upload "$TAG" "${ASSETS[@]}" --clobber -R "$REPO" && gh release edit "$TAG" -R "$REPO" --latest
        else
            # --latest pins make_latest explicitly: without it GitHub sorts by commit date.
            gh release create "$TAG" "${ASSETS[@]}" \
                -R "$REPO" \
                --target "$SHA" \
                --title "ae $VERSION" \
                --notes-file "$NOTES_FILE" \
                --latest
        fi
    }
    if ! publish; then
        echo "Error: publishing $TAG failed. The commit is pushed; the tag is LOCAL only." >&2
        echo "       Nothing is published, so nothing is broken for installers. Retry with:" >&2
        echo "       gh release create $TAG ${ASSETS[*]} -R $REPO --target $SHA --title 'ae $VERSION' --notes-file $NOTES_FILE --latest" >&2
        exit 1
    fi

    echo "Released $TAG"

# THE RELEASE PROOF (docs/development.md, "Release proof"): the tests of a release run on two
# machines, and each one runs what only it can.
#
# 1. The hp ran the WHOLE suite: a green `just test` on exactly this commit, whose gate-receipt
#    (`.local/gate-receipts/<tree hash>` in the hp's main checkout) says so. This recipe reads
#    that receipt over ssh, bounded to 30 s, and checks it strictly: the grammar, the tree and
#    HEAD of this clean checkout, Cargo.lock, the toolchain pin, rustc, the pinned nextest, a
#    host that answered as the one the receipt names and that is not this Mac, and an age of
#    at most 24 h with at most 300 s of clock skew.
# 2. This Mac runs the boundary subset, `just test-macos`.
#
# With `hp=run` (the release passes it) and no receipt for the tree on the hp, the two halves run
# at the same time: HP_LEG's hp_parallel starts the hp whole run and runs the subset beside it,
# then this recipe checks the receipt that run wrote exactly as above.
#
# The checkout must be clean and stay on the same HEAD and tree while the hp is asked and while
# the subset runs. Any doubt is one `release-proof: refused:` line and exit 1. Nothing falls
# back to the whole suite on the Mac. AE_RELEASE_HP=local is the HUMAN-ONLY override for a
# release without the hp: a DEVIATION line, then a fresh local receipt of this tree vouches, or
# the whole `just test` runs here. Agents never set it.

# Prove the hp whole suite plus the Mac subset for this clean commit (AE_RELEASE_HP, AE_RELEASE_HP_ROOT)
release-proof *args:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ NEST }}
    {{ RECEIPT }}
    {{ PROOF }}
    expect=""
    hp_run=no
    for arg in "$@"; do
        case "$arg" in
            expect=*) expect="${arg#expect=}" ;;
            hp=run) hp_run=yes ;;
            *) proof_refuse "unknown argument '$arg' (usage: just release-proof [expect=<head>:<tree>] [hp=run])" ;;
        esac
    done
    proof_destination || proof_refuse "$proof_why"
    proof_identity || proof_refuse "$proof_why"
    head="$id_head"
    tree="$id_tree"
    if [[ -n "$expect" && "$expect" != "$head:$tree" ]]; then
        proof_refuse "this checkout is $head:$tree, not the $expect the release named"
    fi
    echo "release-proof: clean tree $tree at HEAD $head"

    if [[ "$AE_RELEASE_HP" == local ]]; then
        echo "release-proof: DEVIATION: whole suite on this Mac, no hp proof (AE_RELEASE_HP=local, the human-only override)"
        local_root="$(receipt_root "$(git rev-parse --git-common-dir 2>/dev/null || true)")" || proof_refuse "not inside a git checkout"
        if receipt_match "$local_root/.local/gate-receipts/$tree"; then
            echo "release-proof: a whole green just test on this Mac vouches for this tree ($((receipt_age / 60)) min ago): no tests run"
        else
            echo "release-proof: no local receipt matches ($receipt_why): running the whole gate, just test"
            nest just test || proof_refuse "the whole gate failed on this Mac"
        fi
        proof_same "$head" "$tree" || proof_refuse "$proof_why while the whole Mac gate ran"
        echo "release-proof: ok: DEVIATION, the whole suite ran on this Mac for HEAD $head"
        exit 0
    fi

    bound="$(command -v timeout || command -v gtimeout || true)"
    [[ -n "$bound" ]] || proof_refuse "the hp query needs timeout (GNU coreutils; gtimeout on a Mac with brew coreutils)"
    reply_dir="$(mktemp -d)"
    trap 'status=$?; rm -rf "$reply_dir"; exit "$status"' EXIT
    echo "release-proof: asking $AE_RELEASE_HP for the receipt of tree $tree"
    hp_query() {
        status=0
        "$bound" 30 ssh -o BatchMode=yes -o ConnectTimeout=10 -- "$AE_RELEASE_HP" \
            "hostname -s; head -c 8193 $AE_RELEASE_HP_ROOT/.local/gate-receipts/$tree" \
            >"$reply_dir/reply" 2>"$reply_dir/err" || status=$?
    }
    hp_query
    parallel=no
    if ((status != 0)); then
        # With hp=run (the release passes it), a receipt the hp provably does not have yet is
        # made now: the hp whole run and the Mac subset run at the same time (hp_parallel).
        # A timeout or an unreachable hp still refuses, and so does anything the missing-only
        # check on the hp cannot prove absent.
        if [[ "$hp_run" == yes && "$status" != 124 && "$status" != 255 ]]; then
            parallel=yes
        else
            sed 's/^/release-proof: ssh: /' "$reply_dir/err" >&2
            case "$status" in
                124) proof_refuse "the hp did not answer within 30 s" ;;
                255) proof_refuse "ssh could not reach $AE_RELEASE_HP (exit 255)" ;;
                *) proof_refuse "the hp has no readable receipt for tree $tree under $AE_RELEASE_HP_ROOT (exit $status): run a whole green just test there on HEAD $head" ;;
            esac
        fi
    fi
    if [[ "$parallel" == yes ]]; then
        {{ HP_LEG }}
        hp_parallel
        hp_query
        if ((status != 0)); then
            sed 's/^/release-proof: ssh: /' "$reply_dir/err" >&2
            proof_refuse "the hp leg ended green, but its receipt for tree $tree cannot be read (exit $status)"
        fi
    fi
    hp_host="$(head -n 1 "$reply_dir/reply")"
    tail -n +2 "$reply_dir/reply" >"$reply_dir/receipt"
    host_re='^[A-Za-z0-9][A-Za-z0-9.-]*$'
    [[ "$hp_host" =~ $host_re ]] || proof_refuse "the hp answered no plain host name on its first line"
    size="$(wc -c <"$reply_dir/receipt" | tr -d ' ')"
    ((size <= 8192)) || proof_refuse "the hp receipt is larger than 8192 bytes"
    receipt_parse "$reply_dir/receipt" || proof_refuse "the hp receipt is $receipt_why"
    [[ "$r_tree" == "$tree" ]] || proof_refuse "the hp receipt names tree $r_tree, not $tree"
    [[ "$r_head" == "$head" ]] || proof_refuse "the hp receipt names HEAD $r_head, not $head"
    for key in lock_hash toolchain_pin rustc; do
        receipt_fact "$key" || proof_refuse "this Mac cannot read its $key: $receipt_why"
        name="r_$key"
        [[ "${!name}" == "$fact" ]] || proof_refuse "$key differs (hp: ${!name}; this Mac: $fact)"
    done
    hp_nextest="${r_nextest#cargo-nextest }"
    if [[ "$hp_nextest" == "$r_nextest" || "${hp_nextest%% *}" != "{{ NEXTEST_VERSION }}" ]]; then
        proof_refuse "the hp ran $r_nextest, not the pinned cargo-nextest {{ NEXTEST_VERSION }}"
    fi
    [[ "$r_host" == "$hp_host" ]] || proof_refuse "the receipt was written on $r_host, but the hp answered as $hp_host"
    receipt_fact host || proof_refuse "this Mac cannot read its host name: $receipt_why"
    [[ "$r_host" != "$fact" ]] || proof_refuse "the receipt was written on this Mac ($fact), not on the hp"
    age=$(($(date -u +%s) - $(receipt_epoch "$r_time")))
    ((age >= -300)) || proof_refuse "the hp receipt is dated $((-age)) s in the future, beyond the 300 s of clock skew allowed"
    ((age <= 86400)) || proof_refuse "the hp receipt is stale: $((age / 3600)) h old, the limit is 24 h"
    proof_same "$head" "$tree" || proof_refuse "$proof_why while the hp was asked"
    echo "release-proof: the hp ($r_host) ran the whole suite green on this tree $((age / 60)) min ago"
    if [[ "$parallel" == yes ]]; then
        echo "release-proof: ok: the hp ran the whole suite and this Mac the boundary subset at the same time, on HEAD $head"
        exit 0
    fi

    echo "release-proof: running the Mac boundary subset, just test-macos"
    nest just test-macos || proof_refuse "the Mac boundary subset failed"
    proof_same "$head" "$tree" || proof_refuse "$proof_why while the Mac subset ran"
    echo "release-proof: ok: the hp ran the whole suite and this Mac the boundary subset, on HEAD $head"

# ── Bundle ───────────────────────────────────────────────────────────

# Build one release bundle: ae-core + install + SHA256SUMS, tarred as
# ae-<version>-<platform>.tar.gz in the current directory.
#
# ONE SPELLING, and that is the whole point of the recipe. The release workflow
# calls this rather than open-coding the same cp/chmod/tar on each platform leg,
# so a change to what a bundle IS cannot land on one platform and miss the
# other — and the shape is runnable on a laptop, where the suites pin it.
#
# The bundle's SHA256SUMS covers its two EXECUTABLE members by bare basename.
# It is a different file from the release-level SHA256SUMS, which covers the
# tarballs: this one is copied verbatim into ~/.ae/versions/<v>/ at install time
# and is what the installed core validates itself against.
bundle version platform binary:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ NEST }}
    version="{{ version }}"
    platform="{{ platform }}"
    binary="{{ binary }}"
    [ -x "$binary" ] || { echo "Error: $binary is not an executable core" >&2; exit 1; }
    # THE MEMBER MUST BE THE VERSION THE BUNDLE CLAIMS, and the strong proof is
    # to ask it. That only works when this kernel can exec it: `just bundles`
    # cross-builds the Linux half on an Apple Silicon laptop, where the musl ELF
    # will not run. So the proof is asked of a NATIVE member, and degraded — out
    # loud, never silently — to a byte search for the version string in a
    # foreign one. Running the cross-built core is the Linux CI lane's job
    # (.github/workflows/rust.yml runs it, version-checks it, proves it static,
    # and resolves a real name through it); this recipe never pretends to have
    # done that here.
    case "$(uname -s):$(uname -m)" in
        Darwin:arm64) host_platform=darwin-arm64 ;;
        Linux:x86_64) host_platform=linux-x86_64-musl ;;
        *) host_platform= ;;
    esac
    want="ae $version"
    if [ "$platform" = "$host_platform" ]; then
        # The FIRST line only: `--version` answers in two, the second naming the
        # tmux this machine has, which is not part of the version claim. Cut in
        # the shell, not through `head`: under `pipefail` a reader that stops
        # after one line hands the core a broken pipe on the second.
        got="$("$binary" --version)"
        got="${got%%$'\n'*}"
        [ "$got" = "$want" ] || { echo "Error: --version printed '$got', want '$want'" >&2; exit 1; }
    else
        # -F IS LOAD-BEARING. Without it the dots in a CalVer version are BRE
        # wildcards, so `2026.9.1` matches the bytes `2026x9y1` and a wrong or
        # corrupted foreign core passes the only proof this host can make.
        LC_ALL=C grep -Fqa -- "$version" "$binary" ||
            { echo "Error: $binary carries no '$version' string — it is not a $version core" >&2; exit 1; }
        echo "==> $platform is foreign to this host: version proven by byte search, not by running it"
    fi
    root="ae-$version-$platform"
    # C83: a bundle root is published 0555, so `rm -rf` on a previous run's
    # directory fails "Permission denied" — a 0555 directory refuses the unlink
    # of its own entries. Make it writable first, the same way the installer
    # does for its own private trees.
    [ ! -e "$root" ] || chmod -R u+w "$root" 2>/dev/null || true
    rm -rf "$root"
    mkdir "$root"
    cp "$binary" "$root/ae-core"
    cp install "$root/install"
    if command -v sha256sum >/dev/null 2>&1; then
        sums() { sha256sum "$@"; }
    else
        sums() { shasum -a 256 "$@"; }
    fi
    # Bare basenames: the manifest is read relative to the directory holding it,
    # in the version directory as well as here, so it must not carry a path.
    ( cd "$root" && sums ae-core install > SHA256SUMS )
    chmod 0555 "$root/ae-core" "$root/install"
    chmod 0444 "$root/SHA256SUMS"
    chmod 0555 "$root"
    # Keep macOS xattrs out of bundles; GNU tar otherwise warns on WSL extraction.
    tar_flags=(--no-xattrs)
    if tar --version 2>/dev/null | grep -qi bsdtar; then
        tar_flags+=(--no-mac-metadata)
    fi
    nest env COPYFILE_DISABLE=1 tar "${tar_flags[@]}" -czf "$root.tar.gz" "$root"
    echo "==> $root.tar.gz"

# Both release halves, and the release-level SHA256SUMS over them, built HERE.
#
# THIS IS THE RECIPE THAT TAKES GITHUB ACTIONS OFF THE CRITICAL PATH. It emits
# exactly the two platform tarballs and one SHA256SUMS manifest that `just release`
# publishes; the workflow legs build those tarballs separately as proof artifacts, so a
# release no longer waits on a runner and `install` verifies the same bytes it always did.
#
# It does NOT restate what a bundle is. `just bundle` remains the one
# definition, called once per platform — the same two calls the two workflow
# legs make — so a change to a bundle's shape still cannot land on one platform
# and miss the other.
#
# Apple Silicon only, and refused elsewhere rather than half-served: the
# darwin-arm64 half is the native build and the linux-x86_64-musl half is
# cross-linked against the Homebrew musl toolchain, and a Linux host has no way
# to produce the macOS half at all.
#
# Output lands in `dist/` (gitignored), wiped first — a stale tarball from a
# previous version must never reach a release.

# Build both platform bundles + the release SHA256SUMS into ./dist
bundles:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ NEST }}
    {{ DETACHED }}

    case "$(uname -s):$(uname -m)" in
        Darwin:arm64) : ;;
        *)
            echo "Error: just bundles needs an Apple Silicon host — it builds darwin-arm64 natively and cross-links linux-x86_64-musl" >&2
            exit 1
            ;;
    esac

    command -v {{ RUST_MUSL_CC }} >/dev/null 2>&1 || {
        echo "Error: {{ RUST_MUSL_CC }} not found — the linux-x86_64-musl half cannot be linked." >&2
        echo "       brew install messense/macos-cross-toolchains/x86_64-unknown-linux-musl" >&2
        exit 1
    }

    # llvm-readobj comes from the PINNED toolchain's llvm-tools component
    # (rust-toolchain.toml), never from whatever binutils a laptop happens to
    # carry: macOS ships no readelf, and a static proof that depends on the
    # machine is a proof that silently stops running.
    readobj="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-readobj"
    [ -x "$readobj" ] || { echo "Error: $readobj is missing — rustup component add llvm-tools" >&2; exit 1; }

    nest_capture version just version
    out="dist"
    native_bin="target/{{ RUST_NATIVE_TARGET }}/release/ae"
    musl_bin="target/{{ RUST_CROSS_TARGET }}/release/ae"

    # BOTH HALVES IN ONE CARGO INVOCATION: two `--target` flags give one
    # jobserver and one build-directory lock, so the two single-codegen-unit
    # `ae` compiles (lto = thin, codegen-units = 1, both unchanged) run side by
    # side instead of one after the other (two sequential builds took 343 s in a
    # release). Two cargo PROCESSES on one target dir would queue on the lock
    # instead, so this stays one process. An explicit `--target` puts the native
    # binary under target/<triple>/release/, which is why `native_bin` names the
    # triple; `just install` and `just rust-build-release` build the host alone
    # and keep target/release/ae.
    #
    # ring's build script compiles C for the target, so the cross build needs a
    # musl C COMPILER as well as a musl linker. The linker is pinned in
    # .cargo/config.toml; CC_<target> is what the `cc` crate reads, it is scoped
    # to the musl target so it leaves the native half alone, and it is set HERE
    # rather than there because that name does not exist on the Linux CI leg,
    # which builds this same target against musl-tools. The build runs `detached` (#345), so a
    # TERM to the release ends both compiles before the recipe exits; bash exports an assignment
    # that precedes a function call to the commands the function runs, so cargo still sees it.
    CC_x86_64_unknown_linux_musl={{ RUST_MUSL_CC }} \
        detached cargo build --release --locked --target {{ RUST_NATIVE_TARGET }} --target {{ RUST_CROSS_TARGET }}

    # The native half is RUN, which is the whole reason it is the native half.
    echo "==> native: $("$native_bin" --version)"

    # The musl half is proven STATIC. It cannot be run here — that proof, and
    # the musl DNS/NSS proof beside it, stay on the Linux CI leg. What a laptop
    # can prove is the link, and the authoritative signal is the absence of a
    # PT_INTERP segment: no program interpreter means no dynamic loader.
    #
    # Capture FIRST. Piping llvm-readobj straight into grep would turn a broken
    # readobj into "no PT_INTERP found", which is to say into a pass.
    headers="$("$readobj" --program-headers "$musl_bin")"
    if printf '%s\n' "$headers" | grep -q 'PT_INTERP'; then
        echo "Error: the musl binary has a PT_INTERP segment — it is dynamically linked" >&2
        printf '%s\n' "$headers" >&2
        exit 1
    fi
    # Second, independent signal. Rust musl builds are static-pie, which `file`
    # spells "static-pie linked" or "statically linked" depending on version —
    # both contain "static"; a dynamic build says "dynamically".
    described="$(file -b "$musl_bin")"
    case "$described" in
        *dynamically*) echo "Error: file reports a dynamic binary: $described" >&2; exit 1 ;;
        *static*) : ;;
        *) echo "Error: file does not report a static binary: $described" >&2; exit 1 ;;
    esac
    echo "==> musl: static proof ok — $described"

    # C83 again: a bundle root is published 0555, and a 0555 directory refuses
    # the unlink of its own entries, so a previous run's tree is made writable
    # before it is removed.
    [ ! -e "$out" ] || chmod -R u+w "$out" 2>/dev/null || true
    rm -rf "$out"
    mkdir "$out"

    nest just bundle "$version" darwin-arm64 "$native_bin"
    nest just bundle "$version" linux-x86_64-musl "$musl_bin"
    for platform in darwin-arm64 linux-x86_64-musl; do
        root="ae-$version-$platform"
        mv "$root.tar.gz" "$out/"
        chmod -R u+w "$root" 2>/dev/null || true
        rm -rf "$root"
    done

    # THE RELEASE-LEVEL MANIFEST, and a different file from the SHA256SUMS
    # inside each bundle: this one covers the two TARBALLS. `install` reads it
    # to discover the latest version and to verify an archive before extracting
    # it, so its bytes are a contract — `<sha256><SP><SP><basename>`, bare
    # basenames, glob order — byte-identical to the `sha256sum -- ae-*.tar.gz`
    # the publish job ran.
    if command -v sha256sum >/dev/null 2>&1; then
        sums() { sha256sum "$@"; }
    else
        sums() { shasum -a 256 "$@"; }
    fi
    ( cd "$out" && sums -- ae-*.tar.gz > SHA256SUMS )

    echo "==> $out/"
    ls -l "$out"

# ── Install ──────────────────────────────────────────────────────────

# Checkout-mode installation through the same door a release install uses.
#
# NOT `./install`: since slice Z4 that script is a fetch-only bootstrap and
# would download a release over the developer's own build. Bundling the local
# binary first keeps `bundle` the ONE spelling of what a bundle is, and the
# checkout then publishes through `_install` exactly as a downloaded one does.
install:
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(just version)"
    case "$(uname -s):$(uname -m)" in
        Darwin:arm64) platform=darwin-arm64 ;;
        Linux:x86_64) platform=linux-x86_64-musl ;;
        *) echo 'Error: unsupported platform' >&2; exit 1 ;;
    esac
    cargo build --release --locked
    just bundle "$version" "$platform" target/release/ae
    ./target/release/ae _install --from "ae-$version-$platform"

# ── Docs ─────────────────────────────────────────────────────────────
# Optional. Requires `pip install mkdocs-material`.

# Serve the docs site locally with live reload
docs:
    mkdocs serve

# Build the static docs site into ./site (gitignored)
docs-build:
    mkdocs build --strict

# ── Quick Reference ──────────────────────────────────────────────────

# Show available recipes
help:
    @just --list

# ── Rust core lanes (epic #79 / #80) ─────────────────────────────────
# Bash recipes above remain for policy-frozen glue. Everything below is prefixed
# `rust-` and owns core/toolchain work.
#
# PINS ARE THE CONTRACT. This block is the single source of truth for dev-tool
# versions. `rust-setup` installs shared product-development tools; the human-run
# fuzz lane checks its optional cargo-fuzz pin itself. The product compiler pin lives
# in rust-toolchain.toml (cargo/rustup read it, just does not).
#
# EVERY graph-consuming cargo invocation below passes `--locked`. Without it
# cargo will happily UPDATE Cargo.lock to satisfy a build and then report green —
# a committed lockfile that no lane ever enforces is decoration. `cargo fmt` is
# the one exception: it does not resolve the dependency graph.
# Two spellings differ and are not interchangeable:
#   cargo-deny takes it as a GLOBAL option  -> `cargo deny --locked check`
#     (`cargo deny check --locked` exits 2 — measured)
#   cargo-mutants does not accept it at all -> `--cargo-arg=--locked` passes it
#     through to the cargo it drives (measured; bare `--locked` is rejected)

# `just` is a PREREQUISITE of the bootstrap contract, not something rust-setup
# installs — you cannot run a recipe that installs the tool running the recipe.
# The pin is recorded HERE anyway so CI has ONE source of truth to read, and so a
# bump re-keys the CI tool cache instead of silently reusing the old binary.
JUST_VERSION := "1.57.0"

NEXTEST_VERSION := "0.9.151"
TAPLO_VERSION := "0.10.0"
DENY_VERSION := "0.20.2"
MUTANTS_VERSION := "27.1.0"
LLVM_COV_VERSION := "0.9.0"
# The fuzz lane's three pins. cargo-fuzz is installed on request, not by
# `rust-setup`, because the lane is human-run. FUZZ_TOOLCHAIN is the lane's ONE
# nightly source — there is no fuzz/rust-toolchain.toml. Fuzz execution passes
# `cargo +{{ FUZZ_TOOLCHAIN }}`; compile checks use the product's unchanged pin.
CARGO_FUZZ_VERSION := "0.13.2"
FUZZ_TOOLCHAIN := "nightly-2026-08-20"
FUZZ_TARGETS := "archive_harness console_draft_name config_parse meta_parse launch_cmd_lex config_command config_roster_edit quota_claude_cache quota_feed quota_codex_rollout quota_grok_log quota_agy_output quota_go_usage quota_copilot_user quota_muse_usage agy_limit_pane quota_history usage_claude_transcript usage_codex_rollout usage_prices harness_observed picker_agents launch_stamp last_live_epochs sanitize_field request_id_select events_parse worker_birth event_stamp journal_append board_claude board_codex board_grok board_muse board_agy board_opencode brief_retry_record challenge_summary console_command app_view ask_unframe lease_holder card_body effort_flag effort_record effort_fold token_parse codex_developer_record link_scan reader_decode reader_markdown panel_episode unreach_fold jump_return wait_moved autoreseat_episode stats_fold stats_store stats_disk tool_calls_claude tool_calls_codex usage_lines tool_stats_fold tool_stats_incr tool_stats_doc memo_view worktree_porcelain opencode_variant redact_text desk_record desk_pin"
# Arrived with the FIRST runtime dependency (P4.3): cargo-deny gates the graph's
# advisories/licenses/bans/sources, cargo-vet gates its PROVENANCE (who reviewed
# the code). See `rust-vet`.
VET_VERSION := "0.10.2"
# The changelog generator. An UNPINNED generator means CHANGELOG.md and the GitHub
# release body differ by machine and drift under us, so it is installed and probed
# like every other dev tool.
GIT_CLIFF_VERSION := "2.13.1"

# The local Linux gates (`just rust-linux`, `just rust-linux-26`) fetch two
# binaries onto the host and run everything else inside a container. Each base
# image is pinned by its MANIFEST DIGEST — --platform selects the arm64/amd64
# variant out of the same list — and both fetched binaries are digest-checked on
# the way in. rustup-init comes from the VERSIONED archive URL, never the moving
# dist one, so the digests below keep holding.
LINUX_IMAGE := "ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3"
# Ubuntu 26.04 is the second validated Linux host: it ships tmux 3.6, which asks
# the session bus for a systemd transient scope whenever it spawns or respawns a
# pane child, and uutils coreutils in place of GNU's. Only `just rust-linux-26` uses it.
LINUX_IMAGE_26 := "ubuntu:26.04@sha256:6a06111c3b0a8600feadb270aec771dc239bf407b25625f93f44ab00e11f4d3c"
RUSTUP_VERSION := "1.29.1"
LINUX_JUST_SHA_ARM64 := "f225044a81adea6e0b3a8b9370aaf374e6af76c8735ae263ac993df55fd137ec"
LINUX_RUSTUP_SHA_ARM64 := "15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433"

# The foreign target. musl, not gnu — ae ships a STATIC binary with no host
# runtime dependency, and gnu is not that (see rust-toolchain.toml for the NSS caveat).
# It is LINKED here now, by `just bundles`, which builds the Linux release half
# on this laptop; it is RUN only on the Linux CI leg, which stays the proof of
# record for a musl binary that executes and resolves a real name.

RUST_CROSS_TARGET := "x86_64-unknown-linux-musl"

# The native release half's triple. `just bundles` names it with `--target`, in
# the one cargo invocation that also builds the foreign half, so the native
# binary lands in target/<this>/release/ae. It is the host half of
# rust-toolchain.toml's two targets; `tests/it/gate.rs` refuses drift between them.

RUST_NATIVE_TARGET := "aarch64-apple-darwin"

# The musl C compiler and linker driver, in the spelling the prebuilt Homebrew
# tap publishes (messense/macos-cross-toolchains). ONE name, three readers: the
# linker pin in `.cargo/config.toml`, the `CC_x86_64_unknown_linux_musl` the
# `cc` crate reads when ring compiles C for the target, and the advisory in
# `rust-setup`. `tests/it/gate.rs` refuses drift between this and the config.
#
# It is deliberately NOT installed by `rust-setup`: a cross toolchain is a
# machine dependency of RELEASING, and the bootstrap contract (rustup + just,
# nothing else) covers developing. Ubuntu names the same tool `musl-gcc`, which
# is why the workflow legs override the linker rather than sharing this pin.

RUST_MUSL_CC := "x86_64-unknown-linux-musl-gcc"

# Bootstrap: rustup toolchain + the pinned dev tools. Idempotent — every tool is
# version-checked first, so a second run installs nothing. Honest prerequisites:
# rustup and just, nothing else.

# Install the pinned Rust toolchain + dev tools (idempotent)
rust-setup:
    #!/usr/bin/env bash
    set -euo pipefail

    command -v rustup >/dev/null || { echo "error: rustup is required (https://rustup.rs)" >&2; exit 1; }

    # No toolchain argument: rustup resolves the ACTIVE toolchain, which in this
    # directory is rust-toolchain.toml — channel, profile, components and both
    # targets come from the pin rather than being restated here.
    echo "==> toolchain (rust-toolchain.toml)"
    rustup toolchain install --no-self-update

    # `cargo install` compiles from source and is slow, so it runs only on an
    # actual version mismatch. Each probe prints the tool's own reported version
    # and matches the pin as a WHOLE WORD — a prefix match would accept 0.9.1430.
    ensure() {
        local crate="$1" want="$2" probe="$3"
        local have
        have="$(eval "$probe" 2>/dev/null | tr -c '0-9A-Za-z.' '\n' | grep -Fx "$want" || true)"
        if [ -n "$have" ]; then
            printf '    ok   %-16s %s\n' "$crate" "$want"
            return 0
        fi
        printf '    ==>  %-16s installing %s\n' "$crate" "$want"
        cargo install --locked --version "$want" "$crate"
    }

    echo "==> pinned dev tools"
    ensure cargo-nextest  "{{ NEXTEST_VERSION }}"  'cargo nextest --version'
    ensure taplo-cli      "{{ TAPLO_VERSION }}"    'taplo --version'
    ensure cargo-deny     "{{ DENY_VERSION }}"     'cargo deny --version'
    ensure cargo-mutants  "{{ MUTANTS_VERSION }}"  'cargo mutants --version'
    ensure cargo-llvm-cov "{{ LLVM_COV_VERSION }}" 'cargo llvm-cov --version'
    ensure cargo-vet      "{{ VET_VERSION }}"      'cargo vet --version'
    ensure git-cliff      "{{ GIT_CLIFF_VERSION }}" 'git-cliff --version'

    # REPORTED, NOT PROVISIONED, and never fatal. The musl cross toolchain is
    # needed by `just bundles` (and so by `just release`) and by nothing else:
    # `just rust-check` and `just rust-build-release` are native lanes. A clone
    # that only develops is complete without it, so this prints the one line
    # that fixes it and moves on. Both spellings are accepted because the tap
    # publishes the triple-prefixed name and the keg carries the short one.
    echo "==> musl cross toolchain (optional — only 'just bundles' links it)"
    if command -v {{ RUST_MUSL_CC }} >/dev/null 2>&1 || command -v x86_64-linux-musl-gcc >/dev/null 2>&1; then
        printf '    ok   %-16s %s\n' "musl cc" "{{ RUST_MUSL_CC }}"
    else
        printf '    --   %-16s absent; just bundles cannot link the Linux half\n' "musl cc"
        printf '         brew install messense/macos-cross-toolchains/x86_64-unknown-linux-musl\n'
    fi

    echo "==> ready — run: just rust-check"

# THE RUN'S START SNAPSHOT: `HEAD`, the tree hash and `clean` or `dirty`, as one line,
# or empty when git cannot say. `just` evaluates every top-level assignment before it
# runs any recipe, so this is read before the first dependency of `rust-check` starts,
# which is the only point that is "the start of the whole run" without adding a
# dependency to the gate. The receipt writer compares it with the end of the run.
# It never fails `just` (a non-git directory gives an empty snapshot) and takes no git
# lock. It needs only `sh` and `git` on `PATH`, and it costs one `git status` per `just`
# invocation.
RUN_START := `h="$(git rev-parse --verify -q HEAD 2>/dev/null)"; t="$(git rev-parse --verify -q 'HEAD^{tree}' 2>/dev/null)"; [ -n "$h" ] && [ -n "$t" ] || exit 0; if [ -z "$(git --no-optional-locks status --porcelain 2>/dev/null)" ]; then echo "$h $t clean"; else echo "$h $t dirty"; fi`

# Its last act is the GATE RECEIPT (`RECEIPT` below): a whole green run on a clean
# committed tree writes one file naming that tree, so `just release` need not run
# nextest again. The writer is HERE, after the four dependencies, and nowhere else:
# `rust-test`, the lane, `pre` and every filtered run never reach it, so a partial
# or advisory run cannot leave a receipt. A failed dependency stops `just` before
# this body, so a red run neither writes one nor touches the last one. A receipt
# vouches only for the tree the whole run tested: it is written only when the run
# STARTED on a clean tree and the tree is still clean with the same HEAD and tree
# hash at the END (`RUN_START` above); a commit, a reset or a cleanup while the tests
# ran writes none. The receipt is a report and never a failure: whatever goes wrong
# here is one `gate-receipt:` line on stderr and the gate keeps its green exit status.

# The gate: everything a change must pass before it is offered for review
rust-check: rust-fmt-check rust-lint rust-fuzz-check rust-test
    #!/usr/bin/env bash
    set -euo pipefail
    # The receipt's `cargo nextest` calls go without a compiler wrapper, as every other
    # cargo call of the gate does (the note on SIZING below).
    export RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WRAPPER=
    {{ RECEIPT }}
    # A receipt vouches only for the tree the whole run tested. `just` recorded
    # HEAD, the tree hash and the cleanliness when it STARTED (RUN_START); the tree
    # must still be clean, on that HEAD and that tree hash, when the run ENDS.
    receipt_start="{{ RUN_START }}"
    receipt_steady() {
        local head_now tree_now
        if [[ -n "$(git status --porcelain 2>/dev/null || echo unreadable)" ]]; then
            receipt_why="the working tree is not clean, and a receipt names a committed tree"
            return 1
        fi
        head_now="$(git rev-parse --verify -q HEAD 2>/dev/null || true)"
        tree_now="$(git rev-parse --verify -q 'HEAD^{tree}' 2>/dev/null || true)"
        if [[ "$head_now" != "$receipt_head" || "$tree_now" != "$receipt_tree" ]]; then
            receipt_why="HEAD or the tree moved during the run (started on ${receipt_head:0:12}, ended on ${head_now:0:12}), so the tests did not run on the tree the receipt would name"
            return 1
        fi
    }
    receipt_write() {
        local common root dir tree tmp receipt_head receipt_tree receipt_state
        read -r receipt_head receipt_tree receipt_state <<<"$receipt_start" || true
        if [[ -z "$receipt_head" || -z "$receipt_tree" || -z "$receipt_state" ]]; then
            receipt_note "not written: git did not name the tree when the run started"
            return 0
        fi
        if [[ "$receipt_state" != clean ]]; then
            receipt_note "not written: the working tree was not clean when the run started, and a receipt names a committed tree"
            return 0
        fi
        common="$(git rev-parse --git-common-dir 2>/dev/null || true)"
        root="$(receipt_root "$common")" || { receipt_note "not written: not inside a git checkout"; return 0; }
        # A receipt names a COMMITTED tree. Staged, unstaged and untracked changes all
        # mean the tests ran on something no tree hash names.
        receipt_steady || { receipt_note "not written: $receipt_why"; return 0; }
        dir="$root/.local/gate-receipts"
        if [[ -L "$dir" ]] || ! mkdir -p "$dir" 2>/dev/null; then
            receipt_note "not written: $dir is not a usable directory"
            return 0
        fi
        tree="$receipt_tree"
        tmp="$(mktemp "$dir/.tmp.XXXXXX" 2>/dev/null)" || { receipt_note "not written: $dir is not writable"; return 0; }
        # Built from the same readers the release uses, and read back by the strict
        # reader before it is published, so a receipt that is written is one that parses.
        # The steady check runs again right before the publish, after the slow probes.
        if receipt_body >"$tmp" && receipt_parse "$tmp" && receipt_steady && mv "$tmp" "$dir/$tree"; then
            receipt_note "wrote $dir/$tree"
        else
            rm -f "$tmp"
            receipt_note "not written: ${receipt_why:-the receipt could not be published}"
        fi
    }
    receipt_write || true
    exit 0

# Cargo fmt runs `detached`, as every cargo step of the gate does (#342): `exec` alone left its
# rustfmt children running after a TERM. taplo is the last command and is exec'd, so the TERM
# that just passes on reaches taplo itself.
# Check formatting in both languages of the build: Rust and TOML
rust-fmt-check:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ NEST }}
    {{ DETACHED }}
    detached cargo fmt --all --check
    detached cargo fmt --manifest-path fuzz/Cargo.toml --check
    exec taplo fmt --check

# Auto-format Rust + TOML
rust-fmt:
    cargo fmt --all
    cargo fmt --manifest-path fuzz/Cargo.toml
    taplo fmt

# `-D warnings` is what makes [lints] a gate rather than a suggestion — which is
# also why unwrap_used/expect_used are declared "warn" in Cargo.toml and still
# fail here. --all-targets so tests are linted too.

# THE ONE SIZING HELPER of every heavy recipe: the test lane (_tmux-isolated),
# rust-lint and rust-fuzz-check include it with `{{ SIZING }}` and call
# `size_counts`, so the arithmetic below exists once and tests/it/gate.rs pins
# that no recipe carries a copy. It is shell text interpolated into those
# recipes (not a script file), so every caller shares one shell with it.
#
# Each count is the LEAST of its ceilings, so a shell profile that exports a wide
# NEXTEST_TEST_THREADS or CARGO_BUILD_JOBS can lower a run but never lift it past
# what jobs-budget says is free right now. Threads: jobs-budget (the online CPU
# count without it), the lane ceiling `lane_threads` (the first of the three named
# values at the top of the helper), an inherited NEXTEST_TEST_THREADS. The test
# lane then reads load1 ONCE, at its start (size_load): above `load_ceiling`, or
# unreadable, the threads fall to `loaded_threads` and the lane says so on stderr.
# That guard only lowers and never refuses, waits or aborts: an unattended gate
# has to start. A lane override (AE_TEST_THREADS) is not guarded; it replaces the
# rule.
# Jobs: jobs-budget, an inherited CARGO_BUILD_JOBS. Only one positive integer of at
# most three digits is a ceiling (jobs-budget's answer too: a longer one wraps the
# arithmetic, and it is ignored): a relative count such as -2, num-cpus, default or
# an oversize value is NO ceiling, is said to be ignored and never enters the
# arithmetic. Never blocks on jobs-budget. The one deliberate per-run override is
# the lane's own AE_TEST_THREADS / AE_TEST_JOBS (same shape), which replace the
# rule; a malformed one exits 2 rather than guess. The result is exported, so it
# also beats a [build] jobs in a user's cargo config. Without jobs-budget there is
# no live ceiling for the jobs: an inherited value stands untouched (cargo reads a
# relative one itself), else cargo's own default.
#
# size_counts sets threads, jobs, threads_from, jobs_from and exports
# NEXTEST_TEST_THREADS (always) and CARGO_BUILD_JOBS (unless it stands untouched);
# size_say <label> prints the ceilings that applied on stderr; size_load <label>
# applies the load guard to the threads size_counts chose (the test lane calls it);
# load1_now and load_above are the one reading of the machine's load, shared with
# the stress lane's start refusal.
#
# Including the helper also empties RUSTC_WRAPPER and CARGO_BUILD_RUSTC_WRAPPER, so
# no cargo call of those recipes (nor of the release doctest step) runs through a
# compiler wrapper from the caller's shell or `~/.cargo/config.toml`: the repo does not
# use sccache (`.cargo/config.toml`). An empty value, not an unset one, because cargo
# (measured, 1.97.1) reads an empty RUSTC_WRAPPER as "no wrapper" even over a
# `build.rustc-wrapper` config value, which `unset` leaves in force. A cargo call that
# probes or compiles with rustc (not `cargo fmt`) runs `$RUSTC_WRAPPER rustc -vV`, so
# even a call with nothing to compile starts the one per-user sccache server, and that
# server keeps the TMPDIR of the call that started it: a lane scratch the lane then
# deletes (docs/troubleshooting.md). tests/it/gate.rs pins it with a fake wrapper.
SIZING := '''
    export RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WRAPPER=
    lane_threads=16
    loaded_threads=8
    load_ceiling=14
    plain() { [[ "$1" =~ ^[1-9][0-9]{0,2}$ ]]; }
    load1_now() {
        local load
        load="$(LC_ALL=C uptime 2>/dev/null | sed -n 's/.*load averages*: *\([0-9.]*\).*/\1/p' | head -n 1 || true)"
        if [[ "$load" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then printf '%s' "$load"; fi
    }
    load_above() { LC_ALL=C awk -v load="$1" -v ceiling="$load_ceiling" 'BEGIN { exit !(load + 0 > ceiling + 0) }'; }
    size_counts() {
        local ceiling budget=""
        if [[ -n "${AE_TEST_THREADS:-}" ]] && ! plain "$AE_TEST_THREADS"; then
            echo "Error: AE_TEST_THREADS must be one positive integer of at most three digits, got: $AE_TEST_THREADS" >&2
            exit 2
        fi
        if [[ -n "${AE_TEST_JOBS:-}" ]] && ! plain "$AE_TEST_JOBS"; then
            echo "Error: AE_TEST_JOBS must be one positive integer of at most three digits, got: $AE_TEST_JOBS" >&2
            exit 2
        fi
        if [[ -z "${AE_TEST_THREADS:-}" || -z "${AE_TEST_JOBS:-}" ]] && command -v jobs-budget >/dev/null 2>&1; then
            budget="$(jobs-budget 2>/dev/null || true)"
            plain "$budget" || budget=""
        fi
        if [[ -n "${AE_TEST_THREADS:-}" ]]; then
            threads="$AE_TEST_THREADS"
            threads_from=override
        else
            threads_from=budget
            ceiling="$budget"
            if [[ -z "$budget" ]]; then
                threads_from=cpus
                ceiling="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
                [[ "$ceiling" =~ ^[0-9]+$ ]] || ceiling=8
            fi
            threads=$((ceiling < lane_threads ? ceiling : lane_threads))
            if [[ -n "${NEXTEST_TEST_THREADS:-}" ]]; then
                if ! plain "$NEXTEST_TEST_THREADS"; then
                    threads_from="$threads_from; env $NEXTEST_TEST_THREADS ignored"
                elif ((NEXTEST_TEST_THREADS < threads)); then
                    threads_from="env; $threads_from $ceiling"
                    threads="$NEXTEST_TEST_THREADS"
                else
                    threads_from="$threads_from; env $NEXTEST_TEST_THREADS"
                fi
            fi
        fi
        export NEXTEST_TEST_THREADS="$threads"
        if [[ -n "${AE_TEST_JOBS:-}" ]]; then
            jobs_from=override
            jobs="$AE_TEST_JOBS"
            export CARGO_BUILD_JOBS="$jobs"
        elif [[ -n "$budget" ]]; then
            jobs_from=budget
            jobs="$budget"
            if [[ -n "${CARGO_BUILD_JOBS:-}" ]]; then
                if ! plain "$CARGO_BUILD_JOBS"; then
                    jobs_from="budget; env $CARGO_BUILD_JOBS ignored"
                elif ((CARGO_BUILD_JOBS < budget)); then
                    jobs_from="env; budget $budget"
                    jobs="$CARGO_BUILD_JOBS"
                else
                    jobs_from="budget; env $CARGO_BUILD_JOBS"
                fi
            fi
            export CARGO_BUILD_JOBS="$jobs"
        else
            jobs_from="cargo default"
            [[ -z "${CARGO_BUILD_JOBS:-}" ]] || jobs_from=env
        fi
    }
    size_say() {
        echo "$1: nextest threads $NEXTEST_TEST_THREADS ($threads_from), cargo jobs ${CARGO_BUILD_JOBS:-unset} ($jobs_from)" >&2
    }
    size_load() {
        local load1 why
        [[ "$threads_from" != override ]] || return 0
        load1="$(load1_now)"
        if [[ -z "$load1" ]]; then
            why="load1 cannot be read"
        elif load_above "$load1"; then
            why="load1 $load1 is above $load_ceiling"
        else
            return 0
        fi
        if ((threads > loaded_threads)); then
            threads_from="$threads_from; $why, was $threads, capped"
            threads="$loaded_threads"
            export NEXTEST_TEST_THREADS="$threads"
        fi
        echo "$1: $why: nextest threads at most $loaded_threads" >&2
    }
'''

# THE GATE LEDGER (P1d): what a whole green gate cost, in test-seconds, so a slice
# that adds ten minutes of waiting shows in the next report instead of surfacing a
# month later. After nextest AND the doctests passed, a WHOLE run (no filter) of
# the test lane reads the JUnit report nextest wrote (`.config/nextest.toml`
# `[profile.default.junit]`, no new dependency; the lane deletes the old file
# first, so a crashed run cannot be read as a new one) and prints, on stderr:
# the test count, the wall time, the TEST-SECONDS (the sum of the per-test times),
# the THREAD-SECONDS (wall time x threads, so the gap is the time a thread sat
# idle), the 30 slowest tests, and the delta against the previous entry. It then
# appends one tab-separated line to `<main checkout>/.local/gate-ledger.tsv`, found
# like the lane lock (`git rev-parse --git-common-dir`, so a linked worktree writes
# the one ledger). A filtered run prints and writes nothing, and a red run never
# reaches this. THE LEDGER IS A REPORT AND NEVER A FAILURE: every problem (no
# report, no git, an unwritable `.local`) is one `note:` line and the lane's exit
# status stays the gate's.
#
# ledger_report <junit file> <threads>
LEDGER := '''
    ledger_report() {
        local junit="$1" threads="$2" common root ledger rows wall stats head dirty utc prev count sum span
        if [[ ! -f "$junit" || -L "$junit" ]]; then
            echo "note: gate ledger skipped: no JUnit report at $junit" >&2
            return 0
        fi
        rows="$(awk '
            /<testcase / {
                name = ""; cls = ""; secs = 0
                if (match($0, / name="[^"]*"/)) name = substr($0, RSTART + 7, RLENGTH - 8)
                if (match($0, / classname="[^"]*"/)) cls = substr($0, RSTART + 12, RLENGTH - 13)
                if (match($0, / time="[0-9.]+"/)) secs = substr($0, RSTART + 7, RLENGTH - 8)
                printf "%s\t%s\t%s\n", secs, cls, name
            }' "$junit")"
        wall="$(awk '/<testsuites / { if (match($0, / time="[0-9.]+"/)) { print substr($0, RSTART + 7, RLENGTH - 8); exit } }' "$junit")"
        if [[ -z "$rows" || ! "$wall" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
            echo "note: gate ledger skipped: the JUnit report at $junit names no tests or no wall time" >&2
            return 0
        fi
        stats="$(printf '%s\n' "$rows" | awk -F'\t' -v wall="$wall" -v threads="$threads" 'NF == 3 { n++; sum += $1 } END { printf "%d\t%.1f\t%.1f\t%.1f", n, wall, sum, wall * threads }')"
        IFS=$'\t' read -r count wall sum span <<<"$stats"
        common="$(git rev-parse --git-common-dir 2>/dev/null || true)"
        common="$(cd -P "$common" 2>/dev/null && pwd -P || true)"
        if [[ -z "$common" || "${common##*/}" != .git ]]; then
            echo "note: gate ledger skipped: not inside a git checkout" >&2
            return 0
        fi
        root="${common%/.git}"
        ledger="$root/.local/gate-ledger.tsv"
        head="$(git rev-parse --short HEAD 2>/dev/null || echo none)"
        dirty=clean
        [[ -z "$(git status --porcelain 2>/dev/null || true)" ]] || dirty=dirty
        utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        prev=""
        if [[ -f "$ledger" && ! -L "$ledger" ]]; then
            prev="$(grep -v '^#' "$ledger" 2>/dev/null | tail -n 1 || true)"
        fi
        echo "gate ledger: $count tests, wall ${wall} s on $threads threads" >&2
        echo "gate ledger: test-seconds $sum, thread-seconds $span (the threads ran tests $(awk -v a="$sum" -v b="$span" 'BEGIN { printf "%.1f", (b > 0 ? 100 * a / b : 0) }')% of the time)" >&2
        if [[ -n "$prev" ]]; then
            echo "$prev" | awk -F'\t' -v n="$count" -v w="$wall" -v s="$sum" -v t="$span" '
                function pct(now, was) { return was > 0 ? sprintf("%+.1f%%", 100 * (now - was) / was) : "n/a" }
                { printf "gate ledger: delta vs %s (%s %s): tests %+d, wall %+.1f s (%s), test-seconds %+.1f (%s), thread-seconds %+.1f (%s)\n", $1, $2, $3, n - $5, w - $6, pct(w, $6), s - $7, pct(s, $7), t - $8, pct(t, $8) }' >&2
        else
            echo "gate ledger: first entry, no earlier run to compare" >&2
        fi
        echo "gate ledger: the 30 slowest tests (seconds, binary, test)" >&2
        { printf '%s\n' "$rows" | sort -t "$(printf '\t')" -k1,1nr | head -n 30 | awk -F'\t' '{ printf "  %9.3f  %s  %s\n", $1, $2, $3 }' >&2; } || true
        if ! mkdir -p "$root/.local" 2>/dev/null || [[ -L "$ledger" ]]; then
            echo "note: gate ledger entry not written: $root/.local is not usable" >&2
            return 0
        fi
        if [[ ! -e "$ledger" ]]; then
            printf '# utc\thead\ttree\tthreads\ttests\twall_s\ttest_s\tthread_s\n' >"$ledger" 2>/dev/null || true
        fi
        if printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$utc" "$head" "$dirty" "$threads" "$count" "$wall" "$sum" "$span" >>"$ledger" 2>/dev/null; then
            echo "gate ledger: appended to $ledger" >&2
        else
            echo "note: gate ledger entry not written: $ledger is not writable" >&2
        fi
        return 0
    }
'''

# THE GATE RECEIPT (P2): the proof that a WHOLE green gate ran on THIS tree, so a
# release does not run the same nextest a second time. The writer is the tail of
# `rust-check` and the reader is `release`; both interpolate this one helper, so
# the writer and the reader cannot disagree about what a fact is.
#
# A receipt is one file, `<main checkout>/.local/gate-receipts/<tree hash>`: the
# tree hash of the clean committed tree, so it survives a fast-forward merge, and
# the main checkout's `.local` (found like the lane lock), so a gate run in a
# slice worktree is the one a release in the main checkout finds. It holds
# fourteen `key=value` lines in a fixed order, printable ASCII only, one LF each:
#
#   version tree head lock_hash toolchain_pin rustc nextest tmux harness
#   inventory inventory_count scratch host time
#
# `harness` is seven `name=first line of --version` entries (or `name=absent`),
# recorded only: no gate step runs a real harness. `inventory` is the sha256 of the
# sorted `cargo nextest list` lines and `inventory_count` their number.
#
# THE RELEASE REUSES A RECEIPT ONLY WHEN IT PARSES STRICTLY, every deciding fact
# equals its live value (all but `harness` and `time`), and the receipt is not
# newer than now and at most 24 h old. Anything else, a file that is missing,
# unreadable, malformed or stale, is a full gate: a receipt is hostile persisted
# state and a doubtful one never causes a skip. It skips ONLY nextest.
#
# bash 3.2 safe (a Mac's /bin/bash): no associative arrays, no mapfile.
RECEIPT := '''
    receipt_keys=(version tree head lock_hash toolchain_pin rustc nextest tmux harness inventory inventory_count scratch host time)
    receipt_harnesses=(claude codex gemini agy grok muse opencode)
    receipt_note() { echo "gate-receipt: $*" >&2; }
    # printable ASCII with no edge blanks: what the strict reader accepts
    receipt_clean() { LC_ALL=C tr -cd '[:print:]' | sed -e 's/^ *//' -e 's/ *$//'; }
    receipt_sha() {
        if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi
    }
    # the first line a command prints, cut in the shell: a reader that stops
    # early hands the writer a broken pipe under pipefail
    receipt_first() {
        local out
        out="$("$@" 2>/dev/null </dev/null)" || return 1
        printf '%s' "${out%%$'\n'*}" | receipt_clean
    }
    # the main checkout's root, from `git rev-parse --git-common-dir`
    receipt_root() {
        local common
        common="$(cd -P "$1" 2>/dev/null && pwd -P)" || return 1
        [[ "${common##*/}" == .git ]] || return 1
        printf '%s' "${common%/.git}"
    }
    # the filesystem kind under the lane's scratch base, one token
    receipt_scratch() {
        local dir="${AE_TEST_TMPDIR:-/tmp}" kind="" point
        if [[ "$(uname -s)" == Darwin ]]; then
            point="$(df -P "$dir" 2>/dev/null | awk 'NR == 2 { print $NF }')" || point=""
            kind="$(mount 2>/dev/null | awk -v mp="$point" '
                { p = index($0, " on "); q = index($0, " (")
                  if (p > 0 && q > p && substr($0, p + 4, q - p - 4) == mp) { s = substr($0, q + 2); sub(/[,)].*/, "", s); print s; exit } }')" || kind=""
        else
            kind="$(df -T -P "$dir" 2>/dev/null | awk 'NR == 2 { print $2 }')" || kind=""
        fi
        kind="$(printf '%s' "$kind" | tr -cd '[:alnum:]._-')"
        printf '%s' "${kind:-unknown}"
    }
    # one harness' version for the record, bounded: a binary that hangs must not hang the gate
    receipt_harness_version() {
        local bound out
        command -v "$1" >/dev/null 2>&1 || { printf 'absent'; return 0; }
        bound="$(command -v timeout || command -v gtimeout || true)"
        [[ -n "$bound" ]] || { printf 'unknown'; return 0; }
        out="$("$bound" 5 "$1" --version 2>/dev/null </dev/null)" || out=""
        out="$(printf '%s' "${out%%$'\n'*}" | receipt_clean | tr ';' ',')"
        printf '%s' "${out:-unknown}"
    }
    # the test inventory, listed once per shell: receipt_inv (sha256) and receipt_inv_count
    receipt_inventory() {
        local list
        [[ -z "${receipt_inv:-}" ]] || return 0
        list="$(cargo nextest list --locked --all-features --message-format oneline 2>/dev/null | LC_ALL=C sort)" || list=""
        if [[ -z "$list" ]]; then
            receipt_why="cargo nextest list printed no tests"
            return 1
        fi
        receipt_inv="$(printf '%s\n' "$list" | receipt_sha | cut -d' ' -f1)"
        receipt_inv_count="$(printf '%s\n' "$list" | wc -l | tr -d ' ')"
    }
    # receipt_fact <key>: the LIVE value of one fact in $fact, or return 1 with $receipt_why
    receipt_fact() {
        local name
        receipt_why=""
        fact=""
        case "$1" in
            version) fact=1 ;;
            tree) fact="$(git rev-parse 'HEAD^{tree}' 2>/dev/null)" || fact="" ;;
            head) fact="$(git rev-parse HEAD 2>/dev/null)" || fact="" ;;
            lock_hash) fact="$(receipt_sha Cargo.lock 2>/dev/null | cut -d' ' -f1)" || fact="" ;;
            toolchain_pin) fact="$(receipt_sha rust-toolchain.toml 2>/dev/null | cut -d' ' -f1)" || fact="" ;;
            rustc) fact="$(receipt_first rustc --version)" || fact="" ;;
            nextest) fact="$(receipt_first cargo nextest --version)" || fact="" ;;
            tmux) fact="$(receipt_first tmux -V)" || fact="" ;;
            harness)
                for name in "${receipt_harnesses[@]}"; do
                    fact="${fact:+$fact;}$name=$(receipt_harness_version "$name")"
                done
                ;;
            inventory) if receipt_inventory; then fact="$receipt_inv"; fi ;;
            inventory_count) if receipt_inventory; then fact="$receipt_inv_count"; fi ;;
            scratch) fact="$(receipt_scratch)" || fact="" ;;
            host) fact="$(receipt_first hostname -s)" || fact="" ;;
            time) fact="$(date -u +%Y-%m-%dT%H:%M:%SZ)" || fact="" ;;
        esac
        if [[ -z "$fact" ]]; then
            [[ -n "$receipt_why" ]] || receipt_why="the live $1 fact cannot be read"
            return 1
        fi
    }
    # the whole receipt for the live tree, or return 1 naming the fact that cannot be read
    receipt_body() {
        local key out=""
        for key in "${receipt_keys[@]}"; do
            receipt_fact "$key" || return 1
            out="$out$key=$fact"$'\n'
        done
        printf '%s' "$out"
    }
    # UTC YYYY-MM-DDTHH:MM:SSZ to epoch seconds; nothing when it is not a real instant
    receipt_epoch() {
        LC_ALL=C awk -v t="$1" 'BEGIN {
            if (t !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z$/) exit
            y = substr(t, 1, 4) + 0; m = substr(t, 6, 2) + 0; d = substr(t, 9, 2) + 0
            h = substr(t, 12, 2) + 0; mi = substr(t, 15, 2) + 0; s = substr(t, 18, 2) + 0
            if (y < 1970 || m < 1 || m > 12 || h > 23 || mi > 59 || s > 60) exit
            leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
            dim = (m == 2) ? (leap ? 29 : 28) : ((m == 4 || m == 6 || m == 9 || m == 11) ? 30 : 31)
            if (d < 1 || d > dim) exit
            if (m <= 2) y--
            era = int(y / 400); yoe = y - era * 400
            doy = int((153 * ((m + 9) % 12) + 2) / 5) + d - 1
            doe = yoe * 365 + int(yoe / 4) - int(yoe / 100) + doy
            printf "%d", (era * 146097 + doe - 719468) * 86400 + h * 3600 + mi * 60 + s
        }'
    }
    # does <value> satisfy the grammar of <key>?
    receipt_value_ok() {
        local key="$1" value="$2" re i name part rest
        case "$key" in
            version) [[ "$value" == 1 ]] ;;
            tree | head) re='^[0-9a-f]{40}$'; [[ "$value" =~ $re ]] ;;
            lock_hash | toolchain_pin | inventory) re='^[0-9a-f]{64}$'; [[ "$value" =~ $re ]] ;;
            inventory_count) re='^(0|[1-9][0-9]{0,17})$'; [[ "$value" =~ $re ]] ;;
            time) [[ -n "$(receipt_epoch "$value")" ]] ;;
            rustc | nextest | tmux | scratch | host) [[ -n "$value" && "$value" != " "* && "$value" != *" " ]] ;;
            harness)
                i=0
                rest="$value"
                for name in "${receipt_harnesses[@]}"; do
                    i=$((i + 1))
                    if ((i < ${#receipt_harnesses[@]})); then
                        [[ "$rest" == *";"* ]] || return 1
                        part="${rest%%;*}"
                        rest="${rest#*;}"
                    else
                        part="$rest"
                        [[ "$part" != *";"* ]] || return 1
                    fi
                    [[ "$part" == "$name="?* ]] || return 1
                done
                ;;
            *) return 1 ;;
        esac
    }
    # THE STRICT READER: receipt_parse <file> leaves every value in r_<key> and returns 0,
    # or returns 1 with $receipt_why (missing, unreadable, malformed ...). Exactly the
    # fourteen keys, once each, in order, one non-blank line apiece.
    receipt_parse() {
        local file="$1" line key value n=0 stray
        receipt_why=""
        if [[ ! -e "$file" && ! -L "$file" ]]; then receipt_why="missing"; return 1; fi
        if [[ -L "$file" || ! -f "$file" || ! -r "$file" ]]; then receipt_why="unreadable"; return 1; fi
        if [[ ! -s "$file" ]]; then receipt_why="malformed: the receipt is empty"; return 1; fi
        stray="$(LC_ALL=C tr -d '[:print:]\n' <"$file" | wc -c | tr -d ' ')" || stray=1
        if [[ "$stray" != 0 ]]; then receipt_why="malformed: bytes outside printable ASCII and LF"; return 1; fi
        while IFS= read -r line || [[ -n "$line" ]]; do
            if ((n >= ${#receipt_keys[@]})); then receipt_why="malformed: more than ${#receipt_keys[@]} lines"; return 1; fi
            key="${receipt_keys[$n]}"
            if [[ "$line" != "$key="* ]]; then receipt_why="malformed: line $((n + 1)) is not $key=..."; return 1; fi
            value="${line#"$key="}"
            if ! receipt_value_ok "$key" "$value"; then receipt_why="malformed: the value of $key breaks its grammar"; return 1; fi
            printf -v "r_$key" '%s' "$value"
            n=$((n + 1))
        done <"$file"
        if ((n != ${#receipt_keys[@]})); then receipt_why="malformed: $n lines, not ${#receipt_keys[@]}"; return 1; fi
    }
    # THE MATCH: receipt_match <file> returns 0 when the receipt may stand in for nextest,
    # else 1 with $receipt_why. On a match $receipt_age is its age in seconds. The cheap
    # facts first and the inventory last, so a receipt that already fails never pays for
    # `cargo nextest list`.
    receipt_match() {
        local key name stamp now
        receipt_parse "$1" || return 1
        for key in tree head lock_hash toolchain_pin rustc nextest tmux scratch host; do
            receipt_fact "$key" || return 1
            name="r_$key"
            if [[ "$fact" != "${!name}" ]]; then receipt_why="$key differs (receipt: ${!name}; now: $fact)"; return 1; fi
        done
        stamp="$(receipt_epoch "$r_time")"
        now="$(date -u +%s)"
        receipt_age=$((now - stamp))
        if ((receipt_age < 0)); then receipt_why="the receipt is dated in the future"; return 1; fi
        if ((receipt_age > 86400)); then receipt_why="stale: $((receipt_age / 3600)) h old, the limit is 24 h"; return 1; fi
        for key in inventory inventory_count; do
            receipt_fact "$key" || return 1
            name="r_$key"
            if [[ "$fact" != "${!name}" ]]; then receipt_why="$key differs (receipt: ${!name}; now: $fact)"; return 1; fi
        done
    }
'''

# THE MAC SUBSET MANIFEST READER of `just test-macos`. It renders exactly the filterset the
# guard renders (`filterset` in tests/it/gate/macos_subset.rs, which a frozen spec compares
# with this output), and it refuses what the guard's parser refuses: a byte outside printable
# ASCII, tab and LF, a line of no known kind, a `history` line without an issue number, and a
# manifest without exactly one `unit` line. Line kinds, `#` starting a comment:
#
#   unit                                  every unit test: kind(lib) | kind(bin)
#   module <path>                         an integration module and everything under it
#   test <path>                           one integration test, exactly
#   helper <file>::<item> = <test>        the one test pinned for a shared boundary helper
#   history <path> # <issue>              a Mac-only regression, as a module and as a test
#
# bash 3.2 safe (a Mac's /bin/bash): no associative arrays, no mapfile, no empty "${a[@]}".
SUBSET := '''
    # subset_render <manifest>: the filterset in $subset_filterset, or return 1 with $subset_why
    subset_render() {
        local file="$1" raw body comment issue n=0 units=0 stray modules="" exact="" seen_mod="|" seen_exact="|" terms=""
        local words=()
        local path_re='^[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*$'
        local item_re='^[A-Za-z0-9_./-]+[.]rs::[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*$'
        local issue_re='^[0-9]+$'
        subset_why=""
        subset_filterset=""
        if [[ ! -f "$file" || ! -r "$file" ]]; then subset_why="$file is missing or unreadable"; return 1; fi
        stray="$(LC_ALL=C tr -d '[:print:]\t\n' <"$file" | wc -c | tr -d ' ')" || stray=1
        if [[ "$stray" != 0 ]]; then subset_why="$file holds bytes outside printable ASCII, tab and LF"; return 1; fi
        subset_module() { [[ "$seen_mod" == *"|$1|"* ]] || { seen_mod="$seen_mod$1|"; modules="${modules:+$modules|}$1"; }; }
        subset_exact() { [[ "$seen_exact" == *"|$1|"* ]] || { seen_exact="$seen_exact$1|"; exact="${exact:+$exact | }test(=$1)"; }; }
        while IFS= read -r raw || [[ -n "$raw" ]]; do
            n=$((n + 1))
            body="${raw%%#*}"
            comment=""
            if [[ "$raw" == *"#"* ]]; then comment="${raw#*#}"; fi
            read -r -a words <<<"$body" || true
            if ((${#words[@]} == 0)); then
                continue
            elif [[ "${words[0]}" == unit && ${#words[@]} == 1 ]]; then
                units=$((units + 1))
            elif [[ "${words[0]}" == module && ${#words[@]} == 2 && "${words[1]}" =~ $path_re ]]; then
                subset_module "${words[1]}"
            elif [[ "${words[0]}" == test && ${#words[@]} == 2 && "${words[1]}" =~ $path_re ]]; then
                subset_exact "${words[1]}"
            elif [[ "${words[0]}" == helper && ${#words[@]} == 4 && "${words[1]}" =~ $item_re && "${words[2]}" == "=" && "${words[3]}" =~ $path_re ]]; then
                subset_exact "${words[3]}"
            elif [[ "${words[0]}" == history && ${#words[@]} == 2 && "${words[1]}" =~ $path_re ]]; then
                issue="${comment#"${comment%%[![:space:]]*}"}"
                issue="${issue%"${issue##*[![:space:]]}"}"
                if [[ ! "$issue" =~ $issue_re ]]; then subset_why="line $n: a history line needs its issue number after #: $raw"; return 1; fi
                subset_module "${words[1]}"
                subset_exact "${words[1]}"
            else
                subset_why="line $n is not a manifest line: $raw"
                return 1
            fi
        done <"$file"
        if ((units != 1)); then subset_why="$file needs exactly one unit line, found $units"; return 1; fi
        if [[ -n "$modules" ]]; then terms="test(/^($modules)::/)"; fi
        if [[ -n "$exact" ]]; then terms="${terms:+$terms | }$exact"; fi
        subset_filterset="kind(lib) | kind(bin)"
        if [[ -n "$terms" ]]; then subset_filterset="$subset_filterset | (binary_id(ae::it) & ($terms))"; fi
    }
'''

# THE RELEASE PROOF HELPERS, shared by `release-proof` and the `release` recipe that calls it.
# A refusal is one `release-proof: refused: <why>` line on stderr and exit 1. The identity is
# HEAD and its tree of a CLEAN checkout: staged, unstaged and untracked changes refuse, and so
# does any git read that fails. The destination is read from the environment and validated
# before any ssh: AE_RELEASE_HP is `host` or `user@host` and AE_RELEASE_HP_ROOT an absolute
# path without a `..` segment. Neither has a default. `local` is the human-only override.
PROOF := '''
    proof_refuse() { echo "release-proof: refused: $*" >&2; exit 1; }
    # proof_identity: the clean checkout's HEAD and tree in $id_head and $id_tree, or return 1 with $proof_why
    proof_identity() {
        local status sha_re='^[0-9a-f]{40}$'
        id_head=""
        id_tree=""
        status="$(git status --porcelain=v1 --untracked-files=all 2>/dev/null)" || { proof_why="git status cannot be read"; return 1; }
        if [[ -n "$status" ]]; then proof_why="the working tree is not clean (staged, unstaged or untracked changes)"; return 1; fi
        id_head="$(git rev-parse --verify -q HEAD 2>/dev/null)" || { proof_why="git cannot read HEAD"; return 1; }
        id_tree="$(git rev-parse --verify -q 'HEAD^{tree}' 2>/dev/null)" || { proof_why="git cannot read the tree of HEAD"; return 1; }
        if [[ ! "$id_head" =~ $sha_re || ! "$id_tree" =~ $sha_re ]]; then proof_why="git named no HEAD or tree hash"; return 1; fi
    }
    # proof_same <head> <tree>: still clean and on that HEAD and tree, or return 1 with $proof_why
    proof_same() {
        proof_identity || return 1
        if [[ "$id_head" != "$1" ]]; then proof_why="HEAD moved from $1 to $id_head"; return 1; fi
        if [[ "$id_tree" != "$2" ]]; then proof_why="the tree moved from $2 to $id_tree"; return 1; fi
    }
    # proof_destination: a usable AE_RELEASE_HP and AE_RELEASE_HP_ROOT, or return 1 with $proof_why
    proof_destination() {
        local LC_ALL=C hp="${AE_RELEASE_HP:-}" root="${AE_RELEASE_HP_ROOT:-}"
        local user_re='^[A-Za-z0-9_][A-Za-z0-9._-]*@[A-Za-z0-9][A-Za-z0-9.-]*$' host_re='^[A-Za-z0-9][A-Za-z0-9.-]*$' root_re='^/[A-Za-z0-9._/-]+$'
        if [[ -z "$hp" ]]; then proof_why="AE_RELEASE_HP is unset: name the hp as host or user@host, there is no default"; return 1; fi
        if [[ "$hp" == local ]]; then return 0; fi
        if [[ ! "$hp" =~ $user_re && ! "$hp" =~ $host_re ]]; then proof_why="AE_RELEASE_HP is not host or user@host: $hp"; return 1; fi
        if [[ -z "$root" ]]; then proof_why="AE_RELEASE_HP_ROOT is unset: name the hp main checkout, there is no default"; return 1; fi
        if [[ ! "$root" =~ $root_re || "$root/" == *"/../"* ]]; then proof_why="AE_RELEASE_HP_ROOT is not an absolute path without .. segments: $root"; return 1; fi
    }
'''

# THE PARALLEL RELEASE PROOF (docs/development.md, "Release proof"): `release-proof hp=run`
# when the hp has no receipt for the tree yet. The hp whole run and the Mac subset run at the
# same time, and the release goes on only when both are green for this exact commit.
#
# The Mac side, hp_parallel, runs in release-proof. The hp side is four functions, which reach
# the hp as text on the stdin of `ssh <hp> bash -s` (declare -f), so the hp login shell never
# parses them:
#   hp_pre     the missing-only check: the receipt is provably absent and nothing stands in the way
#   hp_launch  starts hp_leg detached (setsid nohup) and prints its pid
#   hp_leg     claims the hp lane lock, ONLY THEN prepares the warm worktree <root>-wt/release
#              at exactly this commit, runs `timeout -k 60 3600 just test` there under
#              AE_LANE_CLAIM=<lock> (another worktree than the claim), releases the
#              lock on a normal end, and writes <root>/.local/release/<run>.exit last
#   hp_poll    prints that exit record, `running`, or `gone`
# The hp leg has no remote cancel: once started it always runs to its end. A Mac refusal (a red
# subset, three lost polls, an interrupt) only stops waiting; the leg finishes alone, and its
# receipt serves a later release of the same tree. On its own timeout or a signal the leg keeps
# the hp lock and writes <run>.suspect; the lead clears that lock after checking. The staging
# ref refs/ae-release/<head> in the hp root carries the commit there; the leg deletes it once
# it has checked the commit out, or when it ends earlier. The Mac lane lock stays with the
# caller of `just release`.
HP_LEG := '''
    hp_pre() {
        local root=$1 tree=$2 head=$3 lock=$4 g r l owner
        g="$root/.local/gate-receipts"
        r="$g/$tree"
        l="$root/.local"
        [ -d "$root" ] && [ -r "$root" ] && [ -x "$root" ] || exit 5
        if [ -L "$l" ] || [ -L "$g" ] || [ -L "$r" ]; then exit 4; fi
        if [ -e "$g" ]; then
            [ -d "$g" ] && [ -r "$g" ] && [ -x "$g" ] || exit 5
        elif [ -e "$l" ]; then
            [ -d "$l" ] && [ -r "$l" ] && [ -x "$l" ] || exit 5
        fi
        if [ -e "$r" ]; then exit 3; fi
        git -C "$root" rev-parse --git-dir >/dev/null 2>&1 || exit 6
        if git -C "$root" rev-parse -q --verify "refs/ae-release/$head" >/dev/null 2>&1; then exit 7; fi
        owner="$root/.local/locks/gate/owner"
        if [ -f "$owner" ] && [ "$(head -n 1 "$owner" | cut -d ' ' -f 1)" = "$lock" ]; then exit 8; fi
        exit 0
    }
    hp_leg() {
        local root=$1 head=$2 tree=$3 lock=$4 runid=$5
        local dir="$root/.local/release" wt="$root-wt/release" ref="refs/ae-release/$head"
        local gate=prep held=no status common
        say() { echo "$(date -u +%FT%TZ) leg $runid: $*"; }
        finish() {
            local state=released
            [ "$held" = no ] || state=held
            printf 'gate=%s run=%s head=%s tree=%s lock=%s\n' "$gate" "$runid" "$head" "$tree" "$state" >"$dir/$runid.exit.tmp" &&
                mv "$dir/$runid.exit.tmp" "$dir/$runid.exit"
        }
        suspect() {
            printf 'run=%s\nhead=%s\ntree=%s\nlock=%s\nstatus=%s\nwhy=%s\nThe hp lane lock %s stays held: the lead clears it after checking that no process of this run is left.\n' \
                "$runid" "$head" "$tree" "$lock" "$gate" "$1" "$lock" >"$dir/$runid.suspect"
        }
        drop_ref() { git -C "$root" update-ref -d "$ref" "$head" 2>/dev/null || say "note: $ref could not be deleted"; }
        unlock() { if just -f "$root/justfile" -d "$root" lane-lock release "$lock"; then held=no; fi; }
        # Whatever ends the leg, its exit record exists afterwards (written last, by finish).
        leg_exit() { [ -e "$dir/$runid.exit" ] || finish; }
        trap leg_exit EXIT
        trap 'gate=signal; suspect "a signal stopped the leg"; exit 1' INT TERM
        say "claiming the hp lane as $lock"
        if ! just -f "$root/justfile" -d "$root" lane-lock claim "$lock" 1800; then
            gate=lock
            drop_ref
            exit 1
        fi
        held=yes
        say "preparing $wt at $head"
        if [ -e "$wt" ] || [ -L "$wt" ]; then
            common="$(git -C "$wt" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
            if [ "$common" != "$(cd -P "$root" && pwd -P)/.git" ] || [ -n "$(git -C "$wt" status --porcelain --untracked-files=all 2>&1)" ]; then
                say "refused: $wt is not a clean worktree of $root"
                gate=worktree
            fi
        elif ! git -C "$root" worktree add -q --detach "$wt" "$head"; then
            gate=worktree
        fi
        if [ "$gate" = prep ]; then
            if ! git -C "$wt" checkout -q --detach "$head" ||
                [ "$(git -C "$wt" rev-parse HEAD 2>/dev/null)" != "$head" ] ||
                [ "$(git -C "$wt" rev-parse 'HEAD^{tree}' 2>/dev/null)" != "$tree" ] ||
                [ -n "$(git -C "$wt" status --porcelain --untracked-files=all 2>&1)" ]; then
                say "refused: $wt is not clean on $head and tree $tree"
                gate=worktree
            fi
        fi
        drop_ref
        if [ "$gate" != prep ]; then
            unlock
            exit 1
        fi
        gate=running
        say "running just test"
        status=0
        # The claim was taken in $root and the run is in $wt, another worktree: it names its claim (#343).
        (cd "$wt" && AE_LANE_CLAIM="$lock" exec timeout -k 60 3600 just test) || status=$?
        gate=$status
        # 124 is the leg's own timeout, 128 and above a signal (timeout reports its child's
        # signal as 128+n, and 137 is its KILL). Either way part of the run may still live, so
        # the lock stays held for the lead; only an ordinary end, green or red, releases it.
        if [ "$status" -eq 124 ] || [ "$status" -ge 128 ]; then
            say "a timeout or a signal stopped just test (exit $status): the lane lock stays held"
            suspect "a timeout or a signal stopped just test (exit $status)"
            exit 1
        fi
        say "just test ended with exit $status"
        unlock
        exit 0
    }
    hp_launch() {
        local root=$1 head=$2 tree=$3 lock=$4 runid=$5 path=$6 dir file
        dir="$root/.local/release"
        mkdir -p "$dir" || exit 6
        for file in "$dir/$runid".*; do
            [ ! -e "$file" ] || exit 9
        done
        if [ "$path" != - ]; then
            PATH="$path:$PATH"
            export PATH
        fi
        setsid nohup bash -c "$(declare -f hp_leg); hp_leg \"\$@\"" hp-leg "$root" "$head" "$tree" "$lock" "$runid" \
            </dev/null >"$dir/$runid.log" 2>&1 &
        echo "$!" >"$dir/$runid.pid"
        echo "$!"
    }
    hp_poll() {
        local dir="$1/.local/release" runid=$2 pid
        if [ -f "$dir/$runid.exit" ]; then cat "$dir/$runid.exit"; return 0; fi
        pid="$(cat "$dir/$runid.pid" 2>/dev/null || true)"
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then echo running; return 0; fi
        if [ -f "$dir/$runid.exit" ]; then cat "$dir/$runid.exit"; return 0; fi
        echo gone
    }
    # hp_remote <function> <args…>: one bounded `ssh <hp> bash -s`, its stdin the function's text.
    hp_remote() {
        local fn=$1
        shift
        { declare -f "$fn"; [ "$fn" != hp_launch ] || declare -f hp_leg; echo "$fn \"\$@\""; } |
            "$bound" 30 ssh -o BatchMode=yes -o ConnectTimeout=10 -- "$AE_RELEASE_HP" bash -s -- "$@"
    }
    hp_parallel() {
        local lock="release-${head:0:12}" ref="refs/ae-release/$head" runid code dropped mac_status lost=0 line deadline path
        local exit_re='^gate=([0-9]+|prep|lock|worktree|running|signal) run=([^ ]+) head=([0-9a-f]{40}) tree=([0-9a-f]{40}) lock=(released|held)$'
        runid="${head:0:12}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
        path="${AE_RELEASE_HP_PATH:-}"
        if [[ -n "$path" && ! "$path" =~ ^/[A-Za-z0-9._/-]+(:/[A-Za-z0-9._/-]+)*$ ]]; then
            proof_refuse "AE_RELEASE_HP_PATH is not a ':'-separated list of absolute directories: $path"
        fi
        echo "release-proof: the hp has no receipt for tree $tree yet: its whole run starts now, beside the Mac subset (run $runid)"
        code=0
        hp_remote hp_pre "$AE_RELEASE_HP_ROOT" "$tree" "$head" "$lock" >"$reply_dir/pre" 2>"$reply_dir/err" || code=$?
        case "$code" in
            0) ;;
            3) proof_refuse "the hp has a receipt for tree $tree that cannot be read" ;;
            4) proof_refuse "the hp receipt path for tree $tree runs through a symbolic link" ;;
            5) proof_refuse "a directory on the hp receipt path under $AE_RELEASE_HP_ROOT is not a readable, searchable directory" ;;
            7) proof_refuse "the hp already has $ref: a release leg for this commit is running, or one ended without deleting it; check it, then delete the ref by hand" ;;
            8) proof_refuse "the hp lane is held by $lock: a release leg for this commit is running" ;;
            124) proof_refuse "the hp did not answer the missing-only check within 30 s" ;;
            255) proof_refuse "ssh could not reach $AE_RELEASE_HP for the missing-only check (exit 255)" ;;
            *) sed 's/^/release-proof: ssh: /' "$reply_dir/err" >&2
               proof_refuse "the missing-only check on the hp failed (exit $code)" ;;
        esac
        code=0
        GIT_SSH_COMMAND="ssh -o BatchMode=yes -o ConnectTimeout=10" "$bound" 120 \
            git push -q "ssh://$AE_RELEASE_HP$AE_RELEASE_HP_ROOT" "$head:$ref" >"$reply_dir/push" 2>&1 || code=$?
        if ((code != 0)); then
            sed 's/^/release-proof: git push: /' "$reply_dir/push" >&2
            proof_refuse "the commit could not be staged on the hp as $ref (exit $code)"
        fi
        code=0
        hp_remote hp_launch "$AE_RELEASE_HP_ROOT" "$head" "$tree" "$lock" "$runid" "${path:--}" >"$reply_dir/launch" 2>"$reply_dir/err" || code=$?
        case "$code" in
            0) echo "release-proof: the hp leg $runid runs as pid $(head -n 1 "$reply_dir/launch")" ;;
            124 | 255) proof_refuse "the hp start of run $runid got no answer (exit $code): the leg may be running; it ends alone and deletes $ref, and a new release refuses until then" ;;
            *) sed 's/^/release-proof: ssh: /' "$reply_dir/err" >&2
               dropped=0
               "$bound" 30 ssh -o BatchMode=yes -o ConnectTimeout=10 -- "$AE_RELEASE_HP" \
                   "git -C $AE_RELEASE_HP_ROOT update-ref -d $ref $head" >/dev/null 2>&1 || dropped=$?
               ((dropped == 0)) || echo "release-proof: $ref could not be deleted on the hp (exit $dropped); delete it by hand" >&2
               proof_refuse "the hp leg did not start (exit $code)" ;;
        esac

        # The Mac subset runs in the foreground, as it does when the hp already has its receipt:
        # an interrupt reaches it as before, and a failure or an interrupt ends this proof nonzero
        # before the bump. The hp leg runs on alone and releases its own lane.
        echo "release-proof: running the Mac boundary subset, just test-macos, while the hp runs"
        mac_status=0
        nest just test-macos || mac_status=$?
        ((mac_status == 0)) || proof_refuse "the Mac boundary subset failed (exit $mac_status); the hp leg $runid runs on alone and releases its own lane"
        proof_same "$head" "$tree" || proof_refuse "$proof_why while the Mac subset ran"
        echo "release-proof: the Mac boundary subset is green; waiting for the hp leg $runid"

        deadline=$(($(date -u +%s) + 1800 + 3660 + 120))
        while :; do
            code=0
            hp_remote hp_poll "$AE_RELEASE_HP_ROOT" "$runid" >"$reply_dir/poll" 2>"$reply_dir/err" || code=$?
            if ((code != 0)); then
                lost=$((lost + 1))
                echo "release-proof: poll $lost of 3 of the hp leg was lost (exit $code)" >&2
                ((lost < 3)) || proof_refuse "three polls of the hp leg in a row were lost; it runs on alone and releases its own lane"
            else
                lost=0
                line="$(head -n 1 "$reply_dir/poll")"
                case "$line" in
                    running) ;;
                    gone) proof_refuse "the hp leg $runid ended without an exit record" ;;
                    *)
                        [[ "$line" =~ $exit_re ]] || proof_refuse "the hp leg $runid left an exit record this proof cannot read"
                        [[ "${BASH_REMATCH[2]}" == "$runid" ]] || proof_refuse "the exit record names run ${BASH_REMATCH[2]}, not $runid"
                        [[ "${BASH_REMATCH[3]}" == "$head" && "${BASH_REMATCH[4]}" == "$tree" ]] || proof_refuse "the exit record names another commit or tree"
                        case "${BASH_REMATCH[1]}" in
                            0) ;;
                            lock) proof_refuse "the hp lane was not free within 1800 s; nothing ran there" ;;
                            worktree) proof_refuse "the warm hp worktree is not a clean worktree of $AE_RELEASE_HP_ROOT on this commit; nothing ran there" ;;
                            124 | 12[89] | 1[3-9][0-9] | 2[0-9][0-9] | signal)
                                proof_refuse "the hp leg $runid was stopped (${BASH_REMATCH[1]}); its lane lock stays held with a suspect note for the lead" ;;
                            *) proof_refuse "the hp whole suite failed (exit ${BASH_REMATCH[1]})" ;;
                        esac
                        [[ "${BASH_REMATCH[5]}" == released ]] || proof_refuse "the hp leg ended green but kept its lane lock"
                        echo "release-proof: the hp leg $runid ended green and released its lane"
                        return 0
                        ;;
                esac
            fi
            (($(date -u +%s) < deadline)) || proof_refuse "the hp leg $runid did not end within its bound"
            sleep 30
        done
    }
'''

# Cargo runs `detached` and taplo through `nest`, so a TERM to any level ends both before the
# recipe does (#342).
# Clippy (-D warnings) + taplo lint
rust-lint:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ SIZING }}
    {{ NEST }}
    {{ DETACHED }}
    size_counts
    size_say rust-lint
    detached nice -n 10 cargo clippy --locked --all-targets --all-features -- -D warnings
    nest taplo lint

# Sized by the one helper, then capped at four jobs (a small crate set). Cargo runs `detached`,
# as in rust-lint (#342).
# Type-check every fuzz bin with the product compiler, sharing the gate cache.
rust-fuzz-check:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ SIZING }}
    {{ NEST }}
    {{ DETACHED }}
    size_counts
    size_say rust-fuzz-check
    jobs="${CARGO_BUILD_JOBS:-4}"
    plain "$jobs" && [ "$jobs" -le 4 ] || jobs=4
    detached nice -n 10 cargo check --locked --manifest-path fuzz/Cargo.toml --bins --target-dir "${CARGO_TARGET_DIR:-$PWD/target}" --jobs "$jobs"

# nextest does NOT run doctests. Doctests are KEPT — they are the executable half
# of the public docs — so they get their own invocation. Dropping that second
# line silently retires a whole lane.

# One config-free named server per test-tool invocation. Every lane that can
# execute the integration target comes through here, including the mutation
# baseline and coverage.
#
# Every test scratch root (`tests/it/scratch.rs`) and every lane directory sits
# under ONE base, AE_TEST_TMPDIR or /tmp: short, because a tmux socket beneath
# it must fit AF_UNIX's 104 bytes. The lane also points TMPDIR into itself, so
# what a test, a product child or cargo-mutants' tree copy leaves in the temp
# dir leaves with the lane. NEXTEST_TEST_THREADS is the least of the free cores
# (jobs-budget, else the CPU count), the lane ceiling `lane_threads` and an
# inherited export, lowered to `loaded_threads` when load1 is above `load_ceiling`
# at the lane start or cannot be read (never a refusal), and
# CARGO_BUILD_JOBS the least of the free cores and an inherited export (without
# jobs-budget an inherited value stands, else cargo's own default); the
# lane-specific AE_TEST_THREADS / AE_TEST_JOBS replace the rule. The three heavy
# lanes run cargo at nice 10.
_tmux-isolated lane *args:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ SIZING }}
    {{ LEDGER }}
    lane="$1"
    shift
    # The stress lane (just rust-stress) refuses FIRST, before any scratch, server
    # or reap exists: a named filterset, a plain repeat count and time bound, a
    # timeout tool, and a quiet machine (load1 at or below `load_ceiling`, the same
    # start ceiling a gate caps its threads at). A load it cannot read refuses too.
    if [[ "$lane" == stress ]]; then
        if (($# != 3)); then
            echo "usage: just rust-stress <nextest filterset> [count] [seconds]" >&2
            exit 2
        fi
        stress_filter="$1"
        stress_count="$2"
        stress_secs="$3"
        if [[ -z "${stress_filter//[[:space:]]/}" || "$stress_filter" == "all()" ]]; then
            echo "Error: rust-stress repeats one named filterset (a module or a test), not the suite: '$stress_filter'" >&2
            exit 2
        fi
        [[ "$stress_count" =~ ^[1-9][0-9]{0,2}$ ]] || { echo "Error: rust-stress count must be a positive integer of at most three digits, got: $stress_count" >&2; exit 2; }
        [[ "$stress_secs" =~ ^[1-9][0-9]{0,4}$ ]] || { echo "Error: rust-stress seconds must be a positive integer of at most five digits, got: $stress_secs" >&2; exit 2; }
        stress_timeout="$(command -v timeout || command -v gtimeout || true)"
        [[ -n "$stress_timeout" ]] || { echo "Error: rust-stress needs timeout (GNU coreutils; gtimeout on a Mac with brew coreutils): the time bound is not optional" >&2; exit 2; }
        load1="$(load1_now)"
        if [[ -z "$load1" ]]; then
            echo "rust-stress: refused: load1 cannot be read from uptime; a start needs load1 at or below $load_ceiling" >&2
            exit 1
        fi
        if load_above "$load1"; then
            echo "rust-stress: refused: load1 $load1 is above $load_ceiling; start when the machine is quiet" >&2
            exit 1
        fi
    fi
    base="${AE_TEST_TMPDIR:-/tmp}"
    base_real="$(cd -P "$base" 2>/dev/null && pwd || printf '%s' "$base")"
    legacy="${TMPDIR:-/tmp}"
    # A process's start identity: its `ps -o lstart=` start time, keeping the ASCII
    # letters and digits, as tests/it/parity.rs::process_identity computes it. A pid
    # alone cannot prove who holds it once its owner ended and the number was reused.
    proc_identity() {
        LC_ALL=C ps -o lstart= -p "$1" 2>/dev/null | LC_ALL=C tr -cd 'A-Za-z0-9'
    }
    # owner_is_dead PID [RECORDED_IDENTITY]. Dead when the pid names no process, or
    # when its identity was recorded at registration and the process now holding the
    # pid has a different, readable one (the pid was reused). Fail-safe: an owner
    # with no recorded identity, or one that cannot be read now, is ALIVE, so a
    # live owner's root is never reaped on a guess.
    owner_is_dead() {
        local owner="$1" recorded="${2-}" error current
        if error="$(LC_ALL=C kill -0 "$owner" 2>&1)" || [[ "$error" != *"No such process"* ]]; then
            [[ -n "$recorded" ]] || return 1
            current="$(proc_identity "$owner" || true)"
            [[ -n "$current" && "$current" != "$recorded" ]]
            return
        fi
        return 0
    }
    # THE BOUND of the stale-scratch sweep (`reap_dead_scratch`). `bound_run <command...>` runs a
    # command as it stands, except while the sweep has set `bound_tool` (timeout or gtimeout): then
    # it runs under `<tool> -k 1 <secs>`, <secs> the least of 10 and what is left of the sweep's
    # `sweep_deadline` (a SECONDS value), and a timeout (124, or 137 after the kill) sets
    # `bound_hit`. Outside the sweep `bound_tool` is empty, so every other caller runs unchanged.
    bound_tool=""
    bound_hit=0
    sweep_deadline=0
    bound_run() {
        local secs status=0
        if [[ -z "$bound_tool" ]]; then
            "$@"
            return
        fi
        secs=$((sweep_deadline - SECONDS))
        if ((secs > 10)); then secs=10; fi
        if ((secs < 1)); then
            bound_hit=1
            return 124
        fi
        "$bound_tool" -k 1 "$secs" "$@" || status=$?
        bound_note "$status"
        return "$status"
    }
    # bound_note STATUS: while the sweep holds `bound_tool`, a timeout status (124, or 137 after the
    # kill) sets `bound_hit`; with no `bound_tool` it does nothing, so the flag never outlives the
    # sweep or reaches the unbounded callers. A command run inside a command substitution loses the
    # flag with its subshell, so its caller notes the status.
    bound_note() { if [[ -n "$bound_tool" ]]; then case "$1" in 124 | 137) bound_hit=1 ;; esac; fi; }
    # A socket whose server is already gone counts as reaped. An installed-shape
    # test server refuses kill-server (src/l2_model/session_tmux.rs): its guard is dropped
    # first, on this explicit socket, and the kill's exit status is never taken
    # as proof: the server must stop answering.
    reap_sockets() {
        local root="$1" socket sockets error failed=0 tries status=0
        shift
        sockets="$(bound_run find "$root" -type s -print 2>/dev/null)" || status=$?
        bound_note "$status"
        ((status == 0)) || return 1
        # The optional rest of the arguments is a guard command, run after the scan and immediately
        # before EACH effect on a socket, set-option and kill-server alike (either can take the
        # bound's seconds): the sweep passes its proof that the root is still the same dead owner's,
        # so a root replaced while the scan or the set-option ran never has its sockets touched.
        if (($# > 0)) && ! "$@"; then return 1; fi
        while IFS= read -r socket; do
            [[ -n "$socket" ]] || continue
            ((bound_hit == 0)) || return 1
            if (($# > 0)) && ! "$@"; then return 1; fi
            bound_run env -u TMUX -u TMUX_PANE tmux -S "$socket" set-option -su command-alias >/dev/null 2>&1 || true
            ((bound_hit == 0)) || return 1
            if (($# > 0)) && ! "$@"; then return 1; fi
            status=0
            error="$(bound_run tmux -S "$socket" kill-server 2>&1)" || status=$?
            bound_note "$status"
            if ((status != 0)) && [[ "$error" != *"no server running"* ]]; then failed=1; fi
            tries=20
            while bound_run env -u TMUX -u TMUX_PANE tmux -S "$socket" list-sessions >/dev/null 2>&1; do
                ((--tries)) || { failed=1; break; }
                sleep 0.1
            done
        done <<<"$sockets"
        return "$failed"
    }
    # The ONE deletion rule, as tests/it/scratch.rs::owned_root: a registered
    # target is deleted only as <base>/ae-it-<owner>/<one name> (or gone), no
    # link, and then as the whole ae-it-<owner>, left in $owned. Any other
    # target is only killed.
    owned_root() {
        local owner="$1" target="$2" prefix rest
        for prefix in "$base" "$base_real"; do
            rest="${target#"$prefix/ae-it-$owner/"}"
            [[ "$rest" != "$target" && -n "$rest" && "$rest" != */* && "$rest" != . && "$rest" != .. ]] || continue
            [[ -d "$prefix/ae-it-$owner" && ! -L "$prefix/ae-it-$owner" && ! -L "$target" ]] || return 1
            [[ -d "$target" || ! -e "$target" ]] || return 1
            owned="$prefix/ae-it-$owner"
            return 0
        done
        return 1
    }
    # Reap every root a DEAD owner registered in lane $1; fails if any is left.
    reap_registry() {
        local registry="$1/.ae-parity-fixtures" owner_dir owner recorded entry scratch failed=0
        [[ -e "$registry" ]] || return 0
        [[ -d "$registry" && ! -L "$registry" && -r "$registry" && -x "$registry" ]] || return 1
        for owner_dir in "$registry"/*; do
            [[ -e "$owner_dir" ]] || continue
            # `<pid>` or `<pid>.<identity>`, the writer's name for an owner.
            owner="${owner_dir##*/}"
            recorded=""
            [[ "$owner" != *.* ]] || { recorded="${owner#*.}"; owner="${owner%%.*}"; }
            if [[ ! -d "$owner_dir" || ! "$owner" =~ ^[0-9]+$ || ( -n "$recorded" && ! "$recorded" =~ ^[A-Za-z0-9]+$ ) ]] || ! owner_is_dead "$owner" "$recorded"; then
                failed=1
                continue
            fi
            for entry in "$owner_dir"/*; do
                if [[ ! -L "$entry" ]]; then
                    [[ ! -e "$entry" ]] || failed=1
                    continue
                fi
                scratch="$(readlink "$entry")" || { failed=1; continue; }
                if owned_root "$owner" "$scratch"; then
                    # A kill between a test's chmod and its restore leaves 0555
                    # dirs `rm -rf` cannot empty; repair owner rwx on dirs only,
                    # per-dir before descent (a batched chmod runs after the
                    # traversal it needed), still inside the proven root: plain
                    # `find` follows no link and `-type d` never matches one.
                    reap_sockets "$owned" && find "$owned" -type d -exec chmod u+rwx {} \; 2>/dev/null && rm -rf "$owned" 2>/dev/null || failed=1
                elif [[ -e "$scratch" ]]; then
                    reap_sockets "$scratch" || failed=1
                fi
            done
        done
        return "$failed"
    }
    # The lane sweep's ONE deletion rule: a transient writer race (a test's
    # orphaned child still writing) never fails the lane — the root is NAMED and
    # kept, and the next lane start retries. Only the registry reap's 30-try
    # bound may fail the lane: a root still there after 30 s is a leak of THIS
    # lane, not a race.
    keep_or_remove() {
        bound_run rm -rf "$1" 2>/dev/null || echo "note: kept $1: could not be deleted (a writer may still hold it); the next lane start retries" >&2
    }
    reap_stale_lanes() {
        local root stale name owner
        for root in "$base" "$legacy"; do
            for stale in "$root"/ae-rust-test.*; do
                [[ -d "$stale" && ! -L "$stale" ]] || continue
                name="${stale##*/ae-rust-test.}"
                owner="${name%%.*}"
                [[ "$owner" =~ ^[0-9]+$ ]] || continue
                owner_is_dead "$owner" || continue
                reap_registry "$stale" || continue
                reap_sockets "$stale" || continue
                keep_or_remove "$stale"
            done
            [[ "$legacy" != "$base" ]] || break
        done
    }
    # A dead test's root no lane registered: `ae-it-<pid>`, and the runner's own
    # `ae-it-shadow-<pid>` and `ae-it-tmux-<pid>`, each a plain directory (never a link) of this
    # uid, and only when older than this lane and its pid is gone by `owner_is_dead`
    # (no recorded identity: a pid that still answers keeps its root): a reused pid's fresh
    # root is never taken. A test killed between a chmod and its restore leaves directories at
    # 0555 that `rm -rf` cannot empty, and the old sweep named such a root again at every lane
    # start; so a root is repaired first as `reap_registry` does (`find -type d`, never a link).
    # THE SWEEP IS BOUNDED, never a failure of the lane: the `sweep_secs` budget starts on entry and
    # is checked before every entry, live roots and foreign names included; at most `sweep_scan`
    # entries are examined and `sweep_max` roots acted on per call. Every external step runs under
    # `bound_run`, whose timeout status survives a command substitution through `bound_note`; what a
    # call leaves stays for the next lane start, and a stop says so in one note. The timeout state is
    # the sweep's own: it is reset when the sweep ends, so the unbounded reaps after it never see it.
    # A root is judged and acted on one at a time, never from a list gathered earlier: `sweep_proof`
    # is taken again, with the inode it first named, after the socket scan and before each socket
    # effect, the repair and the removal, so a pid that came back or a path that was replaced while
    # the sweep worked is left alone. A probe that fails or times out proves nothing and stops the
    # sweep. Without a timeout tool (timeout, or gtimeout from brew coreutils) it cannot be bounded:
    # at the first dead root it says so and runs no external probe.
    sweep_max=64
    sweep_scan=512
    sweep_secs=20
    # sweep_proof DIR OWNER INODE FRESHNESS: DIR is still a plain directory of this uid with that
    # inode, its owner is gone, and (FRESHNESS = fresh) it is older than this lane. The freshness
    # is read once: the sweep's own work changes the directory afterwards.
    sweep_proof() {
        local dir="$1" owner="$2" ino="$3" check="$4" now fresh status=0
        [[ -d "$dir" && ! -L "$dir" && -O "$dir" ]] || return 1
        now="$(bound_run ls -di "$dir" 2>/dev/null)" || status=$?
        bound_note "$status"
        ((status == 0)) || return 1
        [[ "${now%% *}" == "$ino" ]] || return 1
        if [[ "$check" == fresh ]]; then
            fresh="$(bound_run find "$dir" -prune -newer "$test_tmux_tmp" 2>/dev/null)" || status=$?
            bound_note "$status"
            ((status == 0)) || return 1
            [[ -z "$fresh" ]] || return 1
        fi
        owner_is_dead "$owner"
    }
    # sweep_one DIR OWNER: returns 0 when the root was dealt with or left alone, 1 to stop the sweep.
    sweep_one() {
        local dir="$1" owner="$2" ino status=0
        ino="$(bound_run ls -di "$dir" 2>/dev/null)" || status=$?
        bound_note "$status"
        if ((bound_hit)); then return 1; fi
        ((status == 0)) || return 0
        ino="${ino%% *}"
        sweep_proof "$dir" "$owner" "$ino" fresh || { if ((bound_hit)); then return 1; fi; return 0; }
        if ! reap_sockets "$dir" sweep_proof "$dir" "$owner" "$ino" kept; then
            if ((bound_hit)); then return 1; fi
            return 0
        fi
        if ((bound_hit)); then return 1; fi
        sweep_proof "$dir" "$owner" "$ino" kept || { if ((bound_hit)); then return 1; fi; return 0; }
        bound_run find "$dir" -type d -exec chmod u+rwx {} \; >/dev/null 2>&1 || true
        if ((bound_hit)); then return 1; fi
        if sweep_proof "$dir" "$owner" "$ino" kept; then keep_or_remove "$dir"; fi
        if ((bound_hit)); then return 1; fi
        return 0
    }
    reap_dead_scratch() {
        local dir name owner timer scanned=0 acted=0 why=""
        timer="$(command -v timeout || command -v gtimeout || true)"
        sweep_deadline=$((SECONDS + sweep_secs))
        bound_tool="$timer"
        bound_hit=0
        for dir in "$base"/ae-it-*; do
            if ((scanned >= sweep_scan)); then why="$sweep_scan entries examined"; break; fi
            if ((SECONDS >= sweep_deadline)); then why="the ${sweep_secs} s budget"; break; fi
            scanned=$((scanned + 1))
            [[ -d "$dir" && ! -L "$dir" && -O "$dir" ]] || continue
            name="${dir##*/ae-it-}"
            case "$name" in
                shadow-*) owner="${name#shadow-}" ;;
                tmux-*) owner="${name#tmux-}" ;;
                *) owner="$name" ;;
            esac
            [[ "$owner" =~ ^[0-9]+$ ]] || continue
            owner_is_dead "$owner" || continue
            if [[ -z "$timer" ]]; then
                echo "note: stale scratch sweep skipped: no timeout tool (timeout, or gtimeout from brew coreutils) to bound it; the next lane start retries" >&2
                break
            fi
            if ((acted >= sweep_max)); then why="$sweep_max roots"; break; fi
            acted=$((acted + 1))
            sweep_one "$dir" "$owner" || { why="the ${sweep_secs} s budget or a step timeout"; break; }
        done
        # The timeout state belongs to the sweep: the unbounded callers after it never see it.
        bound_tool=""
        bound_hit=0
        [[ -z "$why" ]] || echo "note: stale scratch sweep stopped at $why; the next lane start continues" >&2
    }
    reap_stale_lanes
    test_tmux_tmp="$(mktemp -d "$base/ae-rust-test.$$.XXXXXX")"
    cleanup() {
        local status=$? tries=30 live=30 leaked=0
        bound_tool=""
        [[ -z "${watcher:-}" ]] || kill "$watcher" 2>/dev/null || true
        # Explicit socket, never a short -L: outside this directory that name is the live fleet server.
        # An installed-shape test server refuses kill-server: drop its guard first, then prove
        # the server stopped answering rather than trusting the kill's exit status.
        env -u TMUX -u TMUX_PANE tmux -S "$test_tmux_tmp/tmux-$(id -u)/ae" set-option -su command-alias >/dev/null 2>&1 || true
        env -u TMUX -u TMUX_PANE tmux -S "$test_tmux_tmp/tmux-$(id -u)/ae" kill-server >/dev/null 2>&1 || true
        until ! env -u TMUX -u TMUX_PANE tmux -S "$test_tmux_tmp/tmux-$(id -u)/ae" list-sessions >/dev/null 2>&1; do
            if ((--live == 0)); then leaked=1; break; fi
            sleep 0.1
        done
        reap_dead_scratch
        # A test's orphaned child (a real opencode) can write into its root for
        # seconds after the test; past the bound the lane fails, and the next
        # lane start retries.
        until reap_registry "$test_tmux_tmp"; do
            if ((--tries == 0)); then
                echo "error: kept $test_tmux_tmp: a registered scratch root or its tmux server outlived the lane" >&2
                exit $((status ? status : 1))
            fi
            sleep 1
        done
        if ((leaked)); then
            echo "error: kept $test_tmux_tmp: the lane's own tmux server outlived its cleanup" >&2
            exit $((status ? status : 1))
        fi
        keep_or_remove "$test_tmux_tmp"
    }
    trap cleanup EXIT
    # Cargo runs in a session of its own with no controlling terminal, so no
    # test can take the lane's terminal for a tmux pane it is in (#149). The
    # perl supervisor (REAP) stays in this shell's process group and is started
    # through `nest`, so a signal to any level reaches it and this shell waits
    # for it. It records the run in the main checkout's registry before cargo
    # starts, reaps every process of the run, and ends as cargo did (#340). The
    # registry and `detached` are the one shared text DETACHED.
    {{ NEST }}
    {{ DETACHED }}
    reap_dead_scratch
    mkdir "$test_tmux_tmp/tmp"
    export TMPDIR="$test_tmux_tmp/tmp"
    export TMUX_TMPDIR="$test_tmux_tmp"
    # In-process code reads the state root from the environment, so the lane gives
    # it an empty root of its own: no test reads the developer's real ~/.ae.
    mkdir "$test_tmux_tmp/ae-home"
    export AE_HOME="$test_tmux_tmp/ae-home"
    unset TMUX TMUX_PANE
    size_counts
    size_load "lane $lane"
    # A mutation run is memory-bounded in config (the nextest `mutants` profile:
    # two tests, a 60 s kill), and an env value beats a profile. So the lane may
    # TIGHTEN to one but never widen: anything but a plain 1 or 2 becomes 2, said
    # on stderr, because refusing would make an inherited wide count stop an
    # unattended run.
    if [[ "$lane" == mutants && ! "$NEXTEST_TEST_THREADS" =~ ^[12]$ ]]; then
        threads_from="$threads_from, was $NEXTEST_TEST_THREADS, capped"
        export NEXTEST_TEST_THREADS=2
    fi
    size_say "lane $lane"
    # An owned sentry proves every in-process fleet read stays on this run's
    # server. It is deliberately visible: Name(ae) is a real entitlement, and
    # tests asserting whole-fleet cardinality account for this one row.
    tmux -f /dev/null -L ae new-session -d -s foreign-review-sentry -e AE_SESSION=foreign-review-sentry
    # The mutation lane's memory ceiling. A mutant that turns a loop bound into
    # an unbounded allocation drives one test to ~2 GB/s, which no timeout
    # outruns and no rlimit stops on macOS, so this polls the process table
    # twice a second and SIGKILLs a TEST BINARY (first argv word under
    # target/*/deps/) that is a descendant of this lane's cargo-mutants and
    # holds more than AE_MUTANTS_RSS_MAX_MB (default 2048; an unusable value
    # is ignored). Never cargo, rustc, nextest, a linker or another lane's
    # tree. nextest then reports the test failed, so cargo-mutants will call the
    # mutant CAUGHT: every kill is therefore also appended to the receipt
    # mutants.out/ae-rss-kills.log (utc pid rss_mb binary args), which is what a
    # freeze reads to reclassify it as memory growth. `args`, not `comm`: macOS
    # prints a full path there and Linux a 15-character basename. No pid is
    # remembered past the next cycle (a recycled pid must stay bounded; a dying
    # process that still shows its rss is not logged twice), and the process is
    # re-read by pid just before the signal: a different `args` skips it. The
    # residual is a pid recycled into an identical command line inside the
    # milliseconds between that read and the kill.
    rss_watch() {
        local ceiling="${AE_MUTANTS_RSS_MAX_MB:-}" pid rss args line killed=" " now
        [[ "$ceiling" =~ ^[1-9][0-9]{0,5}$ ]] || ceiling=2048
        while sleep 0.5; do
            now=" "
            while read -r pid rss args; do
                [[ "$killed" != *" $pid "* ]] || continue
                [[ "$(ps -wwo args= -p "$pid" 2>/dev/null)" == "$args" ]] || continue
                kill -KILL "$pid" 2>/dev/null || continue
                now="$now$pid "
                line="$(date -u +%Y-%m-%dT%H:%M:%SZ) $pid $((rss / 1024)) $args"
                mkdir -p mutants.out
                echo "$line" >>mutants.out/ae-rss-kills.log
                echo "lane mutants: killed over the ${ceiling} MB ceiling: $line" >&2
            done < <(ps -axwwo pid=,ppid=,rss=,args= 2>/dev/null | awk -v root="$$" -v cap="$((ceiling * 1024))" '
                { p = $1; par[p] = $2; rss[p] = $3; exe[p] = $4; a = $0; sub(/^ *[0-9]+ +[0-9]+ +[0-9]+ +/, "", a); args[p] = a }
                function owned(p,   n, cm) {
                    for (n = 0; n < 64 && p + 0 > 1; n++) {
                        if (exe[p] ~ /(^|\/)cargo-mutants$/) cm = 1
                        if (p == root) return cm
                        p = par[p]
                    }
                    return 0
                }
                END { for (p in exe) if (exe[p] ~ /\/target\/.*\/deps\/[^\/]+$/ && rss[p] + 0 > cap && owned(p)) print p, rss[p], args[p] }')
            killed="$now"
        done
    }
    case "$lane" in
        test)
            # A filtered run is a receipt, not the gate: nextest only, no
            # doctests. The bare run keeps the same two commands; no "$@"
            # there (unbound under set -u on bash 3.2).
            if (( $# )); then
                detached cargo nextest run --locked --all-features "$@"
                echo "note: filtered run, skipping doctests" >&2
            else
                # The ledger reads the report THIS run writes: remove the last one first.
                junit="${CARGO_TARGET_DIR:-target}/nextest/default/junit.xml"
                rm -f "$junit" 2>/dev/null || { echo "note: gate ledger off: the old report $junit cannot be removed" >&2; junit=""; }
                detached nice -n 10 cargo nextest run --locked --all-features
                detached nice -n 10 cargo test --doc --locked --all-features -- --test-threads "$NEXTEST_TEST_THREADS"
                ledger_report "$junit" "$NEXTEST_TEST_THREADS" || echo "note: gate ledger skipped" >&2
            fi
            ;;
        cov) detached nice -n 10 cargo llvm-cov nextest --locked --all-features ;;
        mutants)
            if [[ "$test_tmux_tmp" == *"{"* || "$test_tmux_tmp" == *"}"* ]]; then
                echo "Error: mutants scratch path cannot contain braces: $test_tmux_tmp" >&2
                exit 1
            fi
            # Both artifact roots leave with the lane, including Cargo's target alias.
            export CARGO_TARGET_DIR="$test_tmux_tmp/target"
            export CARGO_BUILD_TARGET_DIR="$CARGO_TARGET_DIR"
            export CARGO_BUILD_BUILD_DIR="$CARGO_TARGET_DIR"
            rss_watch &
            watcher=$!
            status=0
            detached nice -n 10 cargo mutants --cargo-arg=--locked --jobs 1 "$@" || status=$?
            kill "$watcher" 2>/dev/null || true
            exit "$status"
            ;;
        stress)
            # ONE named filterset repeated count times (checked above, before any
            # state): nextest's own --stress-count, never a synthetic CPU burner.
            # Threads come from the sizing above (the least of the live budget, the
            # lane ceiling and an inherited export); nice 10; the time bound ends
            # the whole group (status 124) and the trap reaps the scratch.
            # nextest 0.9.143 exits 0 when only the LAST iteration passed, whatever
            # the earlier ones did (fixed upstream in 0.9.148, issue 327), so its
            # exit status alone is no verdict. The default profile's JUnit report
            # (.config/nextest.toml) holds one <testsuite name="<binary>@stress-N">
            # per binary and iteration; the lane deletes the old report first and
            # judges the fresh one after a zero exit: every iteration must be
            # there with tests in it, none may carry a failure, an error or a
            # retry that only passed, and a report that is missing, a symlink or
            # cut short fails the run (status 1). A failed iteration exits 100,
            # nextest's own failed-run status, and names the iterations as
            # nextest's log counts them (1-based). A non-zero nextest or timeout
            # status is the lane's status, never softened.
            stress_verdict() {
                local junit="$1" want="$2" said code=0
                if [[ ! -f "$junit" || -L "$junit" ]]; then
                    echo "rust-stress: FAILED: nextest exited 0 but left no JUnit report at $junit, so no iteration is proven to have passed" >&2
                    return 1
                fi
                said="$(awk -v want="$want" '
                    function join(list, sep, item) { return list == "" ? item : list sep item }
                    function attr(line, key,   v) {
                        if (!match(line, " " key "=\"[0-9]+\"")) return -1
                        v = substr(line, RSTART + length(key) + 3, RLENGTH - length(key) - 4)
                        return v + 0
                    }
                    /^[[:space:]]*<testsuite name="[^"]*@stress-[0-9]+" / {
                        match($0, /@stress-[0-9]+"/)
                        n = substr($0, RSTART + 8, RLENGTH - 9) + 0
                        t = attr($0, "tests"); e = attr($0, "errors"); f = attr($0, "failures")
                        if (t < 0 || e < 0 || f < 0) { garbled = 1; next }
                        cur = n; seen[n] = 1; tests[n] += t; bad[n] += e + f
                        next
                    }
                    /<flaky(Failure|Error)/ { if (cur != "") bad[cur]++ }
                    /<\/testsuites>/ { closed = 1 }
                    END {
                        if (garbled || !closed) { print "the JUnit report is cut short or unreadable"; exit 1 }
                        for (i = 0; i < want; i++) {
                            if (!seen[i] || tests[i] == 0) lost = join(lost, ", ", i + 1)
                            else if (bad[i] > 0) failed = join(failed, ", ", i + 1)
                        }
                        if (failed != "") say = "iteration(s) " failed " of " want " failed although nextest exited 0"
                        if (lost != "") say = join(say, "; ", "iteration(s) " lost " of " want " did not run")
                        if (say == "") { print want " of " want " iterations passed in the JUnit report"; exit 0 }
                        print say
                        exit (failed != "" ? 100 : 1)
                    }' "$junit")" || code=$?
                if ((code == 0)); then
                    echo "rust-stress: $said" >&2
                    return 0
                fi
                echo "rust-stress: FAILED: $said (JUnit report: $junit)" >&2
                return "$code"
            }
            junit="${CARGO_TARGET_DIR:-target}/nextest/default/junit.xml"
            rm -f "$junit" 2>/dev/null || true
            if [[ -e "$junit" || -L "$junit" ]]; then
                echo "rust-stress: refused: the old report $junit cannot be removed, and the verdict reads a fresh one" >&2
                exit 1
            fi
            status=0
            detached nice -n 10 "$stress_timeout" -k 5 "$stress_secs" \
                cargo nextest run --locked --all-features \
                --test-threads "$NEXTEST_TEST_THREADS" --stress-count "$stress_count" -E "$stress_filter" || status=$?
            if ((status != 0)); then
                note=""
                ((status != 124)) || note=" (the ${stress_secs} s bound ended the run before every iteration ran)"
                echo "rust-stress: FAILED: nextest exited with status $status$note" >&2
                exit "$status"
            fi
            stress_verdict "$junit" "$stress_count"
            ;;
        *) echo "Error: unknown isolated test lane '$lane'" >&2; exit 2 ;;
    esac

# Run the test suite: nextest + doctests
rust-test:
    exec just _tmux-isolated test

# ON-DEMAND live smoke of ae's adapter rows against the REAL harness binaries and
# logins (docs/development.md, "Live harness smoke"). It spends a few short model
# turns per tool, so it is NEVER a dependency of `test`, `pre`, `rust-check`,
# mutants or CI: both tests are `#[ignore]`d and need AE_HARNESS_SMOKE=1, which
# only this recipe sets. `just harness-smoke` drives AE_HARNESS_SMOKE_TOOLS;
# `just harness-smoke control` runs the hanging-shim control, which spends nothing.
harness-smoke mode="":
    #!/usr/bin/env bash
    set -euo pipefail
    case "${1:-}" in
        "") filter='test(=harness_smoke::live_harness_smoke)' ;;
        control) filter='test(=harness_smoke::live_harness_smoke_hang_control)' ;;
        *) echo "usage: just harness-smoke [control]" >&2; exit 2 ;;
    esac
    # The suite's runners register their scratch under TMUX_TMPDIR, so a run needs one. It is
    # this run's own; a run killed before its Drop leaves registered roots, and this trap kills
    # their private servers and removes them, then the lane.
    base="${AE_TEST_TMPDIR:-/tmp}"
    lane="$(mktemp -d "$base/ae-hs-lane.XXXXXX")"
    cleanup() {
        local status=$? entry root tries leaked=0
        for entry in "$lane"/.ae-parity-fixtures/*/*; do
            [[ -L "$entry" ]] || continue
            root="$(readlink "$entry")"
            case "$root" in
                "$base"/ae-it-*/hs.*)
                    env -u TMUX -u TMUX_PANE tmux -S "$root/sock" set-option -su command-alias >/dev/null 2>&1 || true
                    env -u TMUX -u TMUX_PANE tmux -S "$root/sock" kill-server >/dev/null 2>&1 || true
                    # The kill's exit status is not proof: the server must stop answering
                    # before its root is deleted; a survivor keeps the root and the lane.
                    tries=20
                    while env -u TMUX -u TMUX_PANE tmux -S "$root/sock" list-sessions >/dev/null 2>&1; do
                        ((--tries)) || break
                        sleep 0.1
                    done
                    if ((tries == 0)); then
                        echo "error: kept $root: its tmux server outlived the cleanup" >&2
                        leaked=1
                    else
                        rm -rf "$root"
                    fi
                    ;;
            esac
        done
        if ((leaked)); then
            echo "error: kept $lane: a registered scratch root's tmux server outlived the cleanup" >&2
            exit $((status ? status : 1))
        fi
        rm -rf "$lane"
    }
    trap cleanup EXIT
    export TMUX_TMPDIR="$lane"
    # Lane sizing, as `_tmux-isolated` does it: an explicit CARGO_BUILD_JOBS wins untouched,
    # otherwise jobs-budget (one positive integer of at most three digits, else ignored) says
    # how many cores are free now. One smoke test runs at a time, and cargo runs niced.
    if [[ -z "${CARGO_BUILD_JOBS:-}" ]] && command -v jobs-budget >/dev/null 2>&1; then
        budget="$(jobs-budget 2>/dev/null || true)"
        if [[ "$budget" =~ ^[1-9][0-9]{0,2}$ ]]; then
            export CARGO_BUILD_JOBS="$budget"
        fi
    fi
    export NEXTEST_TEST_THREADS=1
    AE_HARNESS_SMOKE=1 nice -n 10 cargo nextest run --locked --run-ignored only --no-capture -E "binary(it) & $filter"

# Coverage is a REPORT, not a gate. It becomes a gate the day a threshold is
# ratified, and not before.

# Coverage report (not a gate)
rust-cov:
    exec just _tmux-isolated cov

# The lane that asks whether the tests DISCRIMINATE, not just whether they pass.
# Agents write tests that pass; this is the check that costs them something.

# Mutation testing
rust-mutants *args:
    exec just _tmux-isolated mutants "$@"

# The BOUNDED repeat run for a flake hunt: nextest --stress-count over ONE named
# filterset (a module or a test, never the suite) through the isolated lane.
# Threads come from the lane's sizing (the least of jobs-budget, the lane ceiling
# and an inherited export), cargo runs at nice 10 under timeout -k 5 <secs>
# (status 124 when the bound ends it), it refuses to start above load1 14, and the
# lane reaps what it started. nextest 0.9.143 exits 0 when only the last iteration
# passed, so the lane also judges the run's JUnit report and fails (status 100)
# when any iteration failed, or (status 1) when one did not run or the report is
# missing (issue 327). Synthetic CPU burners are not committed
# (docs/development.md, Stress runs). Never reached by test, pre, rust-check,
# mutants or CI. Example: just rust-stress 'test(=menu::a_test)' 20 600

# Bounded flake hunt: repeat one named nextest filterset (load1 <= 14 to start)
rust-stress filter count="10" secs="900":
    just _tmux-isolated stress "$1" "$2" "$3"

# Fuzz the parsers of hostile persisted state on an exact nightly, without
# touching ae's product compiler. This lane REPORTS and is run by a human before
# such a parser cuts over; it never gates CI, which carries no nightly. Details,
# seed layout and the pending targets: fuzz/README.md.

# Refuse a fuzz run unless the pinned tools and a current fuzz lock are here.
_fuzz-preflight:
    #!/usr/bin/env bash
    set -euo pipefail
    want="{{ CARGO_FUZZ_VERSION }}"
    have="$(cargo fuzz --version 2>/dev/null | awk '{ print $2 }' || true)"
    if [ "$have" != "$want" ]; then
        echo "Error: the fuzz lane requires cargo-fuzz $want, found ${have:-none}" >&2
        echo "       cargo install --locked --version $want cargo-fuzz" >&2
        exit 1
    fi
    if ! rustup run {{ FUZZ_TOOLCHAIN }} rustc --version >/dev/null 2>&1; then
        echo "Error: the fuzz lane requires toolchain {{ FUZZ_TOOLCHAIN }}" >&2
        echo "       rustup toolchain install {{ FUZZ_TOOLCHAIN }} --component rust-src" >&2
        exit 1
    fi
    # cargo-fuzz passes -Zbuild-std by default, which compiles std from source.
    if ! rustup component list --toolchain {{ FUZZ_TOOLCHAIN }} | grep -q "^rust-src (installed)"; then
        echo "Error: {{ FUZZ_TOOLCHAIN }} has no rust-src, and cargo-fuzz builds std from source" >&2
        echo "       rustup component add rust-src --toolchain {{ FUZZ_TOOLCHAIN }}" >&2
        exit 1
    fi
    # cargo-fuzz 0.13.2 has no --locked pass-through, so the committed lock is
    # proven HERE instead, before anything builds.
    if ! cargo +{{ FUZZ_TOOLCHAIN }} metadata --locked --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null 2>&1; then
        echo "Error: fuzz/Cargo.lock is not current — a CalVer bump moves the ae version it records" >&2
        echo "       cargo +{{ FUZZ_TOOLCHAIN }} metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null" >&2
        echo "       git add fuzz/Cargo.lock" >&2
        exit 1
    fi
    # fuzz/ is not a workspace member. Check it with the PRODUCT rustfmt, as the
    # format gate does; neither check requires the fuzz nightly.
    if ! cargo fmt --manifest-path fuzz/Cargo.toml --check; then
        echo "Error: the fuzz sources are not rustfmt-clean" >&2
        echo "       cargo fmt --manifest-path fuzz/Cargo.toml" >&2
        exit 1
    fi

# Fuzz one parser (for example: just rust-fuzz target=config_parse secs=60)
rust-fuzz target secs="secs=60": _fuzz-preflight
    #!/usr/bin/env bash
    set -euo pipefail
    target="${1:-}"
    target="${target#target=}"
    secs="${2:-secs=60}"
    secs="${secs#secs=}"
    case " {{ FUZZ_TARGETS }} " in
        *" $target "*) ;;
        *) echo "Error: target must be one of: {{ FUZZ_TARGETS }}" >&2; exit 2 ;;
    esac
    # ANCHORED AND BOUNDED, because libFuzzer reads -max_total_time into an int
    # and treats zero as NO LIMIT. `00` is all-digits and is not the literal `0`,
    # so a leading-zero spelling used to pass a digits-only guard and disable the
    # bound this lane exists to impose; a value past the int range wrapped.
    # 999999s is ~11.5 days: far past any real run, far inside the range.
    if ! [[ $secs =~ ^[1-9][0-9]{0,5}$ ]]; then
        echo "Error: secs must be 1..999999 (a bounded run; 0, 00 and a leading + are not)" >&2
        exit 2
    fi
    seeds="$PWD/fuzz/seeds/$target"
    corpus="$PWD/fuzz/corpus/$target"
    if [ ! -d "$seeds" ]; then
        echo "Error: $target has no seeds directory: $seeds" >&2
        exit 2
    fi
    mkdir -p "$corpus"
    # TRACKED seeds, which is what the evidence line claims and the only part of
    # the input someone else can reproduce from the commit.
    count="$(git ls-files -- "fuzz/seeds/$target" | wc -l | tr -d '[:space:]')"
    commit="$(git rev-parse --short=8 HEAD 2>/dev/null || echo unknown)"
    if ! git diff --quiet HEAD 2>/dev/null; then
        commit="$commit-dirty"
    fi
    # The FIRST corpus path is the one libFuzzer WRITES to; the tracked seeds are
    # read alongside it. Both are absolute: the target runs from the fuzz dir.
    cargo +{{ FUZZ_TOOLCHAIN }} fuzz run "$target" "$corpus" "$seeds" -- -max_total_time="$secs"
    # The cutover evidence line. Paste it into the slice report it gates.
    echo "fuzz evidence: target=$target seeds=$count duration=${secs}s commit=$commit toolchain={{ FUZZ_TOOLCHAIN }} cargo-fuzz={{ CARGO_FUZZ_VERSION }} result=clean"

# Fuzz every hostile parser (for example: just rust-fuzz-all secs=60)
rust-fuzz-all secs="secs=60":
    #!/usr/bin/env bash
    set -euo pipefail
    secs="${1:-secs=60}"
    for target in {{ FUZZ_TARGETS }}; do
        just rust-fuzz "target=$target" "$secs"
    done

# The `--allow license-not-encountered` crutch is GONE as of the first real
# dependency (2026-08-29, P4.3 tracer A): the deny.toml allow-list is now
# minimal-to-encountered, so an unmatched entry is a genuine drift signal rather
# than the zero-dependency baseline noise it used to be. Do not re-add the flag;
# prune or extend the allow-list instead.

# Cargo runs `detached`, as every cargo step of the gate and the release does (#342, #345).
# Supply chain: advisories, licenses, bans, sources (policy in deny.toml)
rust-deny:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ NEST }}
    {{ DETACHED }}
    detached cargo deny --locked check

# Supply-chain PROVENANCE: has a human reviewed each dependency's code? cargo-vet
# arrived with the first runtime dependency (P4.3), orthogonal to cargo-deny (do
# NOT also add cargo-audit — that duplicates deny's RustSec lane). Posture is the
# honest first setup: four trusted registries (mozilla, google, zcash, isrg) are
# imported and PINNED in supply-chain/imports.lock, so `--locked` re-checks
# offline in CI. The current graph is grandfathered as `exemptions` in
# supply-chain/config.toml — the audit BACKLOG, not an audited set: ring, rustls
# and ureq are the priority to delta-audit (their exact versions are newer than
# the registries certify), and cargo-deny already gates them for advisories. A
# dependency or version that no import covers fails HERE until it is audited or
# exemption'd — that is the point.
rust-vet:
    cargo vet --locked

# Reclaim THIS checkout's stale ae build output and keep the compiled dependencies.
# Every CalVer bump and source edit leaves another version's units behind
# (docs/development.md, Build disk); `cargo clean -p ae` drops the ae package's
# artifacts of every version from the six trees below, and the next gate
# rebuilds them in a minute or two. `just rust-prune` only REPORTS;
# `just rust-prune apply=yes` cleans. It judges the one tree it runs in, so run
# it from a checkout's root, once per checkout.
#
# THE GUARDS ARE THE POINT, not the six cleans. It refuses (exit 1, one line
# naming the guard) when a redirect variable is set, when `cargo metadata`
# (asked from this tree, because cargo finds `.cargo/config.toml` from the
# working directory and its parents, not from `--manifest-path`) puts the target
# or build directory anywhere but `target/`, when `target/` or a tree below it is
# a link, when any process outside this run's own ancestry has an argv or a
# working directory under the checkout, and when what `cargo clean --dry-run
# --verbose` says for a tree is not provable: every non-empty line of its STDOUT
# must be an absolute path inside the tree and link-free, and its stderr must hold
# exactly one valid `Summary` line (a genuine zero-removal run has an empty stdout
# and `Summary 0 files`).
# No `rm`, no glob: cargo does the removal, and the two guard trees are named.
# `ps` is required; `lsof` is used when installed, and without it the run says so
# and judges by argv alone. A lane between phases, with no child alive, is
# invisible to both: do not run this during a gate. Do not pipe its output
# either: the pipe's other end runs in the checkout and is refused.
rust-prune apply="":
    #!/usr/bin/env bash
    set -euo pipefail
    case "${1:-}" in
        "") mode=report ;;
        apply=yes) mode=apply ;;
        *) echo "Error: rust-prune takes no argument (a report) or apply=yes" >&2; exit 2 ;;
    esac
    refuse() { echo "rust-prune: refused: $*" >&2; exit 1; }
    for var in CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR CARGO_BUILD_BUILD_DIR CARGO_BUILD_TARGET; do
        [ -z "${!var+x}" ] || refuse "$var is set; a redirected build directory is not one this recipe judges"
    done
    top="$(git rev-parse --show-toplevel 2>/dev/null)" || refuse "not inside a git checkout"
    root="$(cd -P "$top" && pwd -P)"
    [ "$(pwd -P)" = "$root" ] || refuse "run it from the checkout root, $root"
    target="$root/target"
    if [ ! -e "$target" ] && [ ! -L "$target" ]; then
        echo "rust-prune: no target/ in $root, nothing to do"
        exit 0
    fi
    # A real directory, not a link, whose physical path is exactly $1.
    plain_dir() { [ -d "$1" ] && [ ! -L "$1" ] && [ "$(cd -P "$1" 2>/dev/null && pwd -P)" = "$1" ]; }
    plain_dir "$target" || refuse "$target is not a plain directory"
    # Refuse when a process outside THIS run's own ancestry (the recipe shell, just,
    # the invoking shell) has an argv naming the checkout or its working directory
    # inside it. Anything else, same cwd or not, is somebody else's. Without `lsof`
    # the working-directory half cannot be asked: that is said once per run.
    lsof_noted=""
    in_use() {
        local table pid ppid cmd n=0 i cur=$$ hops=0 up="" ancestry=" " out line path
        local -a pids ppids cmds
        table="$(ps -A -ww -o pid=,ppid=,command= 2>/dev/null)" || refuse "ps failed, so a running build cannot be ruled out"
        while read -r pid ppid cmd; do
            case "$pid" in '' | *[!0-9]*) continue ;; esac
            pids[n]="$pid"
            ppids[n]="$ppid"
            cmds[n]="$cmd"
            n=$((n + 1))
        done <<<"$table"
        while [ "$cur" -gt 1 ] && [ "$hops" -lt 64 ]; do
            ancestry="$ancestry$cur "
            up=""
            for ((i = 0; i < n; i++)); do
                if [ "${pids[i]}" = "$cur" ]; then
                    up="${ppids[i]}"
                    break
                fi
            done
            if [ -z "$up" ]; then
                [ "$hops" -gt 0 ] || refuse "this run is not in the process table, so its own ancestry cannot be proven"
                break
            fi
            cur="$up"
            hops=$((hops + 1))
        done
        for ((i = 0; i < n; i++)); do
            case "$ancestry" in *" ${pids[i]} "*) continue ;; esac
            case "${cmds[i]}" in *"$root/"*) refuse "pid ${pids[i]} is running under $root: ${cmds[i]}" ;; esac
        done
        if command -v lsof >/dev/null 2>&1; then
            # From `/`, because lsof lists itself and inherits this directory.
            out="$(cd / && lsof -d cwd -Fpn 2>/dev/null)" || refuse "lsof failed, so a working directory under $root cannot be ruled out"
            pid=""
            while IFS= read -r line; do
                case "$line" in
                    p*) pid="${line#p}" ;;
                    n*)
                        path="${line#n}"
                        case "$path/" in
                            "$root/"*) case "$ancestry" in *" $pid "*) ;; *) refuse "pid $pid has its working directory under $root" ;; esac ;;
                        esac
                        ;;
                esac
            done <<<"$out"
        elif [ -z "$lsof_noted" ]; then
            lsof_noted=yes
            echo "rust-prune: note: lsof is not installed, so only a process whose argv names $root is seen; one that merely has its working directory there is not" >&2
        fi
    }
    in_use
    meta="$(cargo metadata --no-deps --locked --format-version 1 --manifest-path "$root/Cargo.toml" 2>/dev/null)" || refuse "cargo metadata failed, so the build directories are unknown"
    for key in target_directory build_directory; do
        dir="$(printf '%s\n' "$meta" | sed -n 's/.*"'"$key"'":"\([^"]*\)".*/\1/p' | head -1)"
        [ -n "$dir" ] || refuse "cargo metadata names no $key"
        case "$dir" in *\\*) refuse "cargo's $key is not a plain path: $dir" ;; esac
        [ "$(cd -P "$dir" 2>/dev/null && pwd -P)" = "$target" ] || refuse "cargo's $key is $dir, not $target: a redirect in a cargo config"
    done
    # The six cleans, fixed and named; $1 is a callback taking label, tree (skipped
    # when absent), root (everything cargo may remove for it lies inside; a --target
    # clean also lists the host units under target/<profile>) and cargo flags.
    each() {
        "$1" debug "$target/debug" "$target" --profile dev
        "$1" release "$target/release" "$target" --profile release
        "$1" native-release "$target/{{ RUST_NATIVE_TARGET }}" "$target" --profile release --target {{ RUST_NATIVE_TARGET }}
        "$1" musl-release "$target/x86_64-unknown-linux-musl" "$target" --profile release --target x86_64-unknown-linux-musl
        "$1" force-warn-guard "$target/force-warn-guard" "$target/force-warn-guard" --profile dev --target-dir "$target/force-warn-guard"
        "$1" world-read-guard "$target/world-read-guard" "$target/world-read-guard" --profile dev --target-dir "$target/world-read-guard"
    }
    # One dry run with its two streams kept apart: cargo's STDOUT (the paths it would
    # remove) comes first, then a `rust-prune-end <exit status>` line, then its
    # stderr (diagnostics and the Summary). The reader splits at the LAST end line,
    # so nothing on either stream can pose as the other.
    nl=$'\n'
    dry_run() {
        local err rc=0
        {
            err="$(cargo clean --locked --manifest-path "$root/Cargo.toml" -p ae "$@" --dry-run --verbose 2>&1 1>&3)" || rc=$?
            printf 'rust-prune-end %s\n%s\n' "$rc" "$err"
        } 3>&1
    }
    # Dry-run one clean and prove what cargo says. Every non-empty stdout line is an
    # absolute path inside its root with no link on the way (dot segments refused); an
    # unreadable line is a refusal, never skipped. Stderr is diagnostics only, and
    # must hold exactly one valid Summary (`Summary <n> file[s][, <size> total]`) that
    # does not claim files while listing none; a zero-removal run has an empty stdout
    # and `Summary 0 files`.
    # DFS output repeats parents, so the last proven directory spares re-walking them.
    prove() {
        local label="$1" tree="$2" top="$3" raw paths diag rc line dir ok="" d sum="" n="" rest word summaries=0 npaths=0 valid=""
        shift 3
        { [ -e "$tree" ] || [ -L "$tree" ]; } || return 0
        plain_dir "$tree" || refuse "$tree is not a plain directory"
        raw="$(dry_run "$@")"
        raw="$nl$raw"
        case "$raw" in *"${nl}rust-prune-end "*) ;; *) refuse "cargo's dry run for $label left no end marker" ;; esac
        paths=${raw%"${nl}rust-prune-end "*}
        diag=${raw#"$paths${nl}rust-prune-end "}
        rc=${diag%%"$nl"*}
        diag=${diag#"$rc$nl"}
        [ "$rc" = 0 ] || { printf '%s\n' "$diag" >&2; refuse "cargo clean --dry-run failed for $label"; }
        while IFS= read -r line; do
            [ -n "$line" ] || continue
            npaths=$((npaths + 1))
            case "$line" in /*) ;; *) refuse "cargo's dry run for $label listed a line that is not an absolute path: $line" ;; esac
            case "$line" in "$top"/*) ;; *) refuse "cargo would remove $line, outside $top" ;; esac
            case "/$line/" in */../* | */./*) refuse "cargo would remove a path with a dot segment: $line" ;; esac
            [ ! -L "$line" ] || refuse "cargo would remove through a link: $line"
            dir="${line%/*}"
            case "$ok/" in "$dir/"*) continue ;; esac
            d="$dir"
            while [ "${#d}" -gt "${#top}" ]; do
                [ ! -L "$d" ] || refuse "cargo would remove through a link: $d"
                d="${d%/*}"
            done
            ok="$dir"
        done <<<"$paths"
        while IFS= read -r line; do
            word=${line#"${line%%[! ]*}"}
            case "$word" in
                "Summary " | "Summary") summaries=$((summaries + 1)); sum="" ;;
                "Summary "*) summaries=$((summaries + 1)); sum=${word#Summary } ;;
            esac
        done <<<"$diag"
        if [ "$summaries" = 1 ]; then
            n=${sum%% *}
            rest=${sum#"$n"}
            case "$n" in
                '' | *[!0-9]*) ;;
                *) case "$rest" in " file" | " files" | " file, "*" total" | " files, "*" total") valid=yes ;; esac ;;
            esac
        fi
        [ -n "$valid" ] || refuse "cargo's dry run for $label ended without a valid Summary line (found $summaries)"
        [ "$n" = 0 ] || [ "$npaths" -gt 0 ] || refuse "cargo's dry run for $label reports $n files but listed no path on stdout"
        if [ "$mode" = report ]; then
            echo "would clean $label: $sum"
        fi
    }
    clean_one() {
        local label="$1" tree="$2" out sum
        shift 3
        { [ -e "$tree" ] || [ -L "$tree" ]; } || return 0
        out="$(cargo clean --locked --manifest-path "$root/Cargo.toml" -p ae "$@" 2>&1)" || { printf '%s\n' "$out" >&2; refuse "cargo clean failed for $label"; }
        sum="$(printf '%s\n' "$out" | sed -n 's/^ *Removed //p' | head -1)"
        echo "cleaned $label: ${sum:-done}"
    }
    each prove
    if [ "$mode" = report ]; then
        echo "rust-prune: report only, nothing removed; just rust-prune apply=yes cleans"
        exit 0
    fi
    in_use
    each clean_one
    echo "rust-prune: done; the next gate rebuilds the ae units"

# NATIVE is a real, runnable binary and is run here to prove it.
#
# THE MUSL COMPILE-SMOKE WAS REMOVED at the first dependency (2026-08-29, P4.3
# tracer A). It used to be a free `cargo check --target {{ RUST_CROSS_TARGET }}`
# that never linked. `ring` ended that: its build script compiles C for the
# target, so a musl `cargo check` needs a musl C toolchain. This recipe stays
# NATIVE-ONLY regardless: a bare clone must build with rustup and just and
# nothing else, and requiring a cross toolchain of every clone to run a release
# smoke is the wrong trade.
#
# Where the musl target IS linked is `just bundles`, which needs the prebuilt
# Homebrew toolchain ({{ RUST_MUSL_CC }}) and says so when it is missing. Where
# it is RUN stays the Linux CI leg (.github/workflows/rust.yml): a macOS kernel
# cannot exec an ELF binary, so "it links and carries no PT_INTERP" is the whole
# of what a laptop can prove, and the run plus the musl DNS/NSS proof remain
# CI's. {{ RUST_CROSS_TARGET }} is also deny.toml's second graph.

# Release build: native binary (the musl half is `just bundles`; its RUN proof is Linux CI)
rust-build-release:
    cargo build --release --locked
    @echo "==> native binary: target/release/ae"
    @./target/release/ae --version

# ── Linux container gate (local; CI stays off) ───────────────────────

# THE GATE ON REAL LINUX, on this machine, NATIVE arm64. `just rust-linux` (Ubuntu
# 24.04) and `just rust-linux-26` (Ubuntu 26.04, with or without a stalling
# session bus, below) run the whole gate (`just rust-setup`, then `just test`) inside a local
# Linux container. Both are ON DEMAND: `just test`, `just pre` and `just release`
# never reach them. An amd64 gate was tried and DROPPED (under QEMU user emulation
# rust-setup cannot build cargo-nextest — aws-lc-sys's cc segfaults — and six
# more tools would compile at ~6x); the smoke below still proves the shipped
# amd64 bundle under QEMU. The checkout is bound READ-ONLY, so a run cannot
# touch the live tree, and CARGO_HOME, RUSTUP_HOME and CARGO_TARGET_DIR
# live in named volumes (`ae-linux-arm64-*`, `ae-linux26-arm64-*` for the 26.04
# lane, which never shares a cache with the 24.04 one), so the host target/ is never
# written and a warm run skips the toolchain and the seven tool builds. The
# target volume is shared by EVERY checkout, so the container rebuilds the
# `ae` package from the mounted tree before the gate (#190) while the
# dependency artifacts stay warm. The
# container runs NON-ROOT, as a user whose uid IS the invoking user's: root
# ignores mode bits, so ae's 0555/0444 and permission-refusal tests would lie.
# Every container, volume and image this recipe makes carries the
# `ae.lane=linux` label, and the volumes are caches: nothing here removes one
# (remove by name, e.g. `docker volume rm ae-linux-arm64-cargo
# ae-linux-arm64-rustup ae-linux-arm64-target`, for a cold run). The recipes
# themselves take no lock; on a shared machine the batch rule serialises test
# lanes outside them.
#
# The host records HEAD, the tree id and the count of dirty paths, and the
# container compares all three on the mounted checkout before anything runs
# (exit 3 on a mismatch): a lane run is evidence for the tree it names. A dirty
# tree is identified by HEAD and that count only, never by its content, so exact
# evidence needs a clean checkout.
#
# `just rust-linux-26` takes a `bus`: `stall` (the default) adds a STALLING session
# bus; `off` is the validated-host arm, the 26.04 image with the container's own
# session. On `stall` a private dbus-daemon is the session bus and the activation of
# org.freedesktop.systemd1 never completes, so tmux 3.6 waits out its
# transient-scope timeout at every pane child spawn or respawn, as it does on a
# host whose user manager answers slowly. The stall arm PROVES the stall before
# it builds anything: a method call to that name must end in a reply timeout
# (NoReply) after at least two seconds, else the run stops with exit 4. The stall
# arm is the default since it went green (#267).
# These are arm64 lanes. The x86_64 proof of Ubuntu 26.04 is a human run on an
# x86_64 host; nothing here claims it.
# The gate on Ubuntu 24.04, native arm64, in a local container.
rust-linux: (_linux-gate "linux" LINUX_IMAGE "off")

# The gate on Ubuntu 26.04, native arm64: `just rust-linux-26` is the stalling-bus arm (#267), `just rust-linux-26 off` the validated-host arm.
rust-linux-26 bus="stall": (_linux-gate "linux26" LINUX_IMAGE_26 bus)

# The one body behind both lanes: `flavor` names the image and the cache
# volumes, `image` is the pinned base, `bus` is off or stall.
_linux-gate flavor image bus:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ bus }}" in off|stall) ;; *) echo "Error: bus must be off or stall, not {{ bus }}" >&2; exit 2 ;; esac
    # The gate is a NATIVE arm64 container. On a non-arm64 daemon the fetched
    # aarch64 binaries would die in a cryptic exec format error, so the daemon
    # is probed first and anything else refuses with the reason.
    daemon_arch="$(docker run --rm --label ae.lane=linux {{ image }} uname -m)"
    if [ "$daemon_arch" != aarch64 ]; then
        echo "Error: the {{ flavor }} lane runs a native arm64 container; this Docker daemon is $daemon_arch; on a Linux host, just test is already the Linux gate" >&2
        exit 1
    fi
    sums() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
    uid="$(id -u)"
    lane="$(mktemp -d "${TMPDIR:-/tmp}/ae-linux-lane.XXXXXX")"
    # Cleanup makes the lane writable before removing it (C83): an archive
    # member extracted 0555 refuses its own unlink.
    trap 'status=$?; chmod -R u+w "$lane" 2>/dev/null || true; rm -rf "$lane"; exit "$status"' EXIT
    # just and rustup-init are fetched on the HOST and digest-checked here, then
    # bind-mounted read-only: the container needs no curl and fetches nothing
    # beyond what cargo and rustup themselves fetch.
    curl -fsSL --retry 3 -o "$lane/just.tar.gz" \
        "https://github.com/casey/just/releases/download/{{ JUST_VERSION }}/just-{{ JUST_VERSION }}-aarch64-unknown-linux-musl.tar.gz"
    printf '%s  %s\n' "{{ LINUX_JUST_SHA_ARM64 }}" "$lane/just.tar.gz" | sums -c - >/dev/null
    tar -xzf "$lane/just.tar.gz" -C "$lane" just
    curl -fsSL --retry 3 -o "$lane/rustup-init" \
        "https://static.rust-lang.org/rustup/archive/{{ RUSTUP_VERSION }}/aarch64-unknown-linux-gnu/rustup-init"
    printf '%s  %s\n' "{{ LINUX_RUSTUP_SHA_ARM64 }}" "$lane/rustup-init" | sums -c - >/dev/null
    chmod 0755 "$lane/rustup-init"
    # The volumes ARE the warm-run cache; creation and the ownership one-shot are
    # idempotent, and the chown runs only on a volume that is not already the
    # invoking uid, so a warm run never walks its tree again.
    for v in cargo rustup target; do
        docker volume create --label ae.lane=linux "ae-{{ flavor }}-arm64-$v" >/dev/null
        docker run --rm --label ae.lane=linux -v "ae-{{ flavor }}-arm64-$v:/own" {{ image }} \
            sh -c "[ \"\$(stat -c %u /own)\" = $uid ] || chown -R $uid:$uid /own"
    done
    # The image builds from an EMPTY context — the Dockerfile arrives on stdin —
    # so the checkout (whose target/ alone is gigabytes) is never sent to the daemon.
    # The in-container uid comes from the HOST: without the build arg the recipe
    # would rebuild as the default 1000, which ubuntu 24.04 already ships as the
    # `ubuntu` user — the run then cannot write its own volumes (rc=1 in seconds).
    docker build --build-arg UID="$(id -u)" -t ae-{{ flavor }}-arm64 - <<'DOCKERFILE'
    FROM {{ image }}
    LABEL ae.lane=linux
    RUN apt-get update \
            && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
                build-essential tmux git perl procps ca-certificates locales dbus \
            && rm -rf /var/lib/apt/lists/*
    # en_US.UTF-8 serves the collation arm (the non-C locale test is
    # INCONCLUSIVE without it). The run sets no LANG, so every run proves the
    # POSIX locale: ae and the fixtures start their tmux clients `-u` (#187).
    RUN locale-gen en_US.UTF-8
    # The in-container uid IS the invoking user's, so git accepts the bind-mounted
    # checkout as its own and no safe.directory override is needed. ubuntu 24.04
    # already ships a uid-1000 user, so a taken uid falls through to that one and
    # USER stays numeric; the name never matters (HOME is set on docker run).
    ARG UID=1000
    RUN useradd --create-home --uid "$UID" --user-group ae || true
    USER "$UID"
    DOCKERFILE
    repo="$(pwd -P)"
    # A worktree's .git is a FILE naming an absolute gitdir in the main checkout,
    # so tests that read history need that directory mounted at its own path too.
    gitdir="$(git rev-parse --git-common-dir)"
    case "$gitdir" in /*) ;; *) gitdir="$repo/$gitdir" ;; esac
    mounts=(-v "$repo:$repo:ro")
    [ "$gitdir" = "$repo/.git" ] || mounts+=(-v "$gitdir:$gitdir:ro")
    # Inside the container <repo>/target IS the arm64 target volume: the clippy
    # force-warn probes write under it (the checkout is :ro, and a read-only
    # bind cannot grow a mountpoint, so the directory is created on the host
    # first — gitignored). The host's own target/ is shadowed and never written;
    # a fresh worktree without one gets an empty directory to mount over.
    mkdir -p "$repo/target"
    mounts+=(-v "ae-{{ flavor }}-arm64-target:$repo/target")
    # The tree the container is about to test, as the host sees it. The container
    # compares these three before it runs anything.
    head="$(git rev-parse HEAD)"
    tree="$(git rev-parse "HEAD^{tree}")"
    dirty="$(git status --porcelain | wc -l | tr -d '[:space:]')"
    echo "==> ae-{{ flavor }} gate: started $(date -u +%Y-%m-%dT%H:%M:%SZ) head=$head tree=$tree dirty=$dirty bus={{ bus }}"
    start=$SECONDS
    rc=0
    # --init: without it the bash below is PID 1 — the kernel drops SIGTERM to
    # a PID 1 with no handler, so docker stop waits 10 s and then SIGKILLs, and
    # orphaned test processes (tmux servers, ae daemons) reparent to a bash that
    # may not reap them, so a zombie still answers ps and kill -0 to a liveness
    # proof. (Measured: a Ctrl-C on the docker CLI ends this run in NEITHER
    # configuration; a run is stopped with docker stop.)
    docker run --rm --init --label ae.lane=linux --cpus 4 --memory 8g \
        -v "$lane:/stage:ro" \
        "${mounts[@]}" \
        -v "ae-{{ flavor }}-arm64-cargo:/home/ae/.cargo" \
        -v "ae-{{ flavor }}-arm64-rustup:/home/ae/.rustup" \
        -e HOME=/home/ae \
        -e CARGO_HOME=/home/ae/.cargo -e RUSTUP_HOME=/home/ae/.rustup \
        -e CARGO_BUILD_JOBS=4 \
        -e NEXTEST_TEST_THREADS=4 \
        -e NEXTEST_PROFILE=linux \
        -e AE_LANE_HEAD="$head" -e AE_LANE_TREE="$tree" -e AE_LANE_DIRTY="$dirty" \
        -e AE_LANE_BUS={{ bus }} \
        -w "$repo" \
        ae-{{ flavor }}-arm64 \
        bash -c 'set -euo pipefail
            export PATH="/stage:/home/ae/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
            # Source identity first: the checkout mounted here must be the tree
            # the host recorded. git gets a config of its own that trusts this
            # directory (a bind mount can show a foreign owner); it is scoped to
            # these commands, so the suite keeps the environment it had.
            gitcfg="$(mktemp)"
            printf "[safe]\n\tdirectory = %s\n" "$PWD" > "$gitcfg"
            got_head="$(GIT_CONFIG_GLOBAL="$gitcfg" git rev-parse HEAD)"
            got_tree="$(GIT_CONFIG_GLOBAL="$gitcfg" git rev-parse "HEAD^{tree}")"
            got_dirty="$(GIT_CONFIG_GLOBAL="$gitcfg" git status --porcelain | wc -l | tr -d "[:space:]")"
            echo "==> mounted source: head=$got_head tree=$got_tree dirty=$got_dirty"
            [ "$got_head" = "$AE_LANE_HEAD" ] && [ "$got_tree" = "$AE_LANE_TREE" ] && [ "$got_dirty" = "$AE_LANE_DIRTY" ] || exit 3
            # The stalling bus, set up and proven BEFORE the first build step. The
            # service file names the systemd1 activation that never completes; the
            # daemon is the only bus this run sees; the proof is a method call to
            # that very name, which must end in a reply timeout after at least two
            # seconds. Anything else (an immediate unknown-service error, a refused
            # connection, a slow failure of another kind) is not the stall: exit 4.
            if [ "$AE_LANE_BUS" = stall ]; then
                svc="$(mktemp -d)"
                mkdir -p "$svc/dbus-1/services"
                printf "[D-BUS Service]\nName=org.freedesktop.systemd1\nExec=/bin/sleep 30\n" > "$svc/dbus-1/services/org.freedesktop.systemd1.service"
                XDG_DATA_HOME="$svc" dbus-daemon --session --fork --address=unix:path=/tmp/ae-lane-bus
                export DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/ae-lane-bus
                SECONDS=0
                err="$(dbus-send --session --print-reply --dest=org.freedesktop.systemd1 --reply-timeout=2500 /org/freedesktop/systemd1 org.freedesktop.DBus.Peer.Ping 2>&1 || true)"
                echo "==> stalling bus proof after ${SECONDS}s: $err"
                case "$err" in *DBus.Error.NoReply*) ;; *) exit 4 ;; esac
                if [ "$SECONDS" -lt 2 ]; then exit 4; fi
            fi
            # Idempotent bootstrap: rustup proxies land in the cargo volume and no
            # default toolchain is set — rust-toolchain.toml, read by rust-setup,
            # decides what is installed. A warm run re-probes and installs nothing.
            /stage/rustup-init -y --no-modify-path --profile minimal --default-toolchain none
            just rust-setup
            # The target volume is shared by every checkout, but the cargo
            # fingerprint does not see the checkout path (#190): without this
            # the gate could test the wrong tree build. Deps stay warm.
            cargo clean --locked -p ae
            just test' || rc=$?
    echo "==> ae-{{ flavor }} gate: rc=$rc wall=$((SECONDS - start))s"
    exit "$rc"

# THE x86_64 MUSL RELEASE BUNDLE EXECUTES ON REAL LINUX. `just bundles` can only
# byte-search a foreign-architecture core ("version proven by byte search, not
# by running it"); this smoke runs the bundle's ae-core in an amd64 container and
# compares `--version` with this tree's own version, derived at run time — never
# a pinned release. Emulation is the Docker VM's ambient QEMU binfmt; when it is
# missing the run FAILS LOUDLY below instead of passing silently. The bundle
# comes from ./dist and is verified against the SHA256SUMS `just bundles` wrote.
rust-linux-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(just version)"
    tarball="dist/ae-$version-linux-x86_64-musl.tar.gz"
    [ -f "$tarball" ] || { echo "Error: $tarball is missing — run: just bundles" >&2; exit 1; }
    sums() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
    line="$(grep -F " ae-$version-linux-x86_64-musl.tar.gz" dist/SHA256SUMS)" || {
        echo "Error: dist/SHA256SUMS has no entry for $tarball — rebuild: just bundles" >&2
        exit 1
    }
    lane="$(mktemp -d "${TMPDIR:-/tmp}/ae-linux-smoke.XXXXXX")"
    trap 'status=$?; chmod -R u+w "$lane" 2>/dev/null || true; rm -rf "$lane"; exit "$status"' EXIT
    printf '%s\n' "$line" | ( cd dist && sums -c - ) >/dev/null
    tar -xzf "$tarball" -C "$lane"
    if ! out="$(docker run --rm --label ae.lane=linux --platform linux/amd64 \
            -v "$lane:/bundle:ro" {{ LINUX_IMAGE }} \
            "/bundle/ae-$version-linux-x86_64-musl/ae-core" --version 2>&1)"; then
        echo "Error: the amd64 container could not run the musl core — amd64 emulation (QEMU binfmt) is missing in this Docker VM" >&2
        printf '%s\n' "$out" >&2
        exit 1
    fi
    got="${out%%$'\n'*}"
    [ "$got" = "ae $version" ] || { echo "Error: --version printed '$got', want 'ae $version'" >&2; exit 1; }
    echo "==> amd64 smoke ok: $got"

# bacon is deliberately NOT installed by rust-setup: a personal dev loop is not
# part of the bootstrap contract.

# Optional watch loop (requires bacon, installed separately)
rust-watch:
    bacon clippy-all
