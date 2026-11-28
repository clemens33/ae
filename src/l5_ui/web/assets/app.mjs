// ae web: the orchestrator chat in a browser tab. Every server string is
// rendered as a Preact text child, never as markup. Network calls go through
// the one wrapper below, to this origin's /api/ routes.
import { h, render } from "./preact.mjs";
import { useState, useEffect, useLayoutEffect, useRef, useCallback } from "./hooks.mjs";
import htm from "./htm.mjs";

const html = htm.bind(h);
const KEY = "ae-web-session";
const VISIBLE_MS = 2000;
const HIDDEN_MS = 30000;
const NO_SESSION = "open the newest link from ae web";
// A queued or busy send is asked again this often, at most this many times,
// before the page stops asking; the draft id stays, so a later send asks once
// more and never carries twice.
const SEND_AGAIN_MS = 1000;
const SEND_TRIES = 120;
const UNREACHABLE = "ae web is not reachable";
// Why nothing can be sent: the composer's words, also on every control that
// would send without a route.
const NO_ROUTE = "No proven orchestrator to send to";
// The one thing the browser keeps beyond the session: the slugs of the pinned topics, nothing else.
const PINS_KEY = "ae-web-pins";
const PINS_MAX = 32;
// A turn that first shows up this much older than the newest turn already seen is
// an old one come back, not a new one.
const LATE_MICROS = 120 * 1000 * 1000;
// The most missing turns the unread book remembers: one full chat window.
const MISSING_MAX = 500;

// The stored pins: only slugs, each once, at most PINS_MAX. Any other value is no
// pin. Null says the store cannot be read at all.
function readPins() {
  let text;
  try {
    text = localStorage.getItem(PINS_KEY);
  } catch (_) {
    return null;
  }
  let list = [];
  try {
    list = JSON.parse(text || "[]");
  } catch (_) {
    // A value that is no JSON holds no pin.
  }
  const pins = [];
  for (const slug of Array.isArray(list) ? list : []) {
    if (pins.length < PINS_MAX && typeof slug === "string" && slugOk(slug, 48) && !pins.includes(slug)) {
      pins.push(slug);
    }
  }
  return pins;
}

// The pinned topics, and a press that pins or unpins one. Every tab of this address
// shares the store, so a press reads it again before it writes, and a change made
// in another tab is read back at once. Once the store refuses a write, the pins
// live in this page only until it is reloaded: the page neither writes nor reads
// the store again, so its older list never replaces another tab's.
function usePins() {
  const [pins, setPins] = useState(() => readPins() || []);
  const refused = useRef(false);
  useEffect(() => {
    const changed = (event) => {
      if (!refused.current && (event.key === PINS_KEY || event.key === null)) {
        const read = readPins();
        if (read) {
          setPins(read);
        }
      }
    };
    window.addEventListener("storage", changed);
    return () => window.removeEventListener("storage", changed);
  }, []);
  const press = (slug, pin) => {
    const rest = ((!refused.current && readPins()) || pins).filter((one) => one !== slug);
    const next = pin && rest.length < PINS_MAX ? rest.concat(slug) : rest;
    if (!refused.current) {
      try {
        localStorage.setItem(PINS_KEY, JSON.stringify(next));
      } catch (_) {
        refused.current = true;
      }
    }
    setPins(next);
    return next;
  };
  return [pins, press];
}

// What each topic has not shown yet, kept in this page only. A turn is known by
// whether it is the owner's, its time, its speaker and its text. For each one the
// book holds an entry per copy: the round that first showed it and the tags it
// carried then. It follows the turns of the newest snapshot, and those that went
// missing but could still come back as new. For each topic it holds the round the
// topic was last open in.
function newBook() {
  return { state: null, started: false, round: 0, high: 0, turns: new Map(), shown: [], opened: new Map() };
}

// Takes a new snapshot into the book. The first one with a turn is the start, so
// nothing in it is unread. A copy more than there were before is new, unless it is
// far older than the newest turn seen. Then an old turn came back, as one does
// when a full window loses a newer turn.
function admit(book, state) {
  book.state = state;
  const rows = state.chat.rows;
  if (!book.started && !rows.length) {
    return;
  }
  book.round += book.started ? 1 : 0;
  const turns = new Map();
  let newest = book.high;
  book.shown = rows.map((row) => {
    // A seat may be named "you" too, so the owner's turns are kept apart.
    const id = [row.voice === "you", row.micros, row.who, row.body].join("\n");
    const copies = turns.get(id) || [];
    const before = book.turns.get(id) || [];
    const fresh = book.started && row.micros >= book.high - LATE_MICROS;
    const copy = before[copies.length] || { micros: row.micros, at: fresh ? book.round : 0, tags: row.tags || [] };
    copies.push(copy);
    turns.set(id, copies);
    newest = Math.max(newest, row.micros);
    return copy;
  });
  // A turn stamped later than the server read it must not make later turns old.
  book.high = Math.max(book.high, Math.min(newest, state.read_at * 1000000));
  // A missing turn is remembered while it is recent enough to pass for new when it
  // returns, as the turns of a transcript that one poll could not read do. The
  // ones that went missing last come first here; past MISSING_MAX the rest are
  // forgotten, and such a turn can count again if it returns while still recent.
  let room = MISSING_MAX;
  for (const [id, before] of book.turns) {
    const now = turns.get(id) || [];
    const gone = before.length - now.length;
    if (gone > 0 && gone <= room && before[0].micros >= book.high - LATE_MICROS) {
      turns.set(id, now.concat(before.slice(now.length)));
      room -= gone;
    }
  }
  book.turns = turns;
  book.started = true;
}

// How many turns of each tag arrived since its topic was last open. The owner's own
// lines never count, and neither does a tag a turn got after it arrived.
function unreadCounts(book, rows) {
  const counts = new Map();
  book.shown.forEach((copy, at) => {
    const row = rows[at];
    if (row.voice === "you") {
      return;
    }
    for (const tag of row.tags || []) {
      if (copy.tags.includes(tag) && copy.at > (book.opened.get(tag) || 0)) {
        counts.set(tag, (counts.get(tag) || 0) + 1);
      }
    }
  });
  return counts;
}

function stored() {
  try {
    return sessionStorage.getItem(KEY) || "";
  } catch (_) {
    return "";
  }
}

function keep(secret) {
  try {
    if (secret) {
      sessionStorage.setItem(KEY, secret);
    } else {
      sessionStorage.removeItem(KEY);
    }
  } catch (_) {
    // A tab without storage keeps no session across reloads.
  }
}

function api(path, init) {
  const headers = Object.assign({}, init && init.headers);
  const secret = stored();
  if (secret) {
    headers.Authorization = "Bearer " + secret;
  }
  return fetch("/api/" + path, Object.assign({}, init, {
    headers,
    cache: "no-store",
    credentials: "omit",
    redirect: "error",
  }));
}

// The one-time code from the printed link, taken once and removed from the
// address bar.
function takeCode() {
  const fragment = new URLSearchParams(location.hash.slice(1));
  const code = fragment.get("c");
  if (location.hash) {
    history.replaceState(null, "", location.pathname);
  }
  return code && /^[0-9a-f]{32}$/.test(code) ? code : "";
}

async function redeem(code) {
  const answer = await api("session", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ code }),
  });
  if (!answer.ok) {
    return false;
  }
  const body = await answer.json();
  keep(typeof body.session === "string" ? body.session : "");
  return Boolean(stored());
}

function day(micros) {
  return new Date(micros / 1000).toLocaleDateString([], {
    weekday: "short",
    day: "numeric",
    month: "short",
  });
}

