/* Browser-boundary fakes; execute the actual app and graph scripts without a framework. */
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class Element {
  constructor() {
    this.listeners = new Map();
    this.children = [];
    this.attributes = new Map();
    this.dataset = {};
    this.style = {};
    this.value = "";
    this.hidden = true;
    this.textContent = "";
    const classes = new Set();
    this.classList = { add: (v) => classes.add(v), remove: (v) => classes.delete(v),
      contains: (v) => classes.has(v), toggle: (v, on) => on ? classes.add(v) : classes.delete(v) };
  }
  addEventListener(type, fn) { this.listeners.set(type, fn); }
  emit(type, extra = {}) { this.listeners.get(type)?.({ target: this, preventDefault() {}, ...extra }); }
  setAttribute(k, v) { this.attributes.set(k, v); }
  removeAttribute(k) { this.attributes.delete(k); }
  append(...children) { this.children.push(...children); }
  appendChild(child) { this.append(child); }
  replaceChildren(...children) { this.children = children; }
  set innerHTML(value) { if (value === "") this.children = []; }
  contains() { return false; }
  querySelector() { return null; }
  querySelectorAll() { return []; }
  focus() {}
  getBoundingClientRect() { return { width: 600, height: 400, left: 0, top: 0 }; }
  getContext() { return { setTransform() {}, measureText: (text) => ({ width: text.length * 7 }) }; }
}

