// Observe the real listener. Selected cases alter what a single response
// reports to the browser after the listener has answered it.
(() => {
  const real = window.fetch.bind(window);
  const pause = (ms) => new Promise(resolve => window.setTimeout(resolve, ms));
  const reply = (status, value) => new Response(JSON.stringify(value), {
    status, headers: {"Content-Type": "application/json"}
  });
  const s = window.__s3 = {
    nonce: 0, posts: [], topics: 0, state: null, stateCount: 0, loss: false,
    skew: null, holdState: false, cap: null, realReceipt: null, firstClick: null
  };
  s.reset = () => {
    s.nonce = 0;
    s.posts = [];
    s.realReceipt = null;
    s.firstClick = null;
  };
  document.addEventListener("click", event => {
    if (event.target.closest('button[data-action="answer"], button[data-action="new-topic"]')) {
      if (s.firstClick === null) s.firstClick = performance.now();
    }
  }, true);
  window.fetch = async (url, init) => {
    if (url === "/api/state") {
      if (s.holdState) return new Response(null, {status: 304});
      const headers = new Headers(init?.headers);
      if (!s.state) headers.delete("If-None-Match");
      const response = await real(url, {...init, headers});
      s.stateCount += 1;
      if (response.status === 200) s.state = await response.clone().json();
      return response;
    }
    if (url === "/api/topic/calm" && s.skew !== null) {
      const headers = new Headers(init?.headers);
      headers.delete("If-None-Match");
      const response = await real(url, {...init, headers});
      const value = await response.json();
      s.topics += 1;
      const wait = value.waits.find(wait => wait.item === "D67");
      if (wait && s.skew === "missing") wait.item = "D98";
      if (wait && s.skew !== "missing") wait.text = s.skew;
      return reply(response.status, value);
    }
    if (url === "/api/nonce") s.nonce += 1;
    if (url !== "/api/send") return real(url, init);

    s.posts.push({body: init.body, elapsed: performance.now() - s.firstClick});
    const lose = s.loss;
    s.loss = false;
    let response = await real(url, init);
    if (lose) {
      // Let the real worker finish. These receipt polls repeat the same body
      // under the same nonce. They are outside the application's POST count.
      for (let attempt = 0; response.status === 202 && attempt < 240; attempt += 1) {
        await pause(25);
        response = await real(url, init);
      }
      if (response.status === 200) s.realReceipt = await response.clone().json();
      if (s.realReceipt?.outcome === "sent") return reply(408, {error: "fixture lost the reply"});
    }
    s.lastStatus = response.status;
    if (response.status === 200) {
      const receipt = await response.clone().json();
      if (receipt.outcome === "refused") s.cap = receipt.words;
    }
    return response;
  };

  s.root = (which) => document.querySelector({
    desk: '[data-panel=topic][data-slug=smoke] .decision-answer[data-item="D67"]',
    card: '[data-panel=topic][data-slug=calm] .decision-answer[data-item="D67"]',
    topic: '[data-panel=all] .new-topic'
  }[which]);
  s.field = (which, field) => s.root(which)?.querySelector('input[data-field="' + field + '"]');
  s.button = (which, action) => s.root(which)?.querySelector('button[data-action="' + action + '"]');
  s.status = (which) => s.root(which)?.querySelector('[data-role="status"]')?.textContent || "";
  s.preview = () => s.root("topic")?.querySelector('[data-role="preview"]')?.textContent?.trim() || "";
  s.chip = (slug) => document.body.textContent.toLowerCase().includes("asked for #" + slug);
  s.set = async (which, field, value) => {
    const input = s.field(which, field);
    if (!input) throw new Error("missing " + which + " " + field);
    input.focus();
    input.value = value;
    input.dispatchEvent(new InputEvent("input", {bubbles: true, data: value, inputType: "insertText"}));
    input.dispatchEvent(new Event("change", {bubbles: true}));
    input.blur();
    await pause(0);
  };
  s.press = async (which, action) => {
    s.button(which, action)?.click();
    await pause(0);
  };
  s.paste = (which, field, text) => {
    const input = s.field(which, field);
    const data = new DataTransfer();
    data.setData("text/plain", text);
    const event = new ClipboardEvent("paste", {bubbles: true, cancelable: true, clipboardData: data});
    input.dispatchEvent(event);
    return event.defaultPrevented;
  };
  s.wake = () => document.dispatchEvent(new Event("visibilitychange"));
  s.hidden = (hidden) => {
    if (hidden) {
      Object.defineProperty(document, "hidden", {configurable: true, value: true});
      Object.defineProperty(document, "visibilityState", {configurable: true, value: "hidden"});
    } else {
      delete document.hidden;
      delete document.visibilityState;
    }
    s.wake();
  };
})()