function clock(micros) {
  return new Date(micros / 1000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function Row({ row }) {
  return html`<article class=${"msg " + row.voice + " k-" + row.kind} data-tags=${(row.tags || []).join(" ")}>
    <div class="who">
      <b>${row.who}</b>
      ${row.what ? html`<span>${row.what}</span>` : null}
      <time>${clock(row.micros)}</time>
    </div>
    ${row.body ? html`<div class="bub">${row.body}</div>` : null}
  </article>`;
}

// The shared chat; with a topic selected, the turns the server tagged with it.
// While the newest turn is in view the stream stays at it: after a new turn, a
// resize and a view switch alike. A hidden stream (a narrow screen's other
// view) reads no scroll, so hiding it never lets go of the newest turn.
function Stream({ chat, tag }) {
  const box = useRef(null);
  const end = useRef(null);
  const pinned = useRef(true);
  const onScroll = useCallback((event) => {
    const box = event.currentTarget;
    pinned.current = box.scrollHeight - box.scrollTop - box.clientHeight < 48;
  }, []);
  useEffect(() => {
    if (pinned.current && end.current) {
      end.current.scrollIntoView({ block: "end" });
    }
  }, [chat]);
  useEffect(() => {
    const stream = box.current;
    if (!stream || typeof ResizeObserver !== "function") {
      return undefined;
    }
    const follow = new ResizeObserver(() => {
      if (pinned.current && stream.clientHeight) {
        stream.scrollTop = stream.scrollHeight;
      }
    });
    follow.observe(stream);
    if (stream.firstElementChild) {
      follow.observe(stream.firstElementChild);
    }
    return () => follow.disconnect();
  }, []);
  const rows = tag ? chat.rows.filter((row) => (row.tags || []).includes(tag)) : chat.rows;
  const items = [];
  let shown = "";
  for (const row of rows) {
    const date = day(row.micros);
    if (date !== shown) {
      shown = date;
      items.push(html`<div class="day" key=${"d" + date}>${date}</div>`);
    }
    items.push(html`<${Row} key=${row.key} row=${row} />`);
  }
  return html`<div class="stream" ref=${box} onScroll=${onScroll}>
    <div class="inner">
      ${chat.cut > 0 ? html`<p class="gap">${chat.cut} older turns are not shown.</p>` : null}
      ${chat.coverage.map((gap) => html`<p class="gap" key=${"c" + gap}>coverage incomplete: ${gap}</p>`)}
      ${tag && !rows.length ? html`<p class="gap">No messages with #${tag} yet.</p>` : null}
      ${items}
      <div ref=${end}></div>
    </div>
  </div>`;
}

// A memo record's time as the reader's local date and time, or the text as
// written when it is no time.
function when(text) {
  const parsed = new Date(text);
  if (!text || Number.isNaN(parsed.getTime())) {
    return text || "never";
  }
  return parsed.toLocaleString([], {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const GROUPS = {
  needs_you: "Needs you",
  moving: "Moving",
  quiet: "Quiet",
  parked: "Parked",
  closed: "Closed",
  unreadable: "Unreadable",
};

function TopicRow({ row, group, choose }) {
  const waits = row.waits.length;
  // A press keeps the focus where it was, so a draft being typed stays focused.
  return html`<button
    class="row"
    onMouseDown=${(event) => event.preventDefault()}
    onClick=${() => choose(row.slug)}
  >
    <span class=${"dot " + group}></span>
    <span class="nm">${row.title || row.slug}</span>
    <span class="meta" title=${row.reviewed}>${when(row.reviewed)}</span>
    <span class="ln">#${row.slug}${waits ? " · waits on you (" + waits + ")" : ""}</span>
  </button>`;
}

// How many open DECIDE items name a topic, and the words for them.
const decides = (row) => row.waits.filter((wait) => wait.kind === "decide").length;
const decisionWords = (count) => count + (count === 1 ? " decision" : " decisions");

// The side panel's groups, top to bottom. A topic sits in the first group whose
// `has` it meets, so it is listed once. The last group is folded: a topic there
// shows while the fold is open or while it is the selected one.
const SIDE_GROUPS = [
  // A wait is an open DECIDE item or the owner's own open ask.
  { id: "needs", label: "Needs you", has: (row) => row.state === "needs" || row.waits.length > 0 },
  { id: "moving", label: "Moving", has: (row) => row.state === "moving" },
  { id: "pinned", label: "Pinned", has: (row, pins) => pins.includes(row.slug) },
  { id: "other", label: "Unreadable", has: (row) => row.state === null && filed(row) },
  { id: "rest", label: "Quiet, parked, closed", fold: true, has: () => true },
];
const groupOf = (row, pins) => SIDE_GROUPS.find((group) => group.has(row, pins));

// Whether a topic has a row of its own, outside the fold; `keepsUnpinned` asks
// the same as if the topic were not pinned.
const keeps = (row, pins) => !groupOf(row, pins).fold;
const keepsUnpinned = (row) => keeps(row, []);

function deskRow(desk, slug) {
  for (const group of desk ? desk.groups : []) {
    const row = group.rows.find((one) => one.slug === slug);
    if (row) {
      return row;
    }
  }
  return null;
}

// Whether a topic may stay selected. One that is not closed may, wherever its
// row is. A closed one needs a row of its own, unless it is `shut`: the one that
// was closed with no row of its own when the owner opened or unpinned it.
const selectable = (row, pins, shut) =>
  row.state !== "closed" || keeps(row, pins) || row.slug === shut;

// The desk's topics in the side panel's groups, each newest activity first: the
// later of the last review and the newest chat turn that names the topic. The
// sort is stable, so a tie keeps the desk's order. An empty group is left out.
function sideGroups(desk, pins, turns) {
  const spoken = new Map();
  for (const turn of turns) {
    for (const tag of turn.tags || []) {
      spoken.set(tag, Math.max(spoken.get(tag) || 0, turn.micros));
    }
  }
  const lists = SIDE_GROUPS.map((group) => ({ group, topics: [] }));
  for (const { group: server, rows } of desk.groups) {
    for (const row of rows) {
      // In microseconds, as a turn's time is; an unreadable review counts as 0.
      const at = Math.max(Date.parse(row.reviewed) * 1000 || 0, spoken.get(row.slug) || 0);
      const list = lists[SIDE_GROUPS.indexOf(groupOf(row, pins))];
      list.topics.push({ row, server, pinned: pins.includes(row.slug), at });
    }
  }
  for (const list of lists) {
    list.topics.sort((one, other) => other.at - one.at);
  }
  return lists.filter((list) => list.topics.length);
}

// One row of the side panel. A press keeps the focus where it was, so a draft
// being typed stays focused.
function SideRow({ slug, label, title, group, pinned, unread, decisions, active, select }) {
  return html`<button
    type="button"
    class="tab"
    data-role=${slug ? "side-row" : "side-all"}
    data-slug=${slug || null}
    data-pinned=${pinned ? "" : null}
    aria-current=${active ? "true" : null}
    title=${title || null}
    onMouseDown=${(press) => press.preventDefault()}
    onClick=${() => select(slug)}
  >
    ${group ? html`<span class=${"dot " + group}></span>` : null}
    <span class="tl">${label}</span>
    ${unread ? html`<span class="n" data-role="unread">${unread}</span>` : null}
    ${decisions ? html`<span class="dc" data-role="decisions">${decisionWords(decisions)}</span>` : null}
  </button>`;
}

const SIDE_ROWS = "button:is([data-role=side-all],[data-role=side-row])";
const CURRENT_SIDE_ROW = SIDE_ROWS + "[aria-current=true]";

function moveSideFocus(event) {
  const { key, target, currentTarget } = event;
  if ((key !== "ArrowUp" && key !== "ArrowDown") || !target.matches(SIDE_ROWS) ||
      event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
    return;
  }
  const rows = [...currentTarget.querySelectorAll(SIDE_ROWS)];
  const step = key === "ArrowUp" ? -1 : 1;
  event.preventDefault();
  rows[rows.indexOf(target) + step]?.focus();
}

// How far the panel has to scroll to show `element` whole: below zero up, above
// zero down, zero when it is in view. One taller than the panel gets its top
// shown.
function offBy(panel, element) {
  if (!element) {
    return 0;
  }
  const box = panel.getBoundingClientRect();
  const own = element.getBoundingClientRect();
  const top = own.top - box.top;
  const bottom = own.bottom - box.bottom;
  return top < 0 ? top : Math.max(0, Math.min(bottom, top));
}

// How far the panel has to scroll to show its selected row whole.
const cutOff = (panel) => offBy(panel, panel.querySelector(CURRENT_SIDE_ROW));

// All first, then the groups; `active` is "" for All. The panel scrolls to show
// the selected row when the selection changes, and when a change would push a row
// it showed whole out of view. A row the owner scrolled away from stays away.
// While a narrow screen hides the panel, it neither scrolls nor reads a scroll.
// Whether the last group is open lives in this page only. Opening it scrolls the
// group into view when it is cut off: whole when it fits, else its heading at
// the top.
function SidePanel({ lists, unread, active, select }) {
  const panel = useRef(null);
  const follow = useRef(true);
  const last = useRef(active);
  const opened = useRef(false);
  const [unfolded, setUnfolded] = useState(false);
  const focused = document.activeElement;
  const hadFocus = panel.current?.contains(focused);
  const show = useCallback(() => {
    if (follow.current && panel.current && panel.current.clientHeight) {
      panel.current.scrollTop += cutOff(panel.current);
    }
  }, []);
  useLayoutEffect(() => {
    if (last.current !== active) {
      last.current = active;
      follow.current = true;
    }
    show();
    const nav = panel.current;
    if (opened.current && nav?.clientHeight) {
      const group = nav.querySelector("[data-action=toggle-group]")?.closest(".sgroup");
      nav.scrollTop += offBy(nav, group);
      follow.current = Math.abs(cutOff(nav)) <= 1;
    }
    opened.current = false;
    // A row can move to a new list and be remounted. Recover focus only if
    // the render left it on the document and the panel is still visible.
    if (hadFocus && nav?.clientHeight && document.activeElement === document.body) {
      const same = focused.dataset.action === "toggle-group"
        ? nav.querySelector("[data-action=toggle-group]")
        : [...nav.querySelectorAll(SIDE_ROWS)].find((row) => row.dataset.slug === focused.dataset.slug);
      (same || nav.querySelector(CURRENT_SIDE_ROW))?.focus({ preventScroll: true });
    }
  });
  useEffect(() => {
    window.addEventListener("resize", show);
    return () => window.removeEventListener("resize", show);
  }, []);
  const scrolled = (event) => {
    if (event.currentTarget.clientHeight) {
      follow.current = Math.abs(cutOff(event.currentTarget)) <= 1;
    }
  };
  // A press on the selected row shows it whole again, also after a scroll away.
  const pick = (slug) => {
    if (slug === active) {
      follow.current = true;
      show();
    }
    select(slug);
  };
  const topic = ({ row, server, pinned }) => {
    const title = row.title || row.slug;
    const count = unread.get(row.slug) || 0;
    const open = decides(row);
    const words = [
      title,
      "#" + row.slug,
      GROUPS[server] || server,
      pinned && "pinned",
      open && decisionWords(open),
      count && count + " unread",
    ];
    return html`<li key=${row.slug}><${SideRow}
      slug=${row.slug}
      label=${title}
      title=${words.filter(Boolean).join(" · ")}
      group=${server}
      pinned=${pinned}
      unread=${count}
      decisions=${open}
      active=${active === row.slug}
      select=${pick}
    /></li>`;
  };
  return html`<nav
    class="side"
    aria-label="Topics"
    ref=${panel}
    onScroll=${scrolled}
    onKeyDown=${moveSideFocus}
  >
    <ul><li><${SideRow} slug="" label="All" active=${!active} select=${pick} /></li></ul>
    ${lists.map(({ group, topics }) => {
      const rows = group.fold && !unfolded ? topics.filter(({ row }) => row.slug === active) : topics;
      const heading = "side-" + group.id + "-head";
      const list = "side-" + group.id + "-list";
      const label = html`${group.label} <span>${topics.length}</span>`;
      return html`<div class="sgroup" data-group=${group.id} key=${group.id}>
        <h2 class=${group.fold ? "sg fold-heading" : "sg"} id=${heading}>
          ${group.fold ? html`<button
              type="button"
              class="sg-fold"
              data-action="toggle-group"
              aria-expanded=${unfolded ? "true" : "false"}
              aria-controls=${list}
              onMouseDown=${(down) => down.preventDefault()}
              onClick=${() => {
                opened.current = !unfolded;
                setUnfolded(!unfolded);
              }}
            >${label}</button>` : label}
        </h2>
        <ul id=${list} aria-labelledby=${heading}>${rows.map(topic)}</ul>
      </div>`;
    })}
  </nav>`;
}

// The All row's panel. The new-topic line comes first, above every list that
// grows on its own, so no control moves under the pointer. Every decision is
// listed; one that tags no topic is answered here, a tagged one on its topic's card.
// All topics lists every topic in the side panel's order (`lists`): its groups,
// the folded one too, each newest activity first.
function AllPanel({ desk, lists, open, revisit, lines, route }) {
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const all = [];
  for (const { topics } of lists) {
    for (const { row, server } of topics) {
      const words = (row.slug + " " + (row.title || "")).toLowerCase();
      if (!needle || words.includes(needle)) {
        all.push(html`<${TopicRow} key=${"a" + row.slug} row=${row} group=${server} choose=${open} />`);
      }
    }
  }
  const untagged = desk.decide.filter((row) => row.tags.length === 0);
  const tagged = desk.decide.filter((row) => row.tags.length > 0);
  const skipped = desk.skipped;
  const left = skipped.claimed + skipped.unproven + skipped.invalid;
  return html`<aside class="panel allp" data-panel="all" aria-label="All topics">
    <${NewTopic} lines=${lines} route=${route} desk=${desk} />
    ${revisit.entry
      ? html`<${Revisit} entry=${revisit.entry} count=${revisit.count} next=${revisit.next} open=${open} walk=${revisit.walk} />`
      : null}
    ${desk.decide.length
      ? html`<section class="decisions" data-section="decisions">
          <div class="grp need"><span>Decisions</span><span>${desk.decide.length}</span></div>
          ${untagged.length
            ? html`<div data-section="untagged">
                <div class="grp"><span>Decisions without a topic</span></div>
                ${untagged.map(
                  (row) => html`<${DecisionAnswer}
                    key=${row.item}
                    lines=${lines}
                    route=${route}
                    item=${row.item}
                    text=${row.text}
                    decision=${row}
                    compose=${desk.compose || COMPOSE}
                    tag=""
                  />`,
                )}
              </div>`
            : null}
          ${tagged.map(
            (row) => html`<div class="dline" key=${row.item} data-item=${row.item}>
              <p class="dq-text"><span class="mute sm">${row.item}</span> ${row.text}</p>
              <p class="dlinks">
                ${row.reserved ? html`<span class="mute sm">reserved</span>` : null}
                ${row.tags.map(
                  (tag) => html`<button
                    type="button"
                    class="link"
                    key=${tag}
                    data-action="open-tab"
                    data-slug=${tag}
                    onClick=${() => open(tag)}
                  >#${tag}</button>`,
                )}
              </p>
            </div>`,
          )}
        </section>`
      : null}
    <div class="grp"><span>All topics</span><span>${all.length}</span></div>
    <input
      type="search"
      name="topic-search"
      placeholder="Search topics"
      aria-label="Search topics"
      value=${query}
      onInput=${(event) => setQuery(event.currentTarget.value)}
    />
    ${all}
    ${desk.gaps.map((gap, at) => html`<p class="note" key=${"g" + at}>${gap}</p>`)}
    ${left > 0
      ? html`<p class="note">
          ${left} archives not read: ${skipped.claimed} being written, ${skipped.unproven} not proven from
          the orchestrator seat, ${skipped.invalid} damaged.
        </p>`
      : null}
  </aside>`;
}

// Every shown field of a card, each with the record it came from.
const FIELDS = [
  ["title", "Title"],
  ["state", "State"],
  ["what", "What"],
  ["last", "Last"],
  ["next", "Next"],
  ["src", "Sources"],
  ["close", "Closed because"],
];

// The two kinds of wait: a DECIDE item on the board waits for the owner; an
// open ask from the owner's console is owed to the owner.
const WAITS = [
  ["decide", "Waits for you"],
  ["ask", "Owed to you"],
];

// A record's source: its session and its time; the tooltip holds its
// incarnation's uuid and the time as written.
function Source({ origin }) {
  return html`<span class="src" title=${origin.uuid + " · " + origin.at}>
    ${origin.session} · ${when(origin.at)}
  </span>`;
}

// Whether two records come from the same session, incarnation and time.
const sameOrigin = (one, other) =>
  Boolean(one && other) && one.session === other.session && one.uuid === other.uuid && one.at === other.at;

// The selected topic's panel: its card and the decisions that wait on it. The
// header names the source of the newest record once; a field or an earlier
// record names its own only when it differs from that one.
function Card({ card, closeLine, outcome, lines, route, decide, compose, pinned, full, pin }) {
  const fields = card.fields;
  const origin = card.origin || null;
  const own = (record) => (sameOrigin(record, origin) ? null : html`<${Source} origin=${record} />`);
  // Only Quiet, Moving and Parked cards can be closed from here; a card with a
  // wait shows the button disabled and says why, a closed or unreadable one
  // shows none.
  const closable = Boolean(card.verdict);
  const blocked = card.group === "needs_you";
  const stranded = closable && !route;
  return html`<aside class="panel card" data-panel="topic" data-slug=${card.slug} aria-label="Topic">
    <div class="hd">
      <h2>${fields.title ? fields.title.value : card.slug}</h2>
      <button
        type="button"
        class="btn pin"
        data-action="pin"
        aria-pressed=${pinned ? "true" : "false"}
        disabled=${full}
        onMouseDown=${(down) => down.preventDefault()}
        onClick=${() => pin(card.slug, !pinned)}
      >${pinned ? "Unpin topic" : "Pin topic"}</button>
    </div>
    ${full ? html`<p class="note">${PINS_MAX} topics are pinned; unpin one first.</p>` : null}
    <p class="sub">
      #${card.slug} · ${GROUPS[card.group] || card.group} · reviewed ${when(card.reviewed)}${origin
        ? html` · <span data-role="card-origin" title=${origin.uuid}>${origin.session}</span>`
        : null}
    </p>
    ${WAITS.map(([kind, name]) => {
      const waits = card.waits.filter((wait) => wait.kind === kind);
      return waits.length
        ? html`<div class="fld need" key=${kind}>
            <h3>${name}</h3>
            <ul class=${kind === "decide" ? "dqs" : ""}>${waits.map((wait, at) =>
              html`<li key=${kind + at}>${kind === "decide" && wait.item
                ? html`<${DecisionAnswer}
                    lines=${lines}
                    route=${route}
                    item=${wait.item}
                    text=${wait.text}
                    decision=${decide.find((row) => row.item === wait.item && row.text === wait.text) || null}
                    compose=${compose}
                    mark=${wait.id}
                    tag=${card.slug}
                  />`
                : html`${wait.text}${wait.id ? html` <span class="mute sm">${wait.id}</span>` : null}`}</li>`,
            )}</ul>
          </div>`
        : null;
    })}
    ${card.unreadable !== null
      ? html`<div class="fld need"><h3>Unreadable checkpoint</h3><p class="raw">${card.unreadable}</p></div>`
      : null}
    ${FIELDS.filter(([key]) => fields[key]).map(
      ([key, name]) => html`<div class="fld" key=${key}>
        <h3><span>${name}</span>${own(fields[key])}</h3>
        <p class="raw">${fields[key].value}</p>
      </div>`,
    )}
    ${card.history.length
      ? html`<div class="fld">
          <h3>Earlier records</h3>
          <ul class="hist">${card.history.map(
            (entry, at) => html`<li key=${"h" + at}>
              ${own(entry)}
              ${entry.fields.map(
                ([name, value], n) => html`<div class="raw" key=${n}>${name ? html`<span class="mute">${name + ": "}</span>` : null}${value}</div>`,
              )}
            </li>`,
          )}</ul>
        </div>`
      : null}
    ${closable || blocked
      ? html`<div class="cact">
          <button class="btn cl-line" disabled=${!closable || !route} title=${stranded ? NO_ROUTE : null} onClick=${closeLine}>
            Close with one line
          </button>
        </div>`
      : null}
    ${stranded ? html`<p class="note" data-role="no-route">${NO_ROUTE}</p>` : null}
    ${outcome ? html`<p class="note cl-note" role="status">${outcome}</p>` : null}
    ${blocked
      ? html`<p class="note">This topic is under Needs you, so it cannot be closed from here.</p>`
      : null}
  </aside>`;
}

function Head({ state }) {
  const seat = state ? state.orchestrator : null;
  const words = seat
    ? (seat.name || "no orchestrator") + " · " + seat.state + (seat.why ? " · " + seat.why : "")
    : "connecting";
  return html`<header class="top">
    <div class="brand">ae</div>
    <div class="chip">${words}</div>
    <div class="sp"></div>
    ${state ? html`<span class="mute sm">core ${state.version}</span>` : null}
  </header>`;
}

function pause(ms) {
  return new Promise((done) => setTimeout(done, ms));
}

// The words of a refusal the server sent, or its status.
async function refusal(answer) {
  try {
    const body = await answer.json();
    if (body && typeof body.error === "string") {
      return body.error;
    }
  } catch (_) {
    // No JSON: the status says it.
  }
  return "ae web answered " + answer.status;
}

// A receipt as the page shows it: a label for the outcome and the server's
// words verbatim. The words of an unknown outcome already say so.
function receiptNote(receipt) {
  const words = typeof receipt.words === "string" ? receipt.words : "";
  const id = typeof receipt.id === "string" ? receipt.id : "";
  if (receipt.outcome === "sent") {
    return { kind: "sent", label: "sent to " + receipt.seat + " in " + receipt.to, words, id };
  }
  if (receipt.outcome === "refused") {
    return { kind: "refused", label: "refused", words, id };
  }
  return { kind: "unknown", label: "", words: words || "unknown", id };
}

function line(kind, words) {
  return { kind, label: "", words, id: "" };
}

// A new draft: the draft id (nonce) is minted and the exact body, text, route
// and id, is fixed. Resolves to { body } or, when no draft could be made, to
// { note } saying why; nothing was sent then.
async function draft(route, text, decide) {
  const answer = await api("nonce");
  if (answer.status !== 200) {
    return { note: line("error", answer.status === 401 ? NO_SESSION : await refusal(answer)) };
  }
  const minted = await answer.json();
  const nonce = minted && typeof minted.nonce === "string" ? minted.nonce : "";
  if (!nonce) {
    return { note: line("error", "ae web gave no draft id") };
  }
  const body = { text, route: route.id, nonce };
  if (decide) {
    body.decide = decide;
  }
  return { body: JSON.stringify(body) };
}

// One carry: posts the exact body again and again until the server answers
// with a receipt, at most SEND_TRIES times, SEND_AGAIN_MS apart, and shows each
// wait through `show`. It resolves to { receipt }, the server's receipt (200)
// whatever its outcome; to { spent, note }, a refusal of this very body (400,
// 409, 410, 413 and the other 4xx but 401 and 408), when nothing was sent and
// the next send is a new draft id; or to { open, note }, no receipt: a lost
// session (401), a request cut by its deadline (408), any other answer, the end
// of the tries or an unreachable listener. The body may have carried, so the
// caller keeps it and asks again with the same body and id.
async function carry(body, show) {
  try {
    for (let tries = 0; tries < SEND_TRIES; tries += 1) {
      const answer = await api("send", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body,
      });
      if (answer.status === 200) {
        return { receipt: await answer.json() };
      }
      if (answer.status === 202 || answer.status === 503) {
        show(line("wait", answer.status === 202 ? "queued" : "busy, retrying"));
        await pause(SEND_AGAIN_MS);
        continue;
      }
      const note = line("error", answer.status === 401 ? NO_SESSION : await refusal(answer));
      const open = answer.status === 401 || answer.status === 408;
      if (answer.status >= 400 && answer.status < 500 && !open) {
        return { spent: true, note };
      }
      return { open: true, note };
    }
    return { open: true, note: line("error", "no receipt yet: it may still land; Check again asks once more") };
  } catch (_) {
    return { open: true, note: line("error", UNREACHABLE + "; Check again asks once more") };
  }
}

// ---------------------------------------------------------------------------
// Two owner lines sent from the desk: the answer to a decision (`D67 <answer>`)
// and a new topic (`#<slug> new <title>`). Each is pressed, counted down for
// UNDO_MS with an Undo, and only then drafted and carried; nothing is minted or
// posted before the deadline. A press captures the route and, for a decision,
// the bind of the text shown, and they are never retargeted. A changed route or
// decision, a hidden page and an Undo cancel the line before it is sent.

const UNDO_MS = 5000;
const TICK_MS = 250;
// How long a topic asked for is shown as asked: a filed topic that shows up ends it sooner.
const ASKED_MS = 10 * 60 * 1000;
// A row is a filed topic when the orchestrator recorded it: a title, a state or a review. The
// server also shows a bare row for each tag of an open ask, with none of the three, and the
// request for a new topic is such an ask, so that row is no proof the topic exists.
const filed = (row) => row.title !== null || row.state !== null || row.reviewed !== "";
const SLUG_FOLD = { "ä": "ae", "ö": "oe", "ü": "ue", "ß": "ss" };
const ONE_LINE = "One line only: no line break or control character.";
const COMPOSE = { slug_max: 48, title_max: 80, answer_max: 120 };

// The characters Rust's str::trim strips: the Unicode White_Space set. U+FEFF
// is not in it, so String.trim, which strips it, is never used for a line.
function isRustSpace(code) {
  return (
    (code >= 0x09 && code <= 0x0d) ||
    code === 0x20 ||
    code === 0x85 ||
    code === 0xa0 ||
    code === 0x1680 ||
    (code >= 0x2000 && code <= 0x200a) ||
    code === 0x2028 ||
    code === 0x2029 ||
    code === 0x202f ||
    code === 0x205f ||
    code === 0x3000
  );
}

function rustTrim(text) {
  const chars = Array.from(text);
  let from = 0;
  let to = chars.length;
  while (from < to && isRustSpace(chars[from].codePointAt(0))) {
    from += 1;
  }
  while (to > from && isRustSpace(chars[to - 1].codePointAt(0))) {
    to -= 1;
  }
  return chars.slice(from, to).join("");
}

// A typed line part as the page sends it: 1 to `max` Unicode scalar values with
// no control character, line break or paragraph break once trimmed. The raw text
// is checked first, so a break or control character at either end is refused as
// it is anywhere else and never trimmed away; a refused text is returned empty.
// The problem is empty for an empty text, which only disables the button.
function lineText(raw, max) {
  if (REASON_BAD.test(raw)) {
    return { text: "", problem: ONE_LINE };
  }
  const text = rustTrim(raw);
  const length = Array.from(text).length;
  if (length > max) {
    return { text, problem: "At most " + max + " characters; this has " + length + "." };
  }
  return { text, problem: "" };
}

// A slug as ae::topic::is_slug takes it: a lowercase letter or digit first,
// then lowercase letters, digits and hyphens, at most `max` bytes.
function slugOk(slug, max) {
  return slug.length <= max && /^[a-z0-9][a-z0-9-]*$/.test(slug);
}

// The slug a title suggests: umlauts folded, lowercase, every other run of
// characters one hyphen, no hyphen at either end, cut at `max`.
function deriveSlug(title, max) {
  let folded = "";
  for (const ch of rustTrim(title).toLowerCase()) {
    folded += SLUG_FOLD[ch] || ch;
  }
  const slug = folded.replace(/[^a-z0-9]+/g, "-").replace(/^-+/, "");
  return slug.slice(0, max).replace(/-+$/, "");
}

const BLANK = Object.freeze({
  phase: "idle",
  fields: Object.freeze({}),
  note: null,
  line: "",
  item: "",
  bind: "",
  deadline: 0,
});

// What the owner says about one decision before anything is sent.
const QUIET_TALK = Object.freeze({ shown: false, text: "", phase: "idle", note: null, body: "" });
// The longest question or objection, in Unicode scalar values.
const TALK_MAX = 500;

// The lines in flight, one record per key: "<item> <bind>" for a decision and
// "topic" for a new topic. A record is idle (editable), count (undo window),
// send (drafting or carrying), open (no receipt: the same body is asked again)
// or sent (a decision already answered). The decision's text is part of its
// key, so a board that changed the text starts a new record: an answer, a
// receipt or a Check again never lands on text it was not made for. Every view
// that shows one decision shares one record, and its talk, so an open attempt
// outlives a change of the selected topic.
function makeLines(live, redraw) {
  const rec = (key, item, bind) => {
    if (!live.recs.has(key)) {
      live.recs.set(key, { ...BLANK, fields: {}, item: item || "", bind: bind || "" });
    }
    return live.recs.get(key);
  };
  const current = (item) => live.decide.find((row) => row.item === item) || null;
  const ticking = () => {
    const counting = [...live.recs.values()].some((r) => r.phase === "count");
    if (counting && !live.tick) {
      live.tick = setInterval(redraw, TICK_MS);
    } else if (!counting && live.tick) {
      clearInterval(live.tick);
      live.tick = 0;
    }
  };
  // A record whose text left the board: it ends with what it could say.
  const supersede = (key, r) => {
    clearTimeout(r.timer);
    live.recs.delete(key);
    if (r.phase === "count") {
      live.said.set(r.item, line("error", "Cancelled: the decision changed, reload."));
    } else if (r.phase === "open" || r.phase === "send") {
      live.said.set(
        r.item,
        line("error", "An earlier answer to " + r.item + " may have reached the orchestrator: the decision changed, reload."),
      );
    } else if (r.phase === "idle" && (r.fields.answer || r.fields.choice)) {
      live.said.set(r.item, line("error", "The decision changed: your answer was cleared."));
    }
    ticking();
    redraw();
  };
  const talking = (item) => {
    if (!live.talks.has(item)) {
      live.talks.set(item, { ...QUIET_TALK });
    }
    return live.talks.get(item);
  };
  const api = {
    talk: (item) => live.talks.get(item) || QUIET_TALK,
    showTalk(item) {
      talking(item).shown = true;
      redraw();
    },
    typeTalk(item, text) {
      const t = talking(item);
      if (t.phase !== "idle") {
        return;
      }
      t.text = text;
      t.note = null;
      redraw();
    },
    // A question or objection is an ordinary chat line, `#<tag> D67: <words>`,
    // sent as the composer sends: no countdown and no decision proof. An open
    // attempt posts its exact body again.
    async discuss(item, tag, words) {
      const t = talking(item);
      if (t.phase === "send") {
        return;
      }
      if (!t.body && !live.route) {
        t.note = line("error", NO_ROUTE);
        redraw();
        return;
      }
      t.phase = "send";
      t.note = line("wait", t.body ? "asking again" : "sending");
      redraw();
      try {
        if (!t.body) {
          const made = await draft(live.route, (tag ? "#" + tag + " " : "") + item + ": " + words);
          if (!made.body) {
            t.phase = "idle";
            t.note = made.note;
            redraw();
            return;
          }
          t.body = made.body;
        }
        const done = await carry(t.body, (note) => {
          t.note = note;
          redraw();
        });
        if (done.receipt) {
          t.note = receiptNote(done.receipt);
          t.body = "";
          t.phase = "idle";
          if (t.note.kind !== "refused") {
            t.text = "";
          }
        } else if (done.spent) {
          t.note = done.note;
          t.body = "";
          t.phase = "idle";
        } else {
          t.note = done.note;
          t.phase = "open";
        }
      } catch (_) {
        t.phase = t.body ? "open" : "idle";
        t.note = line("error", UNREACHABLE + (t.body ? "; Check again asks once more" : ""));
      }
      redraw();
    },
    key: (item, bind) => item + " " + bind,
    get: (key) => live.recs.get(key) || BLANK,
    said: (item) => live.said.get(item) || null,
    asked: () => live.asked.filter((one) => Date.now() - one.at < ASKED_MS && !live.known.has(one.slug)).map((one) => one.slug),
    set(key, name, value, owner) {
      api.setMany(key, { [name]: value }, owner);
    },
    setMany(key, fields, owner) {
      const r = rec(key, owner ? owner.item : "", owner ? owner.bind : "");
      if (r.phase !== "idle") {
        return;
      }
      r.fields = { ...r.fields, ...fields };
      r.note = null;
      live.said.delete(r.item);
      redraw();
    },
    // A decision is pressed only while the board's own row still holds its
    // bind, is not reserved and offers `choice`, "" for a typed answer.
    press(key, text, decide, slug, choice) {
      const r = rec(key, decide ? decide.item : "", decide ? decide.bind : "");
      if (r.phase !== "idle") {
        return;
      }
      const row = decide ? current(decide.item) : null;
      const offered =
        !decide ||
        (Boolean(row) &&
          row.bind === decide.bind &&
          !row.reserved &&
          (choice ? Boolean(row.choices) && row.choices.options.some((one) => String(one.n) === choice) : !row.choices));
      if (!offered) {
        r.note = line("error", "Not sent: the decision changed, reload.");
        redraw();
        return;
      }
      if (!live.route) {
        r.note = line("error", NO_ROUTE);
        redraw();
        return;
      }
      r.phase = "count";
      r.line = text;
      r.decide = decide;
      r.slug = slug || "";
      r.route = live.route;
      r.deadline = Date.now() + UNDO_MS;
      r.note = null;
      r.body = "";
      live.said.delete(r.item);
      r.timer = setTimeout(() => api.fire(key), UNDO_MS);
      ticking();
      redraw();
    },
    // The line is dropped before anything was sent; `why` is null for an Undo.
    cancel(key, why) {
      const r = live.recs.get(key);
      if (!r || r.phase !== "count") {
        return;
      }
      clearTimeout(r.timer);
      r.phase = "idle";
      r.note = why ? line("error", "Cancelled: " + why) : line("wait", "Undone: nothing was sent.");
      ticking();
      redraw();
    },
    cancelAll(why) {
      for (const key of [...live.recs.keys()]) {
        api.cancel(key, why);
      }
    },
    async fire(key) {
      const r = live.recs.get(key);
      if (!r || r.phase !== "count") {
        return;
      }
      const left = r.deadline - Date.now();
      if (left > 0) {
        r.timer = setTimeout(() => api.fire(key), left);
        return;
      }
      if (document.hidden) {
        api.cancel(key, "the page was hidden.");
        return;
      }
      if (!live.route || live.route.id !== r.route.id) {
        api.cancel(key, "the orchestrator changed, reload.");
        return;
      }
      if (r.slug && live.occupied.has(r.slug)) {
        api.cancel(key, "#" + r.slug + " is a topic already.");
        return;
      }
      const row = r.decide ? current(r.decide.item) : null;
      if (r.decide && (!row || row.bind !== r.decide.bind)) {
        supersede(key, r);
        return;
      }
      r.phase = "send";
      r.note = line("wait", "sending");
      ticking();
      redraw();
      try {
        const made = await draft(r.route, r.line, r.decide);
        if (!made.body) {
          r.phase = "idle";
          r.note = made.note;
          redraw();
          return;
        }
        r.body = made.body;
        await api.carry(r);
      } catch (_) {
        r.phase = r.body ? "open" : "idle";
        r.note = line("error", UNREACHABLE + (r.body ? "; Check again asks once more" : ""));
        redraw();
      }
    },
    // The frozen body, posted again until the server answers it. `r` is the
    // record the attempt belongs to, so a record that has left the board is
    // only ever updated, never shown again.
    async carry(r) {
      const done = await carry(r.body, (note) => {
        r.note = note;
        redraw();
      });
      if (done.receipt) {
        const shown = receiptNote(done.receipt);
        if (shown.kind === "refused") {
          // Nothing was sent: the draft stays to be edited and sent as a new one.
          r.phase = "idle";
          r.note = shown;
        } else if (r.decide) {
          r.phase = "sent";
          r.note =
            shown.kind === "sent"
              ? { kind: "sent", label: "", words: 'Answered "' + r.line + '": waiting for the orchestrator.', id: shown.id }
              : shown;
        } else {
          // A new topic is done with: the form is free for the next one.
          r.phase = "idle";
          r.note = shown;
          r.fields = {};
          if (shown.kind === "sent" && r.slug && !live.known.has(r.slug)) {
            live.asked = live.asked.filter((one) => one.slug !== r.slug);
            live.asked.push({ slug: r.slug, at: Date.now() });
            api.sweepAsked();
          }
        }
      } else if (done.spent) {
        r.phase = "idle";
        r.note = done.note;
      } else {
        r.phase = "open";
        r.note = done.note;
      }
      redraw();
    },
    async check(key) {
      const r = live.recs.get(key);
      if (!r || r.phase !== "open") {
        return;
      }
      r.phase = "send";
      r.note = line("wait", "asking again");
      redraw();
      try {
        await api.carry(r);
      } catch (_) {
        r.phase = "open";
        r.note = line("error", UNREACHABLE + "; Check again asks once more");
        redraw();
      }
    },
    // The asked topics that now have a row or have waited too long stop being
    // kept, and a timer redraws the page at the oldest one's expiry, because a
    // quiet page gets no new snapshot to do it.
    sweepAsked() {
      clearTimeout(live.askedTimer);
      const now = Date.now();
      live.asked = live.asked.filter((one) => now - one.at < ASKED_MS && !live.known.has(one.slug));
      if (live.asked.length) {
        const due = Math.min(...live.asked.map((one) => one.at + ASKED_MS)) - now;
        live.askedTimer = setTimeout(() => {
          api.sweepAsked();
          redraw();
        }, Math.max(due, 0) + 10);
      }
    },
    // A new snapshot: a counting line whose route moved is cancelled, a record
    // whose decision text left the board ends, an asked topic that now has a
    // row (or has waited too long) stops being shown.
    reconcile() {
      for (const [key, r] of [...live.recs]) {
        if (r.phase === "count" && (!live.route || live.route.id !== r.route.id)) {
          api.cancel(key, "the orchestrator changed, reload.");
        } else if (r.phase === "count" && r.slug && live.occupied.has(r.slug)) {
          api.cancel(key, "#" + r.slug + " is a topic already.");
        }
        const row = r.item ? current(r.item) : null;
        if (r.item && (!row || row.bind !== r.bind)) {
          supersede(key, r);
        }
      }
      for (const item of [...live.said.keys()]) {
        if (!current(item)) {
          live.said.delete(item);
        }
      }
      api.sweepAsked();
    },
  };
  return api;
}

// The lines in flight for the whole page, and what moves them: every snapshot
// reconciles them, and a page that is hidden drops every counting line.
function useLines(state) {
  const [, setVersion] = useState(0);
  const box = useRef(null);
  if (box.current === null) {
    const live = { recs: new Map(), talks: new Map(), said: new Map(), asked: [], askedTimer: 0, known: new Set(), occupied: new Set(), tick: 0, route: null, decide: [] };
    live.api = makeLines(live, () => setVersion((n) => n + 1));
    box.current = live;
  }
  const live = box.current;
  const desk = state && state.desk ? state.desk : null;
  live.route = state ? state.route : null;
  live.decide = desk && desk.decide ? desk.decide : [];
  const rows = desk ? desk.groups.flatMap((group) => group.rows) : [];
  // occupied: any row, bare or filed, which the form and the countdown fences refuse a new topic for;
  // known: the filed ones, which are the only proof for the asked chip.
  live.occupied = new Set(rows.map((row) => row.slug));
  live.known = new Set(rows.filter(filed).map((row) => row.slug));
  useEffect(() => {
    live.api.reconcile();
  }, [state]);
  useEffect(() => {
    const hide = () => {
      if (document.hidden) {
        live.api.cancelAll("the page was hidden.");
      }
    };
    document.addEventListener("visibilitychange", hide);
    return () => document.removeEventListener("visibilitychange", hide);
  }, []);
  return live.api;
}

// What one line control says: a paste it refused, the countdown with the seat
// the line will reach, or the note the last attempt left (or, for a record
// that has none yet, what its predecessor on this item said).
function lineNote(r, pasted, said) {
  if (pasted) {
    return line("error", pasted);
  }
  if (r.phase === "count") {
    const left = Math.max(1, Math.ceil((r.deadline - Date.now()) / 1000));
    return line("wait", 'Sending "' + r.line + '" to ' + r.route.seat + " in " + r.route.to + ", " + left + " s left");
  }
  return r.note || (r.phase === "idle" ? said : null);
}

function Note({ note, counting }) {
  return note
    ? html`<p class=${"receipt k-" + note.kind} data-role="status" role=${counting ? null : "status"}>
        ${note.label ? html`<b>${note.label}</b>${note.words ? " " : ""}` : null}${note.words}
        ${note.id ? html` <span class="mute sm">${note.id}</span>` : null}
      </p>`
    : null;
}

// A paste that holds a line break or a control character is refused whole,
// before the input ever sees it. True when it was refused.
const PASTE_REFUSED = "Paste cancelled: the text holds a line break or control character.";

function refusesPaste(event) {
  const clip = event.clipboardData ? event.clipboardData.getData("text") : "";
  if (REASON_BAD.test(clip)) {
    event.preventDefault();
    return true;
  }
  return false;
}

// The controls of one line: the status, the Undo of a counting line and the
// Check again of an open one.
function LineActions({ lines, id, r, pasted, said, focus }) {
  return html`<${Note} note=${lineNote(r, pasted, said)} counting=${r.phase === "count"} />
    ${r.phase === "count"
      ? html`<button
          type="button"
          class="dq-btn"
          data-action="undo"
          onClick=${() => {
            lines.cancel(id, null);
            if (focus.current) {
              focus.current.focus();
            }
          }}
        >
          Undo
        </button>`
      : null}
    ${r.phase === "open"
      ? html`<button type="button" class="dq-btn" data-action="check" onClick=${() => lines.check(id)}>
          Check again
        </button>`
      : null}`;
}

// One decision of the board with its answer. `decision` is the board's own row
// for it, and the controls are live only when its text is the text shown here: a
// card that is behind the board shows its text with Answer off. A reserved
// decision is shown and never answered here. A text that reads as numbered
// options is answered by one of them, as `D67 2`; any other by one line. `tag`
// is the topic whose card shows it, for talk about it.
function DecisionAnswer({ lines, route, item, text, decision, compose, mark, tag }) {
  const aligned = Boolean(decision) && decision.text === text;
  const reserved = aligned && decision.reserved;
  const choices = aligned ? decision.choices : null;
  const key = aligned ? lines.key(item, decision.bind) : "";
  const r = aligned ? lines.get(key) : BLANK;
  const [pasted, setPasted] = useState("");
  const input = useRef(null);
  const form = useRef(null);
  const raw = r.fields.answer || "";
  const checked = lineText(raw, compose.answer_max);
  const picked =
    choices && choices.options.some((option) => String(option.n) === r.fields.choice) ? r.fields.choice : "";
  const reply = choices ? picked : checked.problem ? "" : checked.text;
  const idle = r.phase === "idle";
  const ready = aligned && idle && Boolean(route) && reply !== "";
  const pick = (number) => lines.set(key, "choice", String(number), { item, bind: decision.bind });
  const submit = (event) => {
    event.preventDefault();
    if (ready && !reserved) {
      lines.press(key, item + " " + reply, { item, bind: decision.bind }, "", picked);
    }
  };
  const heading = (words) =>
    html`<p class="dq-text"><span class="mute sm">${item}</span> ${words}${mark ? html` <span class="mute sm">${mark}</span>` : null}</p>`;
  if (reserved) {
    return html`<form class="decision-answer dq" data-item=${item} data-reserved="" onSubmit=${submit}>
      ${heading(text)}
      <p class="dq-reserved" data-role="reserved">Reserved: confirm in the orchestrator pane or on Telegram.</p>
      <${Note} note=${lines.said(item)} />
    </form>`;
  }
  const answer = html`<button type="submit" class="dq-go" data-action="answer" disabled=${!ready}>Answer</button>`;
  return html`<form ref=${form} class="decision-answer dq" data-item=${item} tabIndex="-1" onSubmit=${submit}>
    ${heading(choices ? choices.question : text)}
    ${choices
      ? html`<fieldset class="dq-choices" data-role="choices" disabled=${!idle} aria-label=${"Options for " + item}>
            ${choices.options.map(
              (option) => html`<label
                key=${option.n}
                class="dq-choice"
                data-recommended=${option.n === choices.recommend ? "" : null}
              >
                <input
                  type="radio"
                  name=${"choice-" + item}
                  data-choice=${option.n}
                  value=${option.n}
                  checked=${picked === String(option.n)}
                  onChange=${() => pick(option.n)}
                />
                <span>${option.n}) ${option.label}</span>
                ${option.n === choices.recommend ? html`<span class="dq-rec">recommended</span>` : null}
              </label>`,
            )}
          </fieldset>
          <div class="dq-row">
            ${answer}
            ${choices.recommend
              ? html`<button
                  type="button"
                  class="dq-btn"
                  data-action="recommend"
                  disabled=${!idle || !route}
                  title=${route ? null : NO_ROUTE}
                  onClick=${() => pick(choices.recommend)}
                >
                  Pick the recommended
                </button>`
              : null}
          </div>`
      : html`<div class="dq-row">
          <input
            ref=${input}
            class="dq-input"
            type="text"
            name=${"answer-" + item}
            data-field="answer"
            aria-label=${"Answer to " + item}
            placeholder="Answer"
            autocomplete="off"
            autocapitalize="off"
            spellcheck="false"
            enterkeyhint="send"
            value=${raw}
            readOnly=${!idle}
            disabled=${!aligned}
            onInput=${(event) => {
              setPasted("");
              lines.set(key, "answer", event.currentTarget.value, { item, bind: decision.bind });
            }}
            onPaste=${(event) => {
              if (refusesPaste(event)) {
                setPasted(PASTE_REFUSED);
              }
            }}
          />
          ${answer}
        </div>`}
    ${aligned && idle && !choices && checked.problem ? html`<p class="dq-hint" data-role="problem">${checked.problem}</p>` : null}
    ${aligned
      ? html`<${LineActions} lines=${lines} id=${key} r=${r} pasted=${pasted} said=${lines.said(item)} focus=${choices ? form : input} />`
      : html`<p class="dq-hint" data-role="problem">This text is not the board's now: waiting for the next refresh.</p>`}
    ${aligned ? html`<${Talk} lines=${lines} route=${route} item=${item} tag=${tag} />` : null}
    ${aligned && !route ? html`<p class="dq-hint" data-role="no-route">${NO_ROUTE}</p>` : null}
  </form>`;
}

// Talk about one decision instead of answering it: its own line, its own
// status and Check again. It sits after the answer's status in the form, and
// Enter in it never presses Answer. Without a route nothing new is sent; an open
// attempt still checks its own receipt.
function Talk({ lines, route, item, tag }) {
  const t = lines.talk(item);
  const [pasted, setPasted] = useState("");
  const checked = lineText(t.text, TALK_MAX);
  const idle = t.phase === "idle";
  const open = t.phase === "open";
  const ready = open || (idle && Boolean(route) && checked.text !== "" && checked.problem === "");
  const send = () => {
    if (ready) {
      lines.discuss(item, tag, checked.text);
    }
  };
  return html`<div class="dq-talk">
    ${t.shown
      ? html`<div class="dq-row">
          <input
            class="dq-input"
            type="text"
            name=${"talk-" + item}
            data-field="talk"
            aria-label=${"Question or objection about " + item}
            placeholder="Question or objection"
            autocomplete="off"
            enterkeyhint="send"
            value=${t.text}
            readOnly=${!idle}
            onInput=${(event) => {
              setPasted("");
              lines.typeTalk(item, event.currentTarget.value);
            }}
            onPaste=${(event) => {
              if (refusesPaste(event)) {
                setPasted(PASTE_REFUSED);
              }
            }}
            onKeyDown=${(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                if (!event.isComposing) {
                  send();
                }
              }
            }}
          />
          <button type="button" class="dq-btn" data-action="talk-send" disabled=${!ready} onClick=${send}>
            ${open ? "Check again" : "Send"}
          </button>
        </div>`
      : html`<button
          type="button"
          class="dq-btn"
          data-action="talk"
          disabled=${!route}
          title=${route ? null : NO_ROUTE}
          onClick=${() => lines.showTalk(item)}
        >
          Talk it over
        </button>`}
    ${idle && t.shown && checked.problem ? html`<p class="dq-hint" data-role="problem">${checked.problem}</p>` : null}
    <${Note} note=${pasted ? line("error", pasted) : t.note} />
  </div>`;
}

// A request for a new topic: a title, the slug it suggests (or one typed) and
// the exact line the orchestrator will read.
function NewTopic({ lines, route, desk }) {
  const compose = desk.compose || COMPOSE;
  const r = lines.get("topic");
  const [pasted, setPasted] = useState("");
  const input = useRef(null);
  const idle = r.phase === "idle";
  const manual = Boolean(r.fields.manual);
  const checked = lineText(r.fields.title || "", compose.title_max);
  const derived = deriveSlug(checked.text, compose.slug_max);
  const slug = manual ? r.fields.slug || "" : derived;
  const taken = desk.groups.some((group) => group.rows.some((row) => row.slug === slug));
  let problem = checked.problem;
  if (!problem && checked.text) {
    if (!slugOk(slug, compose.slug_max)) {
      problem = "The slug is lowercase letters, digits and hyphens, starting with a letter or digit, at most " + compose.slug_max + ".";
    } else if (taken) {
      problem = "#" + slug + " is a topic already.";
    }
  }
  const ready = idle && Boolean(route) && checked.text !== "" && !problem;
  const preview = checked.text && slugOk(slug, compose.slug_max) ? "#" + slug + " new " + checked.text : "";
  const submit = (event) => {
    event.preventDefault();
    if (ready) {
      lines.press("topic", preview, null, slug, "");
    }
  };
  const onPaste = (event) => {
    if (refusesPaste(event)) {
      setPasted(PASTE_REFUSED);
    }
  };
  const asked = lines.asked();
  return html`<form class="new-topic dq" onSubmit=${submit}>
    <div class="dq-row">
      <input
        ref=${input}
        class="dq-input"
        type="text"
        name="topic-title"
        data-field="title"
        aria-label="Title of a new topic"
        placeholder="New topic: title"
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        enterkeyhint="send"
        value=${r.fields.title || ""}
        readOnly=${!idle}
        onInput=${(event) => {
          setPasted("");
          lines.set("topic", "title", event.currentTarget.value);
        }}
        onPaste=${onPaste}
      />
      <button type="submit" class="dq-go" data-action="new-topic" disabled=${!ready}>Ask</button>
    </div>
    ${manual
      ? html`<input
          class="dq-input dq-slug"
          type="text"
          name="topic-slug"
          data-field="slug"
          aria-label="Slug of a new topic"
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          value=${slug}
          readOnly=${!idle}
          onInput=${(event) => {
            setPasted("");
            lines.set("topic", "slug", event.currentTarget.value);
          }}
          onPaste=${onPaste}
        />`
      : null}
    <p class="dq-preview" data-role="preview">${preview}</p>
    ${checked.text && !manual
      ? html`<button
          type="button"
          class="dq-btn"
          data-action="edit-slug"
          disabled=${!idle}
          onClick=${() => lines.setMany("topic", { manual: true, slug: derived })}
        >
          Edit slug
        </button>`
      : null}
    ${idle && problem ? html`<p class="dq-hint" data-role="problem">${problem}</p>` : null}
    <${LineActions} lines=${lines} id="topic" r=${r} pasted=${pasted} said=${null} focus=${input} />
    ${asked.map((name) => html`<p class="dq-asked" data-role="asked" key=${name}>asked for #${name}</p>`)}
  </form>`;
}

// The owner's composer. One draft id (nonce) per draft, and one attempt per
// draft id: the exact body first posted (text, route and id) is posted again
// verbatim on every retry and every later check until an answer settles it,
// so the server carries a draft at most once and a check always finds its
// receipt. While an attempt is open the draft cannot change.
//
// With the topic `slug` selected, a draft is sent as `#<slug>: <text>` and the chip
// shows the tag. Removing it sends this one draft untagged. While an attempt is
// made or open, the chip shows the tag that attempt carries.
function Composer({ route, slug, untagged, setUntagged }) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [attempt, setAttempt] = useState("");
  const [carried, setCarried] = useState("");
  const [note, setNote] = useState(null);
  const box = useRef(null);
  const frozen = sending || Boolean(attempt);
  const tag = untagged ? "" : slug;
  const chip = frozen ? carried : tag;
  const ready = Boolean(attempt || (route && text.trim()));

  // A receipt settles the attempt. A refused draft stays to be edited and
  // sent as a new one; one that was sent or may have landed leaves the
  // composer, so it is not sent twice.
  const settle = (receipt) => {
    const shown = receiptNote(receipt);
    setAttempt("");
    setNote(shown);
    if (shown.kind !== "refused") {
      setText("");
      setUntagged(false);
    }
  };

  // The open attempt, or a new one for the draft as it stands.
  const begin = async () => {
    if (attempt) {
      return attempt;
    }
    if (!route || !text.trim()) {
      return "";
    }
    const made = await draft(route, tag ? "#" + tag + ": " + text : text);
    if (!made.body) {
      setNote(made.note);
      return "";
    }
    setAttempt(made.body);
    return made.body;
  };

  const send = async () => {
    if (sending || !ready) {
      return;
    }
    if (!attempt) {
      setCarried(tag);
    }
    setSending(true);
    setNote(line("wait", attempt ? "asking again" : "sending"));
    try {
      const body = await begin();
      if (!body) {
        return;
      }
      const done = await carry(body, setNote);
      if (done.receipt) {
        settle(done.receipt);
        return;
      }
      // A refusal of this very body settles the attempt with nothing sent, so
      // the draft can be edited again. Any other end leaves it open to be
      // asked again, because it may have carried.
      if (done.spent) {
        setAttempt("");
      }
      setNote(done.note);
    } catch (_) {
      setNote(line("error", UNREACHABLE + "; Check again asks once more"));
    } finally {
      setSending(false);
    }
  };

  const onKeyDown = (event) => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      send();
    }
  };

  return html`<form
    class="composer"
    onSubmit=${(event) => {
      event.preventDefault();
      send();
    }}
  >
    ${note
      ? html`<p class=${"receipt k-" + note.kind} role="status">
          ${note.label ? html`<b>${note.label}</b>${note.words ? " " : ""}` : null}${note.words}
          ${note.id ? html` <span class="mute sm">${note.id}</span>` : null}
        </p>`
      : null}
    <textarea
      ref=${box}
      name="draft"
      rows="3"
      aria-label="Message to the orchestrator"
      placeholder=${route ? "Message " + route.seat + " in " + route.to : NO_ROUTE}
      value=${text}
      readOnly=${frozen}
      onInput=${(event) => setText(event.currentTarget.value)}
      onKeyDown=${onKeyDown}
    ></textarea>
    <div class="bar">
      ${chip
        ? html`<span class="tag-chip" data-role="tag-chip" data-slug=${chip}>
            <span class="tag-name" title=${"#" + chip}>#${chip}</span>
            <button
              type="button"
              data-action="untag"
              aria-label=${"Send without #" + chip}
              disabled=${frozen}
              onMouseDown=${(event) => event.preventDefault()}
              onClick=${() => setUntagged(true)}
            >×</button>
          </span>`
        : null}
      <span class="sp"></span>
      <span class="mute sm">Ctrl+Enter</span>
      <button type="submit" class="send" disabled=${sending || !ready}>
        ${sending ? "Sending" : attempt ? "Check again" : "Send"}
      </button>
    </div>
  </form>`;
}


