// ae web: the orchestrator chat in a browser tab. Every server string is
// rendered as a Preact text child, never as markup. Network calls go through
// the one wrapper below, to this origin's /api/ routes.
import { h, render } from "./preact.mjs";
import { useState, useEffect, useRef, useCallback } from "./hooks.mjs";
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
  return html`<article class=${"msg " + row.voice + " k-" + row.kind}>
    <div class="who">
      <b>${row.who}</b>
      ${row.what ? html`<span>${row.what}</span>` : null}
      <time>${clock(row.micros)}</time>
    </div>
    ${row.body ? html`<div class="bub">${row.body}</div>` : null}
  </article>`;
}

function Stream({ chat }) {
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
  const items = [];
  let shown = "";
  for (const row of chat.rows) {
    const date = day(row.micros);
    if (date !== shown) {
      shown = date;
      items.push(html`<div class="day" key=${"d" + date}>${date}</div>`);
    }
    items.push(html`<${Row} key=${row.key} row=${row} />`);
  }
  return html`<div class="stream" onScroll=${onScroll}>
    <div class="inner">
      ${chat.cut > 0 ? html`<p class="gap">${chat.cut} older turns are not shown.</p>` : null}
      ${chat.coverage.map((gap) => html`<p class="gap" key=${"c" + gap}>coverage incomplete: ${gap}</p>`)}
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

function TopicRow({ row, group, chosen, choose }) {
  const waits = row.waits.length;
  // A press keeps the focus where it was, so a draft being typed stays focused.
  return html`<button
    class=${"row" + (chosen === row.slug ? " sel" : "")}
    onMouseDown=${(event) => event.preventDefault()}
    onClick=${() => choose(row.slug)}
  >
    <span class=${"dot " + group}></span>
    <span class="nm">${row.title || row.slug}</span>
    <span class="meta" title=${row.reviewed}>${when(row.reviewed)}</span>
    <span class="ln">#${row.slug}${waits ? " · waits on you (" + waits + ")" : ""}</span>
  </button>`;
}

function Desk({ desk, chosen, choose, revisit }) {
  const [query, setQuery] = useState("");
  const rows = (name) => {
    const found = desk.groups.find((group) => group.group === name);
    return found ? found.rows : [];
  };
  const listed = (name) =>
    rows(name).map(
      (row) => html`<${TopicRow} key=${name + row.slug} row=${row} group=${name} chosen=${chosen} choose=${choose} />`,
    );
  const needle = query.trim().toLowerCase();
  const all = [];
  for (const group of desk.groups) {
    for (const row of group.rows) {
      const words = (row.slug + " " + (row.title || "")).toLowerCase();
      if (!needle || words.includes(needle)) {
        all.push(html`<${TopicRow} key=${"a" + row.slug} row=${row} group=${group.group} chosen=${chosen} choose=${choose} />`);
      }
    }
  }
  const skipped = desk.skipped;
  const left = skipped.claimed + skipped.unproven + skipped.invalid;
  return html`<nav class="col" aria-label="Topics">
    <div class="grp need"><span>${GROUPS.needs_you}</span><span>${rows("needs_you").length}</span></div>
    ${listed("needs_you")}
    ${desk.untagged.length
      ? html`<div class="grp need"><span>Untagged decisions</span><span>${desk.untagged.length}</span></div>
          ${desk.untagged.map((wait, at) => html`<p class="wait" key=${"u" + at}>${wait.text}</p>`)}`
      : null}
    <div class="grp"><span>${GROUPS.moving}</span><span>${rows("moving").length}</span></div>
    ${listed("moving")}
    ${revisit.entry
      ? html`<div class="grp"><span>${GROUPS.quiet}</span><span>${revisit.count}</span></div>
          <${Revisit} entry=${revisit.entry} count=${revisit.count} next=${revisit.next} open=${choose} walk=${revisit.walk} />`
      : null}
    <div class="grp"><span>All topics</span><span>${all.length}</span></div>
    <input
      type="search"
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
  </nav>`;
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

// A record's source: its session, its incarnation's uuid and its time.
function Source({ origin }) {
  return html`<span class="src" title=${origin.at}>
    ${origin.session} · ${origin.uuid} · ${when(origin.at)}
  </span>`;
}

function Card({ card, close, closeLine, outcome }) {
  const fields = card.fields;
  // Only Quiet, Moving and Parked cards can be closed from here; a card with a
  // wait shows the button disabled and says why, a closed or unreadable one
  // shows none.
  const closable = Boolean(card.verdict);
  const blocked = card.group === "needs_you";
  return html`<aside class="card" aria-label="Topic">
    <div class="hd">
      <h2>${fields.title ? fields.title.value : card.slug}</h2>
      <button class="x" aria-label="Close topic" onClick=${close}>×</button>
    </div>
    <p class="sub">#${card.slug} · ${GROUPS[card.group] || card.group} · reviewed ${when(card.reviewed)}</p>
    ${WAITS.map(([kind, name]) => {
      const waits = card.waits.filter((wait) => wait.kind === kind);
      return waits.length
        ? html`<div class="fld need" key=${kind}>
            <h3>${name}</h3>
            <ul>${waits.map((wait, at) => html`<li key=${kind + at}>${wait.text}${wait.id ? html` <span class="mute sm">${wait.id}</span>` : null}</li>`)}</ul>
          </div>`
        : null;
    })}
    ${card.unreadable !== null
      ? html`<div class="fld need"><h3>Unreadable checkpoint</h3><p class="raw">${card.unreadable}</p></div>`
      : null}
    ${FIELDS.filter(([key]) => fields[key]).map(
      ([key, name]) => html`<div class="fld" key=${key}>
        <h3><span>${name}</span><${Source} origin=${fields[key]} /></h3>
        <p class="raw">${fields[key].value}</p>
      </div>`,
    )}
    ${card.history.length
      ? html`<div class="fld">
          <h3>Earlier records</h3>
          <ul>${card.history.map((entry, at) => html`<li key=${"h" + at}><${Source} origin=${entry} /> <span class="raw">${entry.text}</span></li>`)}</ul>
        </div>`
      : null}
    ${closable || blocked
      ? html`<div class="cact">
          <button class="btn cl-line" disabled=${!closable} onClick=${closeLine}>Close with one line</button>
        </div>`
      : null}
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
async function draft(route, text) {
  const answer = await api("nonce");
  if (answer.status !== 200) {
    return { note: line("error", answer.status === 401 ? NO_SESSION : await refusal(answer)) };
  }
  const minted = await answer.json();
  const nonce = minted && typeof minted.nonce === "string" ? minted.nonce : "";
  if (!nonce) {
    return { note: line("error", "ae web gave no draft id") };
  }
  return { body: JSON.stringify({ text, route: route.id, nonce }) };
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

// The owner's composer. One draft id (nonce) per draft, and one attempt per
// draft id: the exact body first posted (text, route and id) is posted again
// verbatim on every retry and every later check until an answer settles it,
// so the server carries a draft at most once and a check always finds its
// receipt. While an attempt is open the draft cannot change.
function Composer({ route, slug, onAttached }) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [attempt, setAttempt] = useState("");
  const [note, setNote] = useState(null);
  const box = useRef(null);
  const frozen = sending || Boolean(attempt);
  // A draft whose first line is a bare #slug is the one Attach wrote.
  const attached = /^#[a-z0-9][a-z0-9-]*$/.test(text.split("\n", 1)[0]);
  useEffect(() => {
    if (onAttached) {
      onAttached(attached);
    }
  }, [attached, onAttached]);
  const ready = Boolean(attempt || (route && text.trim()));

  const attach = () => {
    if (frozen) {
      return;
    }
    const first = "#" + slug;
    setText((now) => (now.split("\n", 1)[0] === first ? now : first + "\n" + now));
    if (box.current) {
      box.current.focus();
    }
  };

  // A receipt settles the attempt. A refused draft stays to be edited and
  // sent as a new one; one that was sent or may have landed leaves the
  // composer, so it is not sent twice.
  const settle = (receipt) => {
    const shown = receiptNote(receipt);
    setAttempt("");
    setNote(shown);
    if (shown.kind !== "refused") {
      setText("");
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
    const made = await draft(route, text);
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
      rows="3"
      aria-label="Message to the orchestrator"
      placeholder=${route ? "Message " + route.seat + " in " + route.to : "No proven orchestrator to send to"}
      value=${text}
      readOnly=${frozen}
      onInput=${(event) => setText(event.currentTarget.value)}
      onKeyDown=${onKeyDown}
    ></textarea>
    <div class="bar">
      ${slug
        ? html`<button
            type="button"
            class="attach"
            disabled=${frozen}
            onMouseDown=${(event) => event.preventDefault()}
            onClick=${attach}
          >
            Attach #${slug}
          </button>`
        : null}
      <span class="sp"></span>
      <span class="mute sm">Ctrl+Enter</span>
      <button type="submit" class="send" disabled=${sending || !ready}>
        ${sending ? "Sending" : attempt ? "Check again" : "Send"}
      </button>
    </div>
  </form>`;
}


// The Revisit line: one quiet topic by name with its next step, in the desk
// column. Next cycles through the quiet topics in the order the server sends.
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
    const text = event.clipboardData ? event.clipboardData.getData("text") : "";
    if (REASON_BAD.test(text)) {
      event.preventDefault();
      setPasted("Paste cancelled: the text holds a line break or control character.");
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
  const [problem, setProblem] = useState("");
  const tag = useRef("");
  // The topic open in the card, its last answer and that answer's tag; the
  // view names what a narrow screen shows.
  const [chosen, setChosen] = useState("");
  const [card, setCard] = useState(null);
  const [view, setView] = useState("chat");
  const [cardProblem, setCardProblem] = useState("");
  const cardTag = useRef("");
  // The Revisit line's shown topic, the walk or close dialog and whether the
  // draft holds an attached topic (the strip hides then).
  const [rv, setRv] = useState("");
  const [walk, setWalk] = useState(null);
  const [attached, setAttached] = useState(false);
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

  // Choosing the open topic again only shows its card: its answer and tag
  // stay, because no new snapshot would bring them back. A card that could
  // not be read leaves no selection, so choosing it again reads it again.
  const choose = useCallback((slug) => {
    setCardProblem("");
    if (slug !== chosen) {
      cardTag.current = "";
      setCard(null);
      setChosen(slug);
    }
    setView("topic");
  }, [chosen]);
  const close = useCallback(() => {
    cardTag.current = "";
    setCard(null);
    setChosen("");
    setCardProblem("");
    setView("topics");
  }, []);

  // The open card follows every new snapshot: it is read from the same one.
  useEffect(() => {
    if (!chosen) {
      return undefined;
    }
    let gone = false;
    // A failed read drops the selection and says why.
    const fail = (why) => {
      cardTag.current = "";
      setCard(null);
      setChosen("");
      setCardProblem("#" + chosen + ": " + why);
    };
    const read = async () => {
      try {
        const headers = cardTag.current ? { "If-None-Match": cardTag.current } : {};
        const answer = await api("topic/" + chosen, { headers });
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
  }, [chosen, state]);

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
    const later = (ms) => {
      clearTimeout(timer);
      timer = setTimeout(poll, ms);
    };
    const poll = async () => {
      if (stopped) {
        return;
      }
      if (busy) {
        again = true;
        return;
      }
      busy = true;
      clearTimeout(timer);
      try {
        const headers = tag.current ? { "If-None-Match": tag.current } : {};
        const answer = await api("state", { headers });
        if (answer.status === 401) {
          keep("");
          tag.current = "";
          setProblem(NO_SESSION);
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
      if (again) {
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
          setProblem(NO_SESSION);
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
    const wake = () => {
      if (ready && !document.hidden) {
        poll();
      }
    };
    document.addEventListener("visibilitychange", wake);
    start();
    return () => {
      stopped = true;
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", wake);
    };
  }, []);

  // A narrow screen shows the list until the chosen card has arrived.
  const shown = view === "topic" && !card ? "topics" : view;
  return html`<div class=${"app view-" + shown + (card ? " has-card" : "")}>
    <${Head} state=${state} />
    ${problem ? html`<p class="problem" role="status">${problem}</p>` : null}
    ${cardProblem ? html`<p class="problem" role="status">${cardProblem}</p>` : null}
    <main>
      ${state && state.desk
        ? html`<${Desk} desk=${state.desk} chosen=${chosen} choose=${choose} revisit=${revisitProps} />`
        : html`<nav class="col" aria-label="Topics"></nav>`}
      <section class="chat" aria-label="Chat">
        ${current && !attached && !walk
          ? html`<${Strip} entry=${current} count=${quiet.length} next=${nextRevisit} open=${choose} />`
          : null}
        ${state ? html`<${Stream} chat=${state.chat} />` : html`<div class="stream"></div>`}
        <${Composer} route=${state ? state.route : null} slug=${card ? card.slug : ""} onAttached=${setAttached} />
      </section>
      ${card
        ? html`<${Card}
            card=${card}
            close=${close}
            closeLine=${startClose}
            outcome=${closed && closed.slug === card.slug ? closed.text : ""}
          />`
        : null}
    </main>
    <nav class="tabs" aria-label="Views">
      <button class=${view === "chat" ? "on" : ""} onClick=${() => setView("chat")}>Chat</button>
      <button class=${view === "chat" ? "" : "on"} onClick=${() => setView("topics")}>Topics</button>
    </nav>
    ${walk
      ? html`<${Walk} walk=${walk} route=${state ? state.route : null} state=${state} stop=${stopWalk} />`
      : null}
  </div>`;
}

render(html`<${App} />`, document.getElementById("root"));
