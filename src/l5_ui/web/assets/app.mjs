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

function App() {
  const [state, setState] = useState(null);
  const [problem, setProblem] = useState("");
  const tag = useRef("");

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

  return html`<div class="app">
    <${Head} state=${state} />
    ${problem ? html`<p class="problem" role="status">${problem}</p>` : null}
    <main>
      <section class="chat" aria-label="Chat">
        ${state ? html`<${Stream} chat=${state.chat} />` : html`<div class="stream"></div>`}
      </section>
    </main>
  </div>`;
}

render(html`<${App} />`, document.getElementById("root"));