// The Revisit line: one quiet topic by name with its next step, on the All
// tab. Next cycles through the quiet topics in the order the server sends.
function Revisit({ entry, count, next, open, walk }) {
  return html`<div class="revisit" data-slug=${entry.slug}>
    <div class="hd">
      <span>Revisit</span>
      ${count > 1 ? html`<button class="link rv-next" onClick=${next}>Next →</button>` : null}
    </div>
    <div class="nm">${entry.title || entry.slug}</div>
    ${entry.line ? html`<p class="ln">${entry.line}</p>` : null}
    <p class="meta" title=${entry.reviewed}>reviewed ${when(entry.reviewed)}</p>
    <div class="acts">
      <button class="link rv-open" onClick=${() => open(entry.slug)}>Open topic</button>
      <button class="link rv-walk" onClick=${walk}>Walk the quiet topics (${count})</button>
    </div>
  </div>`;
}

// The same line above the chat on a narrow screen.
function Strip({ entry, count, next, open }) {
  return html`<div class="pq" data-slug=${entry.slug}>
    <button class="link rv-open" onClick=${() => open(entry.slug)}>Revisit · ${entry.title || entry.slug}</button>
    ${count > 1 ? html`<button class="link rv-next" onClick=${next}>Next →</button>` : null}
  </div>`;
}

