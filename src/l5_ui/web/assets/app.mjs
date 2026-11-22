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

function Desk({ desk, chosen, choose }) {
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

function Card({ card, close }) {
  const fields = card.fields;
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

// The owner's composer. One draft id (nonce) per draft, and one attempt per
// draft id: the exact body first posted (text, route and id) is posted again
// verbatim on every retry and every later check until an answer settles it,
// so the server carries a draft at most once and a check always finds its
// receipt. While an attempt is open the draft cannot change.
function Composer({ route, slug }) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [attempt, setAttempt] = useState("");
  const [note, setNote] = useState(null);
  const box = useRef(null);
  const frozen = sending || Boolean(attempt);
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
    const answer = await api("nonce");
    if (answer.status !== 200) {
      setNote(line("error", answer.status === 401 ? NO_SESSION : await refusal(answer)));
      return "";
    }
    const minted = await answer.json();
    const nonce = minted && typeof minted.nonce === "string" ? minted.nonce : "";
    if (!nonce) {
      setNote(line("error", "ae web gave no draft id"));
      return "";
    }
    const body = JSON.stringify({ text, route: route.id, nonce });
    setAttempt(body);
    return body;
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
      for (let tries = 0; tries < SEND_TRIES; tries += 1) {
        const answer = await api("send", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body,
        });
        if (answer.status === 200) {
          settle(await answer.json());
          return;
        }
        if (answer.status === 202 || answer.status === 503) {
          setNote(line("wait", answer.status === 202 ? "queued" : "busy, retrying"));
          await pause(SEND_AGAIN_MS);
          continue;
        }
        // A refusal of this very body (400, 409, 410, 413 and the other 4xx
        // but 401 and 408) settles the attempt with nothing sent: the draft
        // can be edited again and the next send is a new draft id. A lost
        // session (401), a request cut by its deadline (408) and any other
        // answer leave it open to be asked again: it may have carried.
        const open = answer.status === 401 || answer.status === 408;
        if (answer.status >= 400 && answer.status < 500 && !open) {
          setAttempt("");
        }
        setNote(line("error", answer.status === 401 ? NO_SESSION : await refusal(answer)));
        return;
      }
      setNote(line("error", "no receipt yet: it may still land; Check again asks once more"));
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
        ? html`<${Desk} desk=${state.desk} chosen=${chosen} choose=${choose} />`
        : html`<nav class="col" aria-label="Topics"></nav>`}
      <section class="chat" aria-label="Chat">
        ${state ? html`<${Stream} chat=${state.chat} />` : html`<div class="stream"></div>`}
        <${Composer} route=${state ? state.route : null} slug=${card ? card.slug : ""} />
      </section>
      ${card ? html`<${Card} card=${card} close=${close} />` : null}
    </main>
    <nav class="tabs" aria-label="Views">
      <button class=${view === "chat" ? "on" : ""} onClick=${() => setView("chat")}>Chat</button>
      <button class=${view === "chat" ? "" : "on"} onClick=${() => setView("topics")}>Topics</button>
    </nav>
  </div>`;
}

render(html`<${App} />`, document.getElementById("root"));