async function app({ hash = "#/p/emacs?depth=2&dir=deps", storage = new Map() } = {}) {
  const elements = new Map();
  const element = (id) => {
    if (!elements.has(id)) elements.set(id, new Element());
    return elements.get(id);
  };
  element("theme-select").options = [{ value: "system" }, { value: "light" }, { value: "dark" }];
  const document = new Element();
  document.querySelector = (selector) => element(selector.slice(1));
  document.createElement = () => new Element();
  document.createTextNode = (text) => ({ textContent: text });
  document.documentElement = new Element();
  document.body = new Element();
  const listeners = new Map();
  const location = { hash, href: `http://localhost/${hash}` };
  const entries = [{ hash: "outside", state: null }, { hash, state: null }];
  let cursor = 1, travel = null, backCalls = 0;
  const setLocation = (value) => { location.hash = value; location.href = `http://localhost/${value}`; };
  const history = {
    get state() { return entries[cursor].state; },
    pushState(state, _, url) { entries.splice(cursor + 1); entries.push({ state, hash: url }); cursor++; setLocation(url); },
    replaceState(state, _, url) { entries[cursor] = { state, hash: url }; setLocation(url); },
    back() { backCalls++; travel = -1; },
    forward() { travel = 1; },
  };
  const requests = [];
  const sandbox = {
    document, location, history, console, URL, URLSearchParams, AbortController,
    performance: { now: () => 1000 },
    localStorage: { getItem: (key) => storage.get(key), setItem: (key, value) => storage.set(key, value) },
    setTimeout: () => 1, clearTimeout() {}, requestAnimationFrame: () => 1,
    ResizeObserver: class { observe() {} },
    fetch: (url, { signal } = {}) => {
      if (url.endsWith("/health")) return Promise.resolve({ ok: true, json: async () => ({ packages: 100 }) });
      return new Promise((resolve) => requests.push({ url, signal, resolve }));
    },
    innerWidth: 1400, devicePixelRatio: 1,
    matchMedia: (query) => ({ matches: query.includes("reduced-motion"), addEventListener() {} }),
    addEventListener: (type, fn) => listeners.set(type, fn),
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(fs.readFileSync(require.resolve("../web/graph.js"), "utf8"), sandbox);
  vm.runInContext(fs.readFileSync(require.resolve("../web/app.js"), "utf8"), sandbox);
  const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
  await flush();
  const respond = async (name) => {
    for (const request of requests.filter((r) => !r.done && decodeURIComponent(r.url.split("?")[0]).endsWith(`/${name}`))) {
      request.done = true;
      const params = new URLSearchParams(request.url.split("?")[1]);
      const body = request.url.includes("/graph/")
        ? { root: name, dir: params.get("dir"), generation: 1, truncated: 0,
          nodes: [{ name, degree: 1, depth: 0 }, { name: "child", degree: 1, depth: 1 }], edges: [] }
        : { name, generation: 1, deps: [], dependents: [], module_neighbors: [], dependents_count: 0 };
      request.resolve({ ok: true, json: async () => body });
    }
    await flush();
  };
  const finishTravel = async () => {
    if (travel === null) return;
    cursor += travel; travel = null;
    setLocation(entries[cursor].hash);
    listeners.get("popstate")({ state: history.state });
    listeners.get("hashchange")({});
    await flush();
  };
  return { ...sandbox.__guixvis, element, requests, history, location, storage, respond, finishTravel, flush,
    get backCalls() { return backCalls; }, get entries() { return entries; },
    foreignHash: (hash) => { history.pushState(null, "", hash); listeners.get("hashchange")({}); } };
}

test("Back is bounded at initial deep links and does not duplicate loading or loaded visits", async () => {
  const a = await app();
  assert.equal(a.element("back-btn").disabled, true);
  a.graph.onBack();
  assert.equal(a.backCalls, 0);
  a.graph.onPick("emacs");
  assert.equal(a.entries.length, 2);
  assert.equal(a.requests.length, 2);
  await a.respond("emacs");
  a.graph.onPick("emacs");
  assert.equal(a.entries.length, 2);
  a.graph.onPick("child");
  a.graph.onPick("child");
  assert.equal(a.entries.length, 3);
  assert.equal(a.requests.length, 4);
});

const snapshot = "a".repeat(64);
async function respondExact(a, id, { status = 200, wrongId = false } = {}) {
  for (const r of a.requests.filter((r) => !r.done && new URLSearchParams(r.url.split("?")[1]).get("id") === String(id))) {
    r.done = true;
    const name = decodeURIComponent(r.url.split("?")[0].split("/").pop());
    const params = new URLSearchParams(r.url.split("?")[1]);
    const ref = { id: wrongId ? id + 10 : id, name, snapshot, version: "1", catalog: id === 0 };
    const body = r.url.includes("/graph/")
      ? { root: name, root_id: ref.id, snapshot, generation: 1, dir: params.get("dir"), depth: Number(params.get("depth")),
        truncated: 0, nodes: [{ ...ref, degree: 1, depth: 0 },
          { ...ref, id: id === 0 ? 1 : 0, degree: 1, depth: 1 }], edges: [] }
      : { ...ref, generation: 1, command_safe: false,
        origin: { system: "x86_64-linux", executable: "/fixture/bin/guix", channels: [{ name: "guix", commit: snapshot }], verified: false },
        deps: [{ ...ref, id: id === 0 ? 1 : 0 }],
        dependents: [], module_neighbors: [], dependents_count: 0 };
    r.resolve({ ok: status === 200, status, json: async () => status === 200 ? body : { error: "Snapshot changed. Search again." } });
  }
  await a.flush();
}

test("exact hash, zero ID, node clicks, controls and history retain identity", async () => {
  const a = await app({ hash: `#/p/same?id=0&snapshot=${snapshot}` });
  assert.ok(a.requests.every((r) => r.url.includes(`id=0&snapshot=${snapshot}`)));
  await respondExact(a, 0);
  assert.equal(a.graph.rootName, 0);
  assert.ok(a.els.detail.children.some((e) => /origin unverified/.test(e.textContent)));
  a.graph.onPick(1);
  await respondExact(a, 1);
  assert.equal(a.state.detail.id, 1);
  a.element("depth-plus").emit("click");
  await respondExact(a, 1);
  a.element("dir-btn").emit("click");
  await respondExact(a, 1);
  assert.equal(new URLSearchParams(a.location.hash.split("?")[1]).get("id"), "1");
  a.graph.onBack();
  await a.finishTravel();
  await respondExact(a, 1);
  assert.equal(a.state.dir, "deps");
  a.element("graph").emit("keydown", { key: "ArrowRight" });
  a.element("graph").emit("keydown", { key: "ArrowRight" });
  assert.equal(a.graph.selected, 0);
  a.element("graph").emit("keydown", { key: "Enter" });
  await respondExact(a, 0);
  assert.equal(a.state.detail.id, 0);
});

test("stale or mismatched exact responses never fall back to a name", async () => {
  for (const options of [{ status: 409 }, { wrongId: true }]) {
    const a = await app({ hash: `#/p/same?id=0&snapshot=${snapshot}` });
    await respondExact(a, 0, options);
    assert.equal(a.state.detail, null);
    assert.equal(a.graph.engine, null);
    assert.equal(a.requests.length, 2, "no name retry");
    assert.match(a.els.status.textContent, /search|select/i);
  }
});

test("malformed exact hashes show an error without resolving the name", async () => {
  for (const suffix of ["id=0", `snapshot=${snapshot}`, `id=-1&snapshot=${snapshot}`, "id=0&snapshot=bad"]) {
    const a = await app({ hash: `#/p/same?${suffix}` });
    assert.equal(a.requests.length, 0);
    assert.match(a.els.status.textContent, /invalid|search/i);
  }
});

test("search chooses the second same-name variant by ID", async () => {
  const a = await app();
  await a.respond("emacs");
  a.element("view-btn").emit("click");
  const r = a.requests.find((r) => r.url.includes("/search?"));
  r.resolve({ ok: true, json: async () => ({ snapshot, items: [0, 1].map((id) => ({
    name: "same", id, snapshot, version: "1", deps: 0, dependents: 0,
  })) }) });
  await a.flush();
  a.els.packageList.children[1].children[0].emit("click");
  assert.ok(a.requests.slice(-2).every((r) => r.url.includes(`id=1&snapshot=${snapshot}`)));
  await respondExact(a, 1);
  assert.equal(a.state.detail.id, 1);
});

test("graph limit notices are reset between visits and distinguish unknown edges", async () => {
  const a = await app();
  const finish = async (name, limits) => {
    const request = a.requests.find((r) => !r.done && r.url.includes(`/graph/${name}?`));
    request.done = true;
    request.resolve({ ok: true, json: async () => ({ root: name, generation: 1, dir: "deps",
      nodes: [{ name, degree: 1, depth: 0 }], edges: [], ...limits }) });
    await a.respond(name);
  };
  await finish("emacs", { truncated: 17 });
  a.graph.onPick("child");
  await finish("child", { truncated: 0, edges_truncated: 20 });
  assert.doesNotMatch(a.els.pill.textContent, /17/);
  a.graph.onPick("next");
  await finish("next", { truncated: 0, edges_truncated: 0, edges_total: null, discovery_complete: true });
  assert.equal(a.els.pill.hidden, false);
  assert.match(a.els.pill.textContent, /unknown|limited/i);
});

test("Back restores package depth direction, guards pending travel, and preserves native Forward", async () => {
  const a = await app();
  await a.respond("emacs");
  a.element("depth-plus").emit("click");
  await a.respond("emacs");
  a.element("dir-btn").emit("click");
  await a.respond("emacs");
  a.graph.onPick("child");
  await a.respond("child");
  assert.equal(a.element("back-btn").disabled, false);
  a.element("back-btn").emit("click");
  a.graph.onBack();
  a.graph.onPick("ignored-while-travelling");
  a.element("depth-plus").emit("click");
  assert.equal(a.backCalls, 1);
  await a.finishTravel();
  await a.respond("emacs");
  assert.equal(a.state.name, "emacs");
  assert.equal(a.state.depth, 3);
  assert.equal(a.state.dir, "reverse");
  a.history.back();
  await a.finishTravel();
  await a.respond("emacs");
  assert.equal(a.state.depth, 3);
  assert.equal(a.state.dir, "deps");
  a.history.forward();
  await a.finishTravel();
  await a.respond("emacs");
  assert.equal(a.state.dir, "reverse");
  assert.equal(a.requests.length, 14, "popstate + hashchange must issue only one pair of requests");
});

test("Back cancels pending loads immediately and late results cannot replace the restored graph", async () => {
  const a = await app();
  await a.respond("emacs");
  a.graph.onPick("child");
  const pending = a.requests.slice(-2);
  a.graph.onBack();
  assert.ok(pending.every((r) => r.signal.aborted));
  await a.respond("child");
  assert.notEqual(a.state.loaded?.name, "child");
  await a.finishTravel();
  await a.respond("emacs");
  assert.equal(a.graph.rootName, "emacs");
  assert.equal(a.state.loaded.name, "emacs");
  assert.equal(a.element("back-btn").disabled, true);
});

test("unowned hash entries establish a new safe Back boundary", async () => {
  const a = await app();
  await a.respond("emacs");
  a.graph.onPick("child");
  await a.respond("child");
  a.foreignHash("#/p/external?depth=4&dir=reverse");
  await a.respond("external");
  a.graph.onBack();
  assert.equal(a.backCalls, 0);
  assert.equal(a.state.name, "external");
  assert.equal(a.state.depth, 4);
});

test("Graph style persists independently of theme and view without requests or navigation", async () => {
  const a = await app();
  await a.respond("emacs");
  const engine = a.graph.engine, hash = a.location.hash, count = a.requests.length;
  const select = a.element("graph-style");
  assert.equal(select.value, "bubbles");
  select.value = "rectangles"; select.emit("change");
  assert.equal(a.graph.engine, engine);
  assert.equal(engine.style, "rectangles");
  assert.equal(a.storage.get("guixvis-graph-style"), "rectangles");
  a.element("theme-select").value = "light";
  a.element("theme-select").emit("change");
  a.element("search").value = "preserved query";
  a.element("view-btn").emit("click");
  a.element("view-btn").emit("click");
  assert.equal(engine.style, "rectangles");
  assert.equal(a.element("search").value, "preserved query");
  assert.equal(a.location.hash, hash);
  assert.equal(a.requests.filter((r) => !r.url.includes("/search?")).length, count);
  const b = await app({ storage: a.storage });
  await b.respond("emacs");
  assert.equal(b.graph.engine.style, "rectangles");
  assert.equal(b.element("theme-select").value, "light");
});

test("keyboard selection and Enter follow rectangle nodes, then repeated Back stops at the boundary", async () => {
  const a = await app({ storage: new Map([["guixvis-graph-style", "rectangles"]]) });
  await a.respond("emacs");
  a.element("graph").emit("keydown", { key: "ArrowRight" });
  a.element("graph").emit("keydown", { key: "ArrowRight" });
  assert.equal(a.graph.selected, "child");
  a.element("graph").emit("keydown", { key: "Enter" });
  await a.respond("child");
  assert.equal(a.state.name, "child");
  a.graph.onBack();
  await a.finishTravel();
  await a.respond("emacs");
  a.graph.onBack();
  a.element("back-btn").emit("click");
  assert.equal(a.backCalls, 1);
  assert.equal(a.state.name, "emacs");
});

test("native Back cancels a pending graph and a new visit discards the forward branch", async () => {
  const a = await app();
  await a.respond("emacs");
  a.graph.onPick("child");
  const pending = a.requests.slice(-2);
  a.history.back();
  await a.finishTravel();
  assert.ok(pending.every((r) => r.signal.aborted));
  await a.respond("child");
  await a.respond("emacs");
  assert.equal(a.graph.rootName, "emacs");
  a.graph.onPick("replacement");
  await a.respond("replacement");
  assert.equal(a.entries.length, 3);
  a.graph.onBack();
  await a.finishTravel();
  await a.respond("emacs");
  a.history.forward();
  await a.finishTravel();
  await a.respond("replacement");
  assert.equal(a.graph.rootName, "replacement");
});

test("invalid saved style falls back and full long package names remain in tooltip and details", async () => {
  const name = "very-long-package-name-".repeat(8);
  const a = await app({ hash: `#/p/${name}?depth=2&dir=deps`, storage: new Map([["guixvis-graph-style", "invalid"]]) });
  await a.respond(name);
  assert.equal(a.graph.style, "bubbles");
  a.element("graph-style").value = "rectangles";
  a.element("graph-style").emit("change");
  a.graph.hoverNode(name);
  a.element("graph").emit("mousemove", { clientX: 100, clientY: 100 });
  assert.equal(a.els.tooltip.children[0].textContent, name);
  assert.equal(a.state.detail.name, name);
  assert.equal(a.graph.engine.node(name).name, name);
});