// The page owns the rules of a closing reason; the server serves only its
// longest length. Once trimmed it is one line of 1 to `max` Unicode scalar
// values with no control character and no line break, so a verdict is never
// two chat lines. Every input is checked.
const REASON_BAD = /[\p{Cc}\p{Zl}\p{Zp}]/u;

function reasonProblem(text, max) {
  // The characters are checked before the trim, so a line break or control
  // character at either end is refused as it is anywhere else.
  if (REASON_BAD.test(text)) {
    return "One line only: no line break or control character.";
  }
  const trimmed = text.trim();
  if (!trimmed) {
    return "Write one line saying why it ends.";
  }
  const length = Array.from(trimmed).length;
  return length > max ? "At most " + max + " characters; this has " + length + "." : "";
}

// One read of a topic document: { slug, card, tag } for a fresh one, { slug,
// same } when `tag` still names the current one, { slug, gone } when the topic
// is no longer known and { slug, failed } with the reason when it could not be
// read. Never throws.
async function readTopic(slug, tag) {
  try {
    const headers = tag ? { "If-None-Match": tag } : {};
    const answer = await api("topic/" + slug, { headers });
    if (answer.status === 200) {
      return { slug, card: await answer.json(), tag: answer.headers.get("ETag") || "" };
    }
    if (answer.status === 304) {
      return { slug, same: true };
    }
    if (answer.status === 404) {
      return { slug, gone: true };
    }
    return { slug, failed: "ae web answered " + answer.status };
  } catch (_) {
    return { slug, failed: UNREACHABLE };
  }
}

