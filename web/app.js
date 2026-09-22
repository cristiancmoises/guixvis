/* guixvis web — application glue: state, hash routing, API, DOM.
   Graph math lives in graph.js; styling in style.css. No dependencies. */

"use strict";

(() => {
  const $ = (sel) => document.querySelector(sel);
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const api = {
    async get(path) {
      const res = await fetch(path, { signal: api.signal });
      if (!res.ok) {
        let msg = `HTTP ${res.status}`;
        try {
          const body = await res.json();
          if (body.error) msg = body.error;
        } catch (_) { /* not JSON */ }
        throw new Error(msg);
      }
      return res.json();
    },
  };
  const state = {
    name: null,
    depth: 2,
    dir: "deps",
    generation: 0,
    detail: null,
    graphData: null,
    positions: new Map(),
    abort: null,
    loading: false,
    loaded: null, // { name, depth, dir } of the last successful load
    reducedMotion,
  };

  const els = {
    search: $("#search"),
    results: $("#results"),
    dirBtn: $("#dir-btn"),
    depthVal: $("#depth-val"),
    depthMinus: $("#depth-minus"),
    depthPlus: $("#depth-plus"),
    copyLink: $("#copy-link"),
    commit: $("#commit"),
    canvas: $("#graph"),
    wrap: $("#canvas-wrap"),
    status: $("#graph-status"),
    pill: $("#truncated-pill"),
    hint: $("#hint"),
    sidebar: $("#sidebar"),
    detail: $("#detail"),
    close: $("#close-detail"),
    themeSelect: $("#theme-select"),
    tooltip: (() => {
      let t = $("#tooltip");
      if (!t) {
        t = document.createElement("div");
        t.id = "tooltip";
        t.hidden = true;
        document.body.appendChild(t);
      }
      return t;
    })(),
  };

  const graph = new GraphCanvas(els.canvas, {
    onPick: (name) => open(name),
    onNodeAction: (name, kind, ev) => {
      if (kind === "tooltip") showTooltip(name, ev.x, ev.y, true);
    },
    onTransform: () => { /* deselect handled internally */ },
  });
  graph.dirReverse = false;

  /* ---------- themes ---------- */
  function applyTheme(name) {
    document.documentElement.dataset.theme = name;
    els.themeSelect.value = name;
    try {
      localStorage.setItem("guixvis-theme", name);
    } catch (_) { /* storage unavailable */ }
    graph.refreshColors();
  }
  els.themeSelect.addEventListener("change", () => applyTheme(els.themeSelect.value));
  let savedTheme = "dark";
  try {
    savedTheme = localStorage.getItem("guixvis-theme") || "dark";
  } catch (_) { /* storage unavailable */ }
  if (![...els.themeSelect.options].some((o) => o.value === savedTheme)) {
    savedTheme = "dark";
  }
  applyTheme(savedTheme);

  /* ---------- hash routing ---------- */
  function parseHash() {
    const m = /^#\/p\/([^?]+)(?:\?(.*))?$/.exec(location.hash);
    if (!m) return null;
    const params = new URLSearchParams(m[1] ? m[2] || "" : location.hash.slice(1));
    const name = decodeURIComponent(m[1]);
    const depth = Math.min(8, Math.max(1, parseInt(params.get("depth") || "2", 10) || 2));
    const dir = params.get("dir") === "reverse" ? "reverse" : "deps";
    return { name, depth, dir };
  }

  function canonicalHash(name, depth, dir) {
    return `#/p/${encodeURIComponent(name)}?depth=${depth}&dir=${dir}`;
  }

  function open(name, { keepPositions = true, push = true } = {}) {
    if (!name) return;
    const depth = state.depth;
    const dir = state.dir;
    const loaded = state.loaded;
    if (
      loaded &&
      loaded.name === name &&
      loaded.depth === depth &&
      loaded.dir === dir &&
      !state.loading
    ) {
      return;
    }
    state.name = name;
    const url = canonicalHash(name, depth, dir);
    if (push) history.pushState({ name, depth, dir }, "", url);
    else history.replaceState({ name, depth, dir }, "", url);
    load(name, keepPositions);
  }

  /* ---------- loading ---------- */
  async function load(name, keepPositions) {
    if (state.abort) state.abort.abort();
    state.abort = new AbortController();
    api.signal = state.abort.signal;
    state.loading = true;
    const prevPositions = keepPositions ? state.positions : null;
    graph.setSkeleton(true);
    els.status.textContent = `Loading ${name} graph…`;
    els.sidebar.classList.remove("open");
    const scrim = els.wrap.querySelector(".scrim");
    if (scrim) scrim.remove();

    const detailEl = els.detail;
    detailEl.innerHTML = "";
    for (let i = 0; i < 6; i++) {
      const s = document.createElement("div");
      s.className = "skel";
      s.style.width = `${60 + (i * 13) % 40}%`;
      detailEl.appendChild(s);
    }

    const timeout = setTimeout(() => {
      if (state.loading) {
        els.status.textContent = "Still loading… retry in a moment";
      }
    }, 30000);

    try {
      const [detail, graphData] = await Promise.all([
        api.get(`/api/v1/package/${encodeURIComponent(name)}`),
        api.get(
          `/api/v1/graph/${encodeURIComponent(name)}?dir=${state.dir}&depth=${state.depth}`
        ),
      ]);
      clearTimeout(timeout);
      if (state.generation && detail.generation !== state.generation) {
        // Index swapped mid-flight; the data is stale relative to newer
        // responses. Re-fetch once.
        state.generation = detail.generation;
        return load(name, keepPositions);
      }
      state.generation = detail.generation;
      state.detail = detail;
      state.graphData = graphData;
      state.loaded = { name, depth: state.depth, dir: state.dir };
      renderDetail(detail, graphData);
      buildGraph(graphData, prevPositions);
      state.loading = false;
      graph.setSkeleton(false);
    } catch (err) {
      clearTimeout(timeout);
      state.loading = false;
      if (err.name === "AbortError") return;
      els.status.textContent = `✗ ${err.message}`;
      detailEl.innerHTML = "";
      const empty = document.createElement("p");
      empty.className = "empty";
      empty.textContent = `Could not load "${name}". ${err.message}`;
      detailEl.appendChild(empty);
      graph.setGraph(null, {});
    }
  }

  function buildGraph(data, prevPositions) {
    const nodes = data.nodes.map((n) => ({
      name: n.name,
      degree: n.degree,
      depth: n.depth,
      kind: n.kind,
    }));
    const engine = new GraphEngine(nodes, data.edges, {
      fresh: !prevPositions || prevPositions.size === 0,
      reducedMotion: state.reducedMotion,
    });
    const prev = prevPositions && prevPositions.size ? prevPositions : null;
    if (prev) engine.seedPositions(prev);
    if (state.reducedMotion) engine.settle(600);
    graph.setGraph(engine, { root: data.root });
    graph.dirReverse = data.dir === "reverse";
    // Remember positions for the next navigation.
    state.positions = new Map();
    for (const [name, p] of engine.pos) state.positions.set(name, { x: p.x, y: p.y });

    const n = data.nodes.length;
    const e = data.edges.length;
    els.status.textContent = `Graph of ${data.root} — ${n} nodes, ${e} edges · depth ${state.depth}`;
    if (data.truncated > 0) {
      els.pill.hidden = false;
      els.pill.textContent = `+${data.truncated} beyond budget — raise depth`;
    } else {
      els.pill.hidden = true;
    }
  }

  /* ---------- detail sidebar ---------- */
  function renderDetail(d, g) {
    const el = els.detail;
    el.innerHTML = "";
    const mk = (tag, cls, text) => {
      const e = document.createElement(tag);
      if (cls) e.className = cls;
      if (text !== undefined) e.textContent = text;
      return e;
    };

    const head = mk("div", "pkg-head");
    const h1 = mk("h1", null, d.name);
    const ver = mk("span", "ver", d.version || "");
    head.append(h1, ver);
    for (const lic of (d.licenses || []).slice(0, 3)) {
      head.append(mk("span", "lic", lic));
    }
    el.append(head);

    if (d.synopsis) el.append(mk("p", "syn", d.synopsis));
    if (d.description) {
      const desc = mk("p", "desc", d.description);
      el.append(desc);
    }
    if (d.homepage) {
      const meta = mk("p", "meta");
      meta.append(mk("span", null, "Home: "));
      const a = document.createElement("a");
      a.href = safeUrl(d.homepage);
      a.target = "_blank";
      a.rel = "noopener noreferrer";
      a.textContent = d.homepage;
      meta.append(a);
      el.append(meta);
    }
    if (d.file) {
      el.append(mk("p", "meta", `File: ${d.file}${d.line ? ":" + d.line : ""}`));
    }

    const counts = mk("div", "counts");
    const countBox = (label, value) => {
      const box = mk("div", "count");
      box.append(mk("b", null, String(value)));
      box.append(mk("span", null, label));
      return box;
    };
    counts.append(
      countBox("deps", d.deps.length),
      countBox("dependents", d.dependents_count),
      countBox("same module", d.module_neighbors.length)
    );
    el.append(counts);

    // Related packages: clickable chips (bubbles to explore further).
    const section = (title) => {
      el.append(mk("div", "sec-title", title));
      const wrap = mk("div", "rels");
      el.append(wrap);
      return wrap;
    };

    const depsWrap = section("Dependencies");
    for (const dep of d.deps.slice(0, 12)) {
      depsWrap.append(relChip(dep.name, dep.version, dep.kind));
    }
    if (d.deps.length > 12) depsWrap.append(moreChip(d.deps.length - 12));

    const revWrap = section("Dependents");
    for (const dep of d.dependents.slice(0, 12)) {
      const chip = relChip(dep.name, dep.version);
      const cnt = mk("span", "cnt", `⤴${dep.dependents}`);
      chip.append(cnt);
      revWrap.append(chip);
    }
    if (d.dependents.length > 12) revWrap.append(moreChip(d.dependents.length - 12));

    const modWrap = section("Same module");
    for (const nb of d.module_neighbors.slice(0, 12)) {
      modWrap.append(relChip(nb.name, nb.version));
    }
    if (d.module_neighbors.length > 12) {
      modWrap.append(moreChip(d.module_neighbors.length - 12));
    }
    if (!d.deps.length && !d.dependents.length && !d.module_neighbors.length) {
      el.append(mk("p", "empty", "No related packages found."));
    }

    els.sidebar.classList.add("open");
    if (window.innerWidth <= 1100) {
      const scrim = document.createElement("div");
      scrim.className = "scrim";
      scrim.addEventListener("click", closeSidebar);
      els.wrap.appendChild(scrim);
    }
  }

  function relChip(name, version, kind) {
    const chip = document.createElement("button");
    chip.className = "rel";
    const nm = document.createElement("span");
    nm.textContent = name;
    chip.append(nm);
    if (version) {
      const v = document.createElement("span");
      v.className = "r-ver";
      v.textContent = version;
      chip.append(v);
    }
    if (kind === "propagated") chip.append(badge("P", "p"));
    if (kind === "native") chip.append(badge("N", "n"));
    chip.addEventListener("click", () => open(name));
    chip.title = `Open ${name}`;
    return chip;
  }

  function badge(text, cls) {
    const b = document.createElement("span");
    b.className = cls;
    b.textContent = text;
    return b;
  }

  function moreChip(n) {
    const m = document.createElement("span");
    m.className = "more-link";
    m.textContent = `+${n} more`;
    return m;
  }

  function safeUrl(url) {
    try {
      const u = new URL(url);
      if (u.protocol === "http:" || u.protocol === "https:") return url;
    } catch (_) { /* relative or malformed */ }
    return "#";
  }

  function closeSidebar() {
    els.sidebar.classList.remove("open");
    const scrim = els.wrap.querySelector(".scrim");
    if (scrim) scrim.remove();
  }

  /* ---------- search ---------- */
  let searchTimer = null;
  let searchIndex = 0;
  let searchItems = [];

  els.search.addEventListener("input", () => {
    clearTimeout(searchTimer);
    const q = els.search.value.trim();
    if (!q) {
      els.results.hidden = true;
      return;
    }
    searchTimer = setTimeout(() => runSearch(q), 150);
  });

  async function runSearch(q) {
    if (state.abort) state.abort.abort();
    state.abort = new AbortController();
    api.signal = state.abort.signal;
    try {
      const data = await api.get(`/api/v1/search?q=${encodeURIComponent(q)}&limit=20`);
      searchItems = data.items;
      searchIndex = 0;
      renderResults();
    } catch (err) {
      if (err.name === "AbortError") return;
      els.results.hidden = false;
      els.results.innerHTML = "";
      const li = document.createElement("li");
      li.textContent = `✗ ${err.message}`;
      els.results.appendChild(li);
    }
  }

  function renderResults() {
    els.results.innerHTML = "";
    if (!searchItems.length) {
      const li = document.createElement("li");
      li.textContent = "no matches";
      els.results.appendChild(li);
    } else {
      searchItems.forEach((item, i) => {
        const li = document.createElement("li");
        li.setAttribute("role", "option");
        li.setAttribute("aria-selected", String(i === searchIndex));
        const name = spanWithMarks(item.name, item.name_spans);
        name.className = "r-name" + (item.nameMatch === false ? " r-name-syn" : "");
        li.append(name);
        if (item.version) {
          const v = document.createElement("span");
          v.className = "r-ver";
          v.textContent = item.version;
          li.append(v);
        }
        if (item.license) {
          const l = document.createElement("span");
          l.className = "r-lic";
          l.textContent = "·" + item.license;
          li.append(l);
        }
        if (item.deps != null) {
          const c = document.createElement("span");
          c.className = "r-counts";
          c.textContent = `⤵${item.deps} ⤴${item.dependents}`;
          li.append(c);
        }
        if (item.synopsis) {
          const s = spanWithMarks(item.synopsis, item.synopsis_spans);
          s.className = "r-syn";
          li.append(s);
        }
        li.addEventListener("mousedown", (ev) => {
          ev.preventDefault();
          open(item.name);
          els.search.value = "";
          els.results.hidden = true;
        });
        els.results.appendChild(li);
      });
    }
    els.results.hidden = false;
  }

  function spanWithMarks(text, spans) {
    const span = document.createElement("span");
    if (!spans || !spans.length) {
      span.textContent = text;
      return span;
    }
    const chars = Array.from(text);
    let pos = 0;
    for (const [s, e] of spans) {
      if (s > pos) span.append(document.createTextNode(chars.slice(pos, s).join("")));
      const mark = document.createElement("mark");
      mark.textContent = chars.slice(s, e).join("");
      span.append(mark);
      pos = e;
    }
    if (pos < chars.length) {
      span.append(document.createTextNode(chars.slice(pos).join("")));
    }
    return span;
  }

  els.search.addEventListener("keydown", (ev) => {
    if (ev.key === "ArrowDown" && !els.results.hidden) {
      ev.preventDefault();
      searchIndex = (searchIndex + 1) % Math.max(1, searchItems.length);
      renderResults();
    } else if (ev.key === "ArrowUp" && !els.results.hidden) {
      ev.preventDefault();
      searchIndex = (searchIndex - 1 + searchItems.length) % Math.max(1, searchItems.length);
      renderResults();
    } else if (ev.key === "Enter" && !els.results.hidden && searchItems[searchIndex]) {
      ev.preventDefault();
      open(searchItems[searchIndex].name);
      els.search.value = "";
      els.results.hidden = true;
    } else if (ev.key === "Escape") {
      els.results.hidden = true;
    }
  });

  document.addEventListener("click", (ev) => {
    if (!els.search.contains(ev.target) && !els.results.contains(ev.target)) {
      els.results.hidden = true;
    }
  });

  /* ---------- controls ---------- */
  function depthValue() {
    return parseInt(els.depthVal.textContent, 10) || 2;
  }

  els.depthPlus.addEventListener("click", () => {
    if (state.depth < 8) {
      state.depth += 1;
      els.depthVal.textContent = state.depth;
      if (state.name) open(state.name, { push: true });
    }
  });
  els.depthMinus.addEventListener("click", () => {
    if (state.depth > 1) {
      state.depth -= 1;
      els.depthVal.textContent = state.depth;
      if (state.name) open(state.name, { push: true });
    }
  });
  els.dirBtn.addEventListener("click", () => {
    state.dir = state.dir === "deps" ? "reverse" : "deps";
    els.dirBtn.textContent = state.dir === "deps" ? "deps ▾" : "reverse ▾";
    els.dirBtn.classList.toggle("active", state.dir === "reverse");
    if (state.name) open(state.name, { push: true });
  });
  els.copyLink.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(location.href);
      els.copyLink.textContent = "✓";
      setTimeout(() => (els.copyLink.textContent = "⧉"), 1200);
    } catch (_) {
      els.copyLink.textContent = "✗";
      setTimeout(() => (els.copyLink.textContent = "⧉"), 1200);
    }
  });
  els.close.addEventListener("click", closeSidebar);

  /* ---------- history ---------- */
  window.addEventListener("popstate", () => {
    const parsed = parseHash();
    if (!parsed) return;
    if (parsed.name !== state.name || parsed.depth !== state.depth || parsed.dir !== state.dir) {
      state.depth = parsed.depth;
      state.dir = parsed.dir;
      els.depthVal.textContent = state.depth;
      els.dirBtn.textContent = state.dir === "deps" ? "deps ▾" : "reverse ▾";
      els.dirBtn.classList.toggle("active", state.dir === "reverse");
      load(parsed.name, true);
    }
  });

  /* ---------- keyboard shortcuts ---------- */
  document.addEventListener("keydown", (ev) => {
    if (ev.target === els.search) return;
    if (ev.key === "/") {
      ev.preventDefault();
      els.search.focus();
    } else if (ev.key === "Escape") {
      closeSidebar();
      graph.clearSelection();
    }
  });

  els.canvas.addEventListener("keydown", (ev) => {
    const names = graph.engine ? graph.engine.names : [];
    if (!names.length) return;
    const idx = graph.selected ? names.indexOf(graph.selected) : -1;
    if (ev.key === "ArrowRight" || ev.key === "ArrowDown") {
      ev.preventDefault();
      graph.selectNode(names[(idx + 1) % names.length]);
    } else if (ev.key === "ArrowLeft" || ev.key === "ArrowUp") {
      ev.preventDefault();
      graph.selectNode(names[(idx - 1 + names.length) % names.length]);
    } else if (ev.key === "Enter" && graph.selected) {
      ev.preventDefault();
      open(graph.selected);
    }
  });

  /* ---------- tooltip ---------- */
  function showTooltip(name, x, y, persist) {
    const node = graph.engine && graph.engine.node(name);
    if (!node) return;
    els.tooltip.innerHTML = "";
    const b = document.createElement("b");
    b.textContent = name;
    els.tooltip.append(b);
    const syn = document.createElement("div");
    syn.className = "tt-syn";
    syn.textContent = `degree ${node.degree} · depth ${node.depth}`;
    els.tooltip.append(syn);
    els.tooltip.hidden = false;
    const rect = els.tooltip.getBoundingClientRect();
    els.tooltip.style.left = `${Math.min(x + 14, window.innerWidth - rect.width - 10)}px`;
    els.tooltip.style.top = `${Math.min(y + 14, window.innerHeight - rect.height - 10)}px`;
    if (!persist) {
      setTimeout(() => {
        els.tooltip.hidden = true;
      }, 1400);
    }
  }

  els.canvas.addEventListener("mousemove", (ev) => {
    if (!graph.engine || !graph.hovered) return;
    showTooltip(graph.hovered, ev.clientX, ev.clientY, false);
  });

  /* ---------- boot ---------- */
  async function boot() {
    const health = await api.get("/api/v1/health");
    if (health.packages > 0) {
      els.commit.textContent = `${health.packages.toLocaleString()} pkgs`;
      if (health.guix_commit) {
        els.commit.textContent += ` · ${health.guix_commit.slice(0, 7)}`;
      }
    } else if (health.phase === "loading") {
      els.commit.textContent = `indexing… ${health.done}/${health.total || "?"}`;
      setTimeout(boot, 1500);
      return;
    } else {
      els.commit.textContent = health.phase === "failed" ? "index failed" : "no index";
      return;
    }

    const parsed = parseHash();
    if (parsed) {
      state.depth = parsed.depth;
      state.dir = parsed.dir;
      els.depthVal.textContent = parsed.depth;
      els.dirBtn.textContent = state.dir === "deps" ? "deps ▾" : "reverse ▾";
      els.dirBtn.classList.toggle("active", state.dir === "reverse");
      load(parsed.name, false);
      history.replaceState(
        { name: parsed.name, depth: parsed.depth, dir: parsed.dir },
        "",
        canonicalHash(parsed.name, parsed.depth, parsed.dir)
      );
    } else {
      // No deep link: open a hub package so the canvas is never empty.
      open("emacs", { keepPositions: false, push: false });
    }
  }

  /* ---------- render loop ---------- */
  function resize() {
    const rect = els.wrap.getBoundingClientRect();
    graph.resize(rect.width, rect.height, window.devicePixelRatio || 1);
  }
  window.addEventListener("resize", resize);
  resize();

  function loop() {
    graph.frame();
    requestAnimationFrame(loop);
  }
  requestAnimationFrame(loop);

  boot().catch((err) => {
    els.status.textContent = `✗ cannot reach API: ${err.message}`;
  });

  // Debug/testing hook (also handy in the browser console).
  window.__guixvis = { graph, state, els };
})();