const DONE = { keep: "kept", ask: "asked for a next step", close: "closed" };

// The walk through the quiet topics, and with one entry the dialog that closes
// one topic from its card. A verdict is one owner chat line carried like a
// composer draft: the topic document supplies the line, the orchestrator turns
// it into a memo record, and the page never keeps a store of its own.
// A server receipt settles a step: sent and unknown are both final (the server
// never carries that draft again), a refusal leaves the step to be tried again
// as a new draft. Only a carry without any receipt keeps the dialog from being
// dismissed; reloading the page is the one other way out.
function Walk({ walk, route, state, stop }) {
  const entries = walk.entries;
  const single = walk.single;
  const [at, setAt] = useState(0);
  const [log, setLog] = useState([]);
  const [phase, setPhase] = useState("idle");
  const [note, setNote] = useState(null);
  const [refused, setRefused] = useState(false);
  const [closing, setClosing] = useState(single);
  const [reason, setReason] = useState("");
  const [pasted, setPasted] = useState("");
  const [doc, setDoc] = useState(null);
  const attempt = useRef(null);
  const last = useRef("keep");
  const held = useRef({ slug: "", value: "" });
  const keepBox = useRef(null);
  const reasonBox = useRef(null);
  const idleNow = useRef(true);
  const entry = entries[at] || null;
  const slug = entry ? entry.slug : "";
  const idle = phase === "idle";
  idleNow.current = idle;
  // A walk sends only to the orchestrator it was opened against. When the
  // route changed, nothing new is sent; an open attempt still checks its own
  // receipt, because its body names the route it was written for.
  const sendTo = walk.route;
  const stale = !sendTo || !route || route.id !== sendTo.id;

  // The topic document of this step, read again on every new snapshot.
  useEffect(() => {
    if (!slug) {
      return undefined;
    }
    let gone = false;
    const again = async () => {
      const read = await readTopic(slug, held.current.slug === slug ? held.current.value : "");
      if (gone || read.same) {
        return;
      }
      if (read.card) {
        held.current = { slug, value: read.tag };
      }
      // A failed read keeps the document already shown.
      setDoc((before) => (read.failed && before && before.slug === slug && before.card ? before : read));
    };
    again();
    return () => {
      gone = true;
    };
  }, [slug, state]);

  const loaded = Boolean(doc && doc.slug === slug);
  const card = loaded && doc.card ? doc.card : null;
  const verdict = card ? card.verdict : null;
  const eligible = Boolean(verdict) && (single || card.group === "quiet");
  const live = eligible && !stale && idle;
  const max = verdict ? verdict.reason_max : 200;
  const problem = reasonProblem(reason, max);
  const name = (card && card.fields.title && card.fields.title.value) || (entry && entry.title) || slug;

  // The dialog takes the focus when it opens, so that the page behind it
  // cannot act on a key, and gives the focus back to what opened it.
  useEffect(() => {
    const opener = document.activeElement;
    const box = document.querySelector(".walk");
    if (box) {
      box.focus();
    }
    return () => {
      if (opener && opener.isConnected && typeof opener.focus === "function") {
        opener.focus();
      }
    };
  }, []);

  // Focus goes to the first action once a step can take one.
  useEffect(() => {
    const target = closing ? reasonBox.current : keepBox.current;
    if (live && target) {
      target.focus();
    }
  }, [slug, live, closing]);

  // Escape stops the walk, and Tab stays inside it, while no carry is open.
  useEffect(() => {
    const onKey = (event) => {
      if (event.key === "Escape") {
        if (idleNow.current) {
          event.preventDefault();
          stop();
        }
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const box = document.querySelector(".walk");
      if (!box) {
        return;
      }
      const items = Array.from(box.querySelectorAll("button:not(:disabled), input:not(:disabled)"));
      const at = items.indexOf(document.activeElement);
      if (!items.length) {
        event.preventDefault();
        box.focus();
      } else if (at < 0) {
        // Focus on the dialog box or outside the enabled items enters the cycle.
        event.preventDefault();
        items[event.shiftKey ? items.length - 1 : 0].focus();
      } else if (!event.shiftKey && at === items.length - 1) {
        event.preventDefault();
        items[0].focus();
      } else if (event.shiftKey && at === 0) {
        event.preventDefault();
        items[items.length - 1].focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [stop]);

  const next = (text) => {
    setLog((before) => before.concat(text));
    setNote(null);
    setRefused(false);
    setReason("");
    setPasted("");
    setClosing(single);
    setPhase("idle");
    setAt((now) => now + 1);
  };

  // A receipt settles the step.
  const finish = (sent, receipt) => {
    const shown = receiptNote(receipt);
    attempt.current = null;
    if (shown.kind === "refused") {
      setNote(shown);
      setRefused(true);
      setPhase("idle");
      return;
    }
    if (single && shown.kind === "sent") {
      // The dialog goes, and its card keeps what happened.
      stop({ slug, text: "Closing line sent for #" + slug + ": " + shown.label + "." });
      return;
    }
    const words =
      shown.kind === "sent"
        ? sent.name + ": " + DONE[sent.kind] + (sent.kind === "close" ? ": " + sent.reason : "") + ", " + shown.label
        : sent.name + ": " + shown.words;
    next(words);
  };

  // The open attempt carried, or carried again with the same body and id.
  const carryOpen = async () => {
    const sent = attempt.current;
    setPhase("pending");
    setNote(line("wait", "sending"));
    const done = await carry(sent.body, setNote);
    if (done.receipt) {
      finish(sent, done.receipt);
      return;
    }
    if (done.spent) {
      attempt.current = null;
    }
    setNote(done.note);
    setRefused(Boolean(done.spent));
    setPhase(done.spent ? "idle" : "noanswer");
  };

  // Check the topic once more, then send the verdict as a new draft.
  const act = async (kind) => {
    if (!live || (kind === "close" && problem)) {
      return;
    }
    last.current = kind;
    setPhase("pending");
    setRefused(false);
    setNote(line("wait", "checking the topic"));
    const fresh = await readTopic(slug, "");
    if (!fresh.failed) {
      setDoc(fresh);
    }
    const lines = fresh.card ? fresh.card.verdict : null;
    if (!lines || (!single && fresh.card.group !== "quiet")) {
      setNote(fresh.failed ? line("error", fresh.failed) : null);
      setPhase("idle");
      return;
    }
    const text = kind === "close" ? lines.close + reason.trim() : lines[kind];
    let made;
    try {
      made = await draft(sendTo, text);
    } catch (_) {
      made = { note: line("error", UNREACHABLE) };
    }
    if (!made.body) {
      setNote(made.note);
      setPhase("idle");
      return;
    }
    attempt.current = { body: made.body, kind, name, reason: reason.trim() };
    await carryOpen();
  };

  const skip = () => {
    if (idle) {
      next(name + ": skipped, " + (card ? "no longer quiet" : "not readable"));
    }
  };

  const onReason = (event) => {
    setReason(event.currentTarget.value);
    setPasted("");
  };
  const onPaste = (event) => {
    if (refusesPaste(event)) {
      setPasted(PASTE_REFUSED);
    }
  };
  const onReasonKey = (event) => {
    // Confirming an input-method candidate with Enter is no send.
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      if (!problem) {
        act("close");
      }
    }
  };
  const dismiss = (event) => {
    if (event.target === event.currentTarget && idle) {
      stop();
    }
  };

  const doneList = html`<ul class="w-done">${log.map((text, each) => html`<li key=${each}>${text}</li>`)}</ul>`;
  let body;
  if (!entry) {
    body = html`<h2>${single ? "Closing line" : log.length ? "Walk done" : "No quiet topic is left to review"}</h2>
      ${log.length ? doneList : null}
      <p class="mute sm">Each line sent shows under Owed to you until the orchestrator answers.</p>
      <div class="w-acts"><button class="btn pri w-end" onClick=${stop}>Back to the chat</button></div>`;
  } else {
    const why = !loaded
      ? null
      : doc.gone
        ? (single ? "#" + slug + " is gone." : "No longer quiet: #" + slug + " is gone.")
        : doc.failed
          ? "Cannot read #" + slug + ": " + doc.failed
          : !eligible
            ? (single ? "This topic cannot be closed now: it is " : "No longer quiet: it is now ") +
              (GROUPS[card.group] || card.group) + "."
            : null;
    body = html`${single
        ? null
        : html`<div class="w-step mute sm">Quiet topic ${at + 1} of ${entries.length}</div>
            <div class="steps">${entries.map((_, each) => html`<i key=${each} class=${each <= at ? "on" : ""}></i>`)}</div>`}
      ${!loaded ? html`<p class="w-wait mute">Reading #${slug}.</p>` : html`<h2 class="nm">${name}</h2>`}
      ${card
        ? html`<p class="sub mute sm">#${slug} · ${GROUPS[card.group] || card.group} · reviewed ${when(card.reviewed)}</p>
            ${[["what", "What"], ["last", "Last"], ["next", "Next"]]
              .filter(([key]) => card.fields[key])
              .map(([key, label]) => html`<div class="fld" key=${key}><h3>${label}</h3><p class="raw">${card.fields[key].value}</p></div>`)}`
        : null}
      ${why ? html`<p class="w-gone">${why}</p>` : null}
      ${stale && loaded
        ? html`<p class="w-hint mute sm">${route && sendTo ? "The orchestrator changed since this opened: stop, then open it again." : "No proven orchestrator to send to."}</p>`
        : null}
      ${note ? html`<p class="w-note" role="status">${note.words}</p>` : null}
      ${refused && note && note.words.startsWith("cap-full")
        ? html`<p class="w-hint mute sm">The walk waits here: answer or withdraw one open ask, then press Try again.</p>`
        : null}
      <div class="w-acts">
        ${eligible && !closing
          ? html`<button class="btn" data-walk="keep" ref=${keepBox} disabled=${!live} onClick=${() => act("keep")}>
                Keep<small>Still wanted. It stays, and the review time moves to now.</small>
              </button>
              <button class="btn" data-walk="ask" disabled=${!live} onClick=${() => act("ask")}>
                Ask for a next step<small>Sends one chat line to the orchestrator.</small>
              </button>
              <button class="btn" data-walk="close" disabled=${!live} onClick=${() => setClosing(true)}>
                Close with one line<small>It leaves the list. The line stays under Closed.</small>
              </button>`
          : null}
        ${eligible && closing
          ? html`<label class="w-field">Closing line
                <input
                  class="reason"
                  type="text"
                  name="reason"
                  ref=${reasonBox}
                  placeholder="Why it ends, in one line"
                  value=${reason}
                  disabled=${!idle}
                  onInput=${onReason}
                  onPaste=${onPaste}
                  onKeyDown=${onReasonKey}
                />
              </label>
              <p class="w-hint mute sm" role="status">${pasted || problem}</p>
              <button class="btn pri w-closeok" disabled=${!live || Boolean(problem)} onClick=${() => act("close")}>Close the topic</button>
              <button class="link w-undo" disabled=${!idle} onClick=${single ? stop : () => setClosing(false)}>${single ? "Cancel" : "Back"}</button>`
          : null}
        ${refused ? html`<button class="btn w-retry" disabled=${!live || (last.current === "close" && Boolean(problem))} onClick=${() => act(last.current)}>Try again</button>` : null}
        ${phase === "noanswer" ? html`<button class="btn w-check" onClick=${carryOpen}>Check again</button>` : null}
        ${loaded && !eligible && !single ? html`<button class="link w-skip" disabled=${!idle} onClick=${skip}>Skip</button>` : null}
        <button class="link w-stop" disabled=${!idle} onClick=${stop}>Stop</button>
      </div>
      ${log.length ? doneList : null}`;
  }
  return html`<div class="walk-back" onClick=${dismiss}>
    <div
      class="walk"
      role="dialog"
      aria-modal="true"
      aria-label=${single ? "Close a topic" : "Walk the quiet topics"}
      data-carry=${phase}
      tabIndex="-1"
    >
      ${body}
    </div>
  </div>`;
}

function App() {
  const [state, setState] = useState(null);
  const lines = useLines(state);
  const [problem, setProblem] = useState("");
  const tag = useRef("");
  // The selected topic, "" for All; `shut`, its slug while it may stay selected
  // although closed with no row of its own ("" else; it lives in this page
  // only); whether its draft goes untagged; the selected topic's card, that
  // answer's tag; and what a narrow screen shows, the chat or the panel.
  const [chosen, setChosen] = useState("");
  const [shut, setShut] = useState("");
  const [pins, writePin] = usePins();
  const [untagged, setUntagged] = useState(false);
  const [card, setCard] = useState(null);
  const [view, setView] = useState("chat");
  const [cardProblem, setCardProblem] = useState("");
  const cardTag = useRef("");
  const desk = state && state.desk ? state.desk : null;
  const chosenRow = chosen ? deskRow(desk, chosen) : null;
  // A selected topic that lost its place shows All at once.
  const active = chosenRow && selectable(chosenRow, pins, shut) ? chosen : "";
  const lists = desk ? sideGroups(desk, pins, state.chat.rows) : [];
  const book = useRef(null);
  book.current = book.current || newBook();
  if (state && book.current.state !== state) {
    admit(book.current, state);
  }
  // The selected topic has shown every turn there is.
  if (active) {
    book.current.opened.set(active, book.current.round);
  }
  const unread = state ? unreadCounts(book.current, state.chat.rows) : new Map();
  // The Revisit line's shown topic and the walk or close dialog.
  const [rv, setRv] = useState("");
  const [walk, setWalk] = useState(null);
  const quiet = (state && state.desk && state.desk.revisit) || [];
  // A shown topic that left the quiet list falls back to the first of it.
  const current = quiet.find((entry) => entry.slug === rv) || quiet[0] || null;
  // The fallback is committed as the selection only when the selection left
  // the list, so a topic that left and comes back does not take it back. The
  // updater reads the selection as it stands then: a Next press made before
  // this effect ran is kept, never overwritten with the value of an older render.
  const shownSlug = current ? current.slug : "";
  useEffect(() => {
    setRv((before) => (quiet.some((entry) => entry.slug === before) ? before : shownSlug));
  }, [shownSlug]);
  const nextRevisit = () => {
    if (current) {
      setRv(current.next);
    }
  };
  // What a closed dialog leaves on its card: the sent line of a single close.
  const [closed, setClosed] = useState(null);
  const stopWalk = useCallback((said) => {
    setWalk(null);
    if (said && typeof said.slug === "string" && typeof said.text === "string") {
      setClosed(said);
    }
  }, []);
  const startWalk = () => {
    if (quiet.length) {
      setWalk({
        entries: quiet.map((entry) => ({ slug: entry.slug, title: entry.title })),
        single: false,
        route: state.route,
      });
    }
  };
  const startClose = () => {
    if (card) {
      setClosed(null);
      const title = card.fields.title ? card.fields.title.value : null;
      setWalk({ entries: [{ slug: card.slug, title }], single: true, route: state ? state.route : null });
    }
  };
  const revisitProps = { entry: current, count: quiet.length, next: nextRevisit, walk: startWalk };

  const leave = () => {
    cardTag.current = "";
    setCard(null);
    setChosen("");
    setShut("");
    setUntagged(false);
  };
  // Selects a topic, "" for All, and shows the chat or the panel on a narrow
  // screen. A topic with no row of its own has one while it is selected.
  // Choosing the selected topic again keeps its card and its draft's tag: no new
  // snapshot would bring the card back.
  const open = (slug, shows) => {
    setCardProblem("");
    if (slug !== chosen) {
      cardTag.current = "";
      setCard(null);
      setChosen(slug);
      setUntagged(false);
    }
    const row = deskRow(desk, slug);
    setShut(row && row.state === "closed" && !keeps(row, pins) ? slug : "");
    setView(shows);
  };
  const selectTab = (slug) => open(slug, "chat");
  const openTopic = (slug) => open(slug, "panel");

  // Unpinning the selected topic leaves it open until another one is chosen, also
  // when it is closed and nothing else keeps it.
  const pinTab = (slug, pin) => {
    const next = writePin(slug, pin);
    const row = deskRow(desk, slug);
    if (next.includes(slug)) {
      setShut((before) => (before === slug ? "" : before));
    } else if (slug === active && row && row.state === "closed" && !keepsUnpinned(row)) {
      setShut(slug);
    }
  };

  // A selected topic that lost its place is dropped for good, so it does not open
  // again by itself when it gets one back. Once the board or a pin keeps `shut`,
  // or it is no longer closed, it is forgotten, so a later close drops it like
  // any other topic.
  useEffect(() => {
    if (!desk) {
      return;
    }
    if (chosen && !active) {
      leave();
      return;
    }
    const row = shut ? deskRow(desk, shut) : null;
    if (row && (keeps(row, pins) || row.state !== "closed")) {
      setShut("");
    }
  }, [state, pins]);

  // The open card follows every new snapshot: it is read from the same one.
  useEffect(() => {
    if (!active) {
      return undefined;
    }
    let gone = false;
    // A failed read shows All and says why.
    const fail = (why) => {
      leave();
      setCardProblem("#" + active + ": " + why);
    };
    const read = async () => {
      try {
        const headers = cardTag.current ? { "If-None-Match": cardTag.current } : {};
        const answer = await api("topic/" + active, { headers });
        if (gone) {
          return;
        }
        if (answer.status === 200) {
          const next = await answer.json();
          if (gone) {
            return;
          }
          cardTag.current = answer.headers.get("ETag") || "";
          setCard(next);
        } else if (answer.status === 404) {
          fail("no such topic");
        } else if (answer.status !== 304) {
          fail("ae web answered " + answer.status);
        }
      } catch (_) {
        if (!gone) {
          fail("ae web is not reachable");
        }
      }
    };
    read();
    return () => {
      gone = true;
    };
  }, [active, state]);

  useEffect(() => {
    let stopped = false;
    let timer = 0;
    // One poll at a time and one timer: a wake during a fetch asks for one
    // more poll right after it, so an older answer never lands last.
    let busy = false;
    let again = false;
    // No wake polls before the startup exchange settled: an anonymous 401
    // landing after it would clear the fresh session.
    let ready = false;
    // Signed out: a 401 said the session is gone, and the refusal that said it
    // already printed the owner a link. Nothing asks again, on the timer or on
    // a wake, until a code is redeemed in this tab; asking a listener that
    // refuses every request only prints it more links.
    let halted = false;
    const later = (ms) => {
      clearTimeout(timer);
      timer = setTimeout(poll, ms);
    };
    const poll = async () => {
      if (stopped || halted) {
        return;
      }
      if (busy) {
        again = true;
        return;
      }
      busy = true;
      clearTimeout(timer);
      // The secret this request carries: an answer for a secret that is no
      // longer the stored one (a code was redeemed meanwhile) says nothing
      // about the new session.
      const used = stored();
      try {
        const headers = tag.current ? { "If-None-Match": tag.current } : {};
        const answer = await api("state", { headers });
        if (answer.status === 401) {
          if (stored() === used) {
            keep("");
            tag.current = "";
            setProblem(NO_SESSION);
            halted = true;
          } else {
            again = true;
          }
        } else if (answer.status === 200) {
          const next = await answer.json();
          tag.current = answer.headers.get("ETag") || "";
          setState(next);
          setProblem("");
        } else if (answer.status === 304) {
          setProblem("");
        } else {
          setProblem("ae web answered " + answer.status);
        }
      } catch (_) {
        setProblem("ae web is not reachable");
      }
      busy = false;
      if (stopped) {
        return;
      }
      if (halted) {
        again = false;
      } else if (again) {
        again = false;
        later(0);
      } else {
        later(document.hidden ? HIDDEN_MS : VISIBLE_MS);
      }
    };
    const start = async () => {
      try {
        const code = takeCode();
        if (code && !(await redeem(code))) {
          // The listener refused the code and printed the owner a new link.
          setProblem(NO_SESSION);
          halted = true;
        }
      } catch (_) {
        setProblem("ae web is not reachable");
      }
      if (!stored()) {
        setProblem(NO_SESSION);
      }
      ready = true;
      poll();
    };
    // A link opened in this tab after it loaded changes only the fragment, so
    // the page is not reloaded: take the code from it here.
    const open = async () => {
      if (!ready) {
        return;
      }
      const code = takeCode();
      if (!code) {
        return;
      }
      try {
        const redeemed = await redeem(code);
        if (stopped) {
          return;
        }
        if (redeemed) {
          halted = false;
          setProblem("");
          poll();
        } else if (halted) {
          setProblem(NO_SESSION);
        }
      } catch (_) {
        if (!stopped) {
          setProblem("ae web is not reachable");
        }
      }
    };
    const wake = () => {
      if (ready && !document.hidden) {
        poll();
      }
    };
    document.addEventListener("visibilitychange", wake);
    window.addEventListener("hashchange", open);
    start();
    return () => {
      stopped = true;
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", wake);
      window.removeEventListener("hashchange", open);
    };
  }, []);

  const shown = card && card.slug === active ? card : null;
  let panel = html`<aside class="panel" aria-label="Topics"></aside>`;
  if (desk && !active) {
    panel = html`<${AllPanel} desk=${desk} lists=${lists} open=${openTopic} revisit=${revisitProps} lines=${lines} route=${state.route} />`;
  } else if (shown) {
    panel = html`<${Card}
      key=${shown.slug}
      card=${shown}
      closeLine=${startClose}
      outcome=${closed && closed.slug === shown.slug ? closed.text : ""}
      lines=${lines}
      route=${state ? state.route : null}
      decide=${desk ? desk.decide : []}
      compose=${(desk && desk.compose) || COMPOSE}
      pinned=${pins.includes(shown.slug)}
      full=${!pins.includes(shown.slug) && pins.length >= PINS_MAX}
      pin=${pinTab}
    />`;
  } else if (active) {
    panel = html`<aside class="panel" aria-label="Topic"><p class="note">Reading #${active}.</p></aside>`;
  }
  return html`<div class=${"app view-" + view}>
    <${Head} state=${state} />
    ${problem ? html`<p class="problem" role="status">${problem}</p>` : null}
    ${cardProblem ? html`<p class="problem" role="status">${cardProblem}</p>` : null}
    <main>
      <${SidePanel} lists=${lists} unread=${unread} active=${active} select=${selectTab} />
      <section class="chat" aria-label="Chat">
        ${current && !active && !walk
          ? html`<${Strip} entry=${current} count=${quiet.length} next=${nextRevisit} open=${openTopic} />`
          : null}
        ${state ? html`<${Stream} key=${"s" + active} chat=${state.chat} tag=${active} />` : html`<div class="stream"></div>`}
        <${Composer}
          route=${state ? state.route : null}
          slug=${active}
          untagged=${untagged}
          setUntagged=${setUntagged}
        />
      </section>
      ${panel}
    </main>
    <nav class="views" aria-label="Views">
      <button
        type="button"
        data-role="view-topics"
        class=${view === "topics" ? "on" : ""}
        onClick=${() => setView("topics")}
      >
        Topics
      </button>
      <button type="button" data-role="view-chat" class=${view === "chat" ? "on" : ""} onClick=${() => setView("chat")}>
        Chat
      </button>
      <button type="button" data-role="view-panel" class=${view === "panel" ? "on" : ""} onClick=${() => setView("panel")}>
        ${active ? "Topic" : "Overview"}
      </button>
    </nav>
    ${walk
      ? html`<${Walk} walk=${walk} route=${state ? state.route : null} state=${state} stop=${stopWalk} />`
      : null}
  </div>`;
}

render(html`<${App} />`, document.getElementById("root"));
