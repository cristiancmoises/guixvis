/* guixvis web — application glue: state, hash routing, API, DOM.
   Graph math lives in graph.js; styling in style.css. No dependencies. */

"use strict";

(() => {
  const $ = (sel) => document.querySelector(sel);
  const motionPreference = window.matchMedia("(prefers-reduced-motion: reduce)");
  const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
  const api = {
    async get(path, signal) {
      const res = await fetch(path, { signal });
      if (!res.ok) {
        let msg = `HTTP ${res.status}`;
        try {
          const body = await res.json();
          if (body.error) msg = body.error;
        } catch (_) { /* not JSON */ }
        const error = new Error(msg);
        error.status = res.status;
        throw error;
      }
      return res.json();
    },
  };
  const state = {
    name: null,
    ref: null,
    depth: 2,
    dir: "deps",
    generation: 0,
    detail: null,
    graphData: null,
    positions: new Map(),
    abort: null,
    request: 0,
    loading: false,
    loaded: null, // { name, depth, dir } of the last successful load
    reducedMotion: motionPreference.matches,
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
    graphStyle: $("#graph-style"),
    backBtn: $("#back-btn"),
    viewBtn: $("#view-btn"),
    browser: $("#package-browser"),
    packageList: $("#package-list"),
    browseStatus: $("#browse-status"),
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

  let animationFrame = null;
  // Only entries created in this document lifetime are safe for in-app Back.
  // Reloads and manually edited hashes deliberately start a new boundary.
  const navigation = { session: `${Date.now()}-${Math.random()}`, entries: [], index: -1, pending: false };
  const graph = new GraphCanvas(els.canvas, {
    onPick: (key) => openGraphNode(key),
    onBack: () => back(),
    onNodeAction: (name, kind, ev) => {
      if (kind === "tooltip") showTooltip(name, ev.x, ev.y, true);
    },
    onTransform: () => { /* deselect handled internally */ },
  });
  graph.dirReverse = false;
  graph.reducedMotion = state.reducedMotion;
  graph.onInvalidate = requestGraphFrame;

  function applyGraphStyle(value, save = true) {
    const style = value === "rectangles" ? "rectangles" : "bubbles";
    els.graphStyle.value = style;
    graph.setStyle(style);
    try {
      if (save) localStorage.setItem("guixvis-graph-style", style);
    } catch (_) { /* storage unavailable */ }
  }
  let savedStyle = "bubbles";
  try { savedStyle = localStorage.getItem("guixvis-graph-style"); }
  catch (_) { /* storage unavailable */ }
  applyGraphStyle(savedStyle, false);
  els.graphStyle.addEventListener("change", () => applyGraphStyle(els.graphStyle.value));
  els.backBtn.addEventListener("click", back);
  updateBack();

  /* ---------- themes ---------- */
  function applyTheme(name, save = true) {
    const palette = name === "system" ? (systemTheme.matches ? "dark" : "light") : name;
    document.documentElement.dataset.theme = palette;
    document.documentElement.style.colorScheme = palette === "light" ? "light" : "dark";
    els.themeSelect.value = name;
    try {
      if (save) localStorage.setItem("guixvis-theme", name);
    } catch (_) { /* storage unavailable */ }
    graph.refreshColors();
  }
  els.themeSelect.addEventListener("change", () => applyTheme(els.themeSelect.value));
  let savedTheme = "system";
  try {
    savedTheme = localStorage.getItem("guixvis-theme") || "system";
  } catch (_) { /* storage unavailable */ }
  if (![...els.themeSelect.options].some((o) => o.value === savedTheme)) {
    savedTheme = "system";
  }
  applyTheme(savedTheme, false);
  systemTheme.addEventListener("change", () => {
    if (els.themeSelect.value === "system") applyTheme("system", false);
  });
  motionPreference.addEventListener("change", () => {
    state.reducedMotion = motionPreference.matches;
    graph.reducedMotion = state.reducedMotion;
    if (graph.engine) {
      graph.engine.reducedMotion = state.reducedMotion;
      if (state.reducedMotion && !graph.engine.separated) graph.engine.settle();
    }
    graph.invalidate();
  });

  /* ---------- hash routing ---------- */
  function packageRef(value) {
    const ref = typeof value === "string" ? { name: value } : value;
    if (!ref || typeof ref.name !== "string" || !ref.name) throw new Error("Invalid package reference. Search again.");
    if (ref.id == null && ref.snapshot == null) return { name: ref.name };
    if (!Number.isInteger(ref.id) || ref.id < 0 || ref.id > 0xffffffff ||
        typeof ref.snapshot !== "string" || !/^[a-f0-9]{64}$/.test(ref.snapshot)) {
      throw new Error("Invalid package identity. Search again.");
    }
    return { name: ref.name, id: ref.id, snapshot: ref.snapshot };
  }

  function refKey(ref) {
    return ref?.id != null ? `${ref.snapshot}/${ref.id}` : `name/${ref?.name}`;
  }

  function identityParams(ref) {
    return new URLSearchParams(ref.id != null ? { id: String(ref.id), snapshot: ref.snapshot } : {});
  }

  function openGraphNode(key) {
    const node = graph.engine?.node(key);
    if (node) open(node);
    else if (typeof key === "string" && !graph.engine?.exact) open(key);
  }

  function parseHash() {
    const m = /^#\/p\/([^?]+)(?:\?(.*))?$/.exec(location.hash);
    if (!m) return null;
    const params = new URLSearchParams(m[2] || "");
    let name;
    try { name = decodeURIComponent(m[1]); }
    catch (_) { return null; }
    const depth = Math.min(8, Math.max(1, parseInt(params.get("depth") || "2", 10) || 2));
    const dir = params.get("dir") === "reverse" ? "reverse" : "deps";
    const exact = params.has("id") || params.has("snapshot");
    if (exact && (params.getAll("id").length !== 1 || params.getAll("snapshot").length !== 1 ||
        !/^(0|[1-9][0-9]*)$/.test(params.get("id")))) {
      return { invalid: true };
    }
    try {
      const ref = packageRef(exact ? { name, id: Number(params.get("id")), snapshot: params.get("snapshot") } : name);
      return { ref, depth, dir };
    } catch (_) { return { invalid: true }; }
  }

  function canonicalHash(ref, depth, dir) {
    const params = identityParams(ref);
    params.set("depth", depth);
    params.set("dir", dir);
    return `#/p/${encodeURIComponent(ref.name)}?${params}`;
  }

  function updateBack() {
    els.backBtn.disabled = navigation.pending || navigation.index <= 0;
  }

  function writeHistory(url, push) {
    if (!push || navigation.index < 0) {
      navigation.entries = [url];
      navigation.index = 0;
      push = false;
    } else {
      navigation.entries.splice(navigation.index + 1);
      navigation.entries.push(url);
      navigation.index += 1;
    }
    const marker = { guixvis: { session: navigation.session, index: navigation.index } };
    history[push ? "pushState" : "replaceState"](marker, "", url);
    updateBack();
  }

  function back() {
    if (navigation.pending || navigation.index <= 0) return;
    navigation.pending = true;
    // Invalidate in-flight responses before the asynchronous popstate arrives.
    if (state.abort) state.abort.abort();
    state.request += 1;
    state.loading = false;
    updateBack();
    history.back();
  }

  function open(value, { keepPositions = true, push = true } = {}) {
    if (!value || navigation.pending) return;
    const ref = packageRef(value);
    const depth = state.depth;
    const dir = state.dir;
    const url = canonicalHash(ref, depth, dir);
    if (url === navigation.entries[navigation.index] && (state.loaded || state.loading)) {
      if (!state.loading) showSidebar();
      return;
    }
    // Retrying a failed visit must not create a duplicate history entry either.
    if (url !== navigation.entries[navigation.index]) writeHistory(url, push);
    load(ref, keepPositions);
  }

  /* ---------- loading ---------- */
  async function load(ref, keepPositions, retry = true) {
    const { name } = ref;
    if (state.abort) state.abort.abort();
    state.abort = new AbortController();
    const signal = state.abort.signal;
    const request = ++state.request;
    const depth = state.depth;
    const dir = state.dir;
    const prevPositions = keepPositions && state.ref?.snapshot === ref.snapshot ? state.positions : null;
    state.ref = ref;
    state.name = name;
    state.loading = true;
    state.loaded = null;
    state.detail = null;
    state.graphData = null;
    els.pill.hidden = true;
    graph.setSkeleton(true);
    els.status.textContent = `Loading ${name} graph…`;
    closeSidebar();

    const detailEl = els.detail;
    detailEl.innerHTML = "";
    for (let i = 0; i < 6; i++) {
      const s = document.createElement("div");
      s.className = "skel";
      s.style.width = `${60 + (i * 13) % 40}%`;
      detailEl.appendChild(s);
    }

    const timeout = setTimeout(() => {
      if (request === state.request && state.loading) {
        els.status.textContent = "Still loading… retry in a moment";
      }
    }, 30000);

    try {
      const exact = identityParams(ref);
      const graphParams = identityParams(ref);
      graphParams.set("dir", dir);
      graphParams.set("depth", depth);
      const [detail, graphData] = await Promise.all([
        api.get(`/api/v1/package/${encodeURIComponent(name)}${exact.size ? `?${exact}` : ""}`, signal),
        api.get(
          `/api/v1/graph/${encodeURIComponent(name)}?${graphParams}`, signal
        ),
      ]);
      clearTimeout(timeout);
      if (signal.aborted || request !== state.request) return;
      if (detail.generation !== graphData.generation || detail.snapshot !== graphData.snapshot) {
        if (retry && ref.id == null) return load(ref, false, false);
        throw new Error("Package index changed while loading. Select the package again.");
      }
      const resolved = packageRef(detail);
      if (detail.name !== name || graphData.root !== name ||
          (ref.id != null && refKey(resolved) !== refKey(ref)) ||
          (resolved.id != null && (graphData.root_id !== resolved.id ||
            !graphData.nodes.every((n) => n.snapshot === resolved.snapshot)))) {
        throw new Error("Package identity changed. Search and select the package again.");
      }
      const sameSnapshot = state.ref?.snapshot === resolved.snapshot;
      // A legacy deep link resolves once, then every subsequent action is exact.
      state.ref = resolved;
      const url = canonicalHash(resolved, depth, dir);
      navigation.entries[navigation.index] = url;
      history.replaceState(history.state, "", url);
      state.generation = detail.generation;
      buildGraph(graphData, sameSnapshot ? prevPositions : null);
      state.detail = detail;
      state.graphData = graphData;
      state.loaded = { ...resolved, depth, dir };
      renderDetail(detail, graphData);
      state.loading = false;
      graph.setSkeleton(false);
    } catch (err) {
      clearTimeout(timeout);
      if (signal.aborted || request !== state.request) return;
      state.loading = false;
      if (err.name === "AbortError") return;
      state.detail = null;
      state.graphData = null;
      state.loaded = null;
      state.positions = new Map();
      if (err.status === 409) {
        state.ref = null;
        state.name = null;
        navigation.entries = [];
        navigation.index = -1;
        updateBack();
        searchItems = [];
        searchedQuery = null;
        els.packageList.replaceChildren();
        hideResults();
      }
      els.status.textContent = `✗ ${err.message}`;
      detailEl.innerHTML = "";
      const empty = document.createElement("p");
      empty.className = "empty";
      empty.textContent = `Could not load "${name}". ${err.message}`;
      detailEl.appendChild(empty);
      graph.setGraph(null, {});
      showSidebar();
    }
  }

  function buildGraph(data, prevPositions) {
    const nodes = data.nodes;
    const engine = new GraphEngine(nodes, data.edges, {
      fresh: !prevPositions || prevPositions.size === 0,
      reducedMotion: state.reducedMotion,
    });
    const prev = prevPositions && prevPositions.size ? prevPositions : null;
    if (prev) engine.seedPositions(prev);
    graph.setGraph(engine, { root: data.root_id ?? data.root });
    graph.dirReverse = data.dir === "reverse";
    // Keep the live positions, including simulation ticks and manual dragging.
    state.positions = engine.pos;

    const n = data.nodes.length;
    const e = data.edges.length;
    els.status.textContent = `Graph of ${data.root} — ${n} nodes, ${e} edges · depth ${state.depth}${data.complete === false ? " · incomplete index" : ""}`;
    const limits = [];
    if (data.truncated > 0) limits.push(`+${data.truncated} beyond the graph limit`);
    if (data.edges_truncated) limits.push(`${data.edges_truncated} connections not drawn`);
    if (data.discovery_complete === false || data.edges_total === null) limits.push("Graph limited; total unknown");
    els.pill.textContent = limits.join(" · ");
    els.pill.hidden = limits.length === 0;
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
    if (d.id != null) el.append(mk("p", "meta", `ID ${d.id} · ${d.catalog ? "catalog" : "private dependency"}`));
    if (d.complete === false) el.append(mk("p", "meta", `Incomplete index: ${d.diagnostics_count} extraction diagnostics.`));
    if (d.origin) {
      const o = d.origin;
      el.append(mk("p", "meta", `Guix origin ${o.verified ? "verified" : "unverified"} · ${o.system || "unknown system"}`));
      el.append(mk("p", "meta", `Guix: ${o.executable || "unknown executable"}`));
      el.append(mk("p", "meta", (o.channels || []).map((c) => `${c.name}@${c.commit}`).join(" · ")));
    }

    if (d.synopsis) el.append(mk("p", "syn", d.synopsis));
    if (d.description) {
      const desc = mk("p", "desc", d.description);
      el.append(desc);
    }
    if (d.homepage) {
      const meta = mk("p", "meta");
      meta.append(mk("span", null, "Home: "));
      const url = safeUrl(d.homepage);
      if (url) {
        const a = document.createElement("a");
        a.href = url;
        a.target = "_blank";
        a.rel = "noopener noreferrer";
        a.textContent = d.homepage;
        meta.append(a);
      } else meta.append(document.createTextNode(d.homepage));
      el.append(meta);
    }
    if (d.file) {
      el.append(mk("p", "meta", `File: ${d.file}${d.line ? ":" + d.line : ""}`));
    }

    const actions = mk("section", "pkg-actions");
    actions.append(mk("h2", "sec-title", "Use this package"));
    actions.append(mk("p", "action-help", "Copy a command to review and run in your terminal."));
    if (d.command_safe === false) actions.append(mk("p", "action-help",
      "This private or ambiguous variant cannot be identified exactly by a Guix name/version command. The copied command may select a different package; review its definition first."));
    // Guix shell uses -- to begin the command, so validate its package operand.
    const operand = d.version ? `${d.name}@${d.version}` : d.name;
    const quoted = "'" + operand.replace(/'/g, "'\\''") + "'";
    for (const [label, prefix] of [
      ["Install", "guix install"], ["Remove", "guix remove"],
      ["Show", "guix show"], ["Shell", "guix shell"],
    ]) {
      const command = `${prefix}${label === "Shell" ? " " : " -- "}${quoted}`;
      const row = mk("div", "command-row");
      const preview = mk("code", null, command);
      const copy = mk("button", null, `Copy ${label.toLowerCase()}`);
      if (!/^[a-zA-Z0-9][a-zA-Z0-9+._-]*$/.test(d.name)) {
        preview.textContent = "Package name cannot be used in a command safely.";
        copy.disabled = true;
      }
      copy.setAttribute("aria-label", `Copy ${label.toLowerCase()} command for ${d.name}`);
      copy.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText(command);
          copy.textContent = "Copied";
        } catch (_) {
          copy.textContent = "Select to copy";
          const range = document.createRange();
          range.selectNodeContents(preview);
          const selection = window.getSelection();
          selection.removeAllRanges();
          selection.addRange(range);
        }
        setTimeout(() => { copy.textContent = `Copy ${label.toLowerCase()}`; }, 1800);
      });
      row.append(preview, copy);
      actions.append(row);
    }
    el.append(actions);

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
      depsWrap.append(relChip(dep));
    }
    if (d.deps.length > 12) depsWrap.append(moreChip(d.deps.length - 12));

    const revWrap = section("Dependents");
    for (const dep of d.dependents.slice(0, 12)) {
      const chip = relChip(dep);
      const cnt = mk("span", "cnt", `⤴${dep.dependents}`);
      chip.append(cnt);
      revWrap.append(chip);
    }
    if (d.dependents.length > 12) revWrap.append(moreChip(d.dependents.length - 12));

    const modWrap = section("Same module");
    for (const nb of d.module_neighbors.slice(0, 12)) {
      modWrap.append(relChip(nb));
    }
    if (d.module_neighbors.length > 12) {
      modWrap.append(moreChip(d.module_neighbors.length - 12));
    }
    if (!d.deps.length && !d.dependents.length && !d.module_neighbors.length) {
      el.append(mk("p", "empty", "No related packages found."));
    }

    showSidebar();
    markCurrentPackage();
  }

  function showSidebar() {
    els.sidebar.classList.add("open");
    els.sidebar.inert = false;
    if (window.innerWidth <= 1100) {
      const focusWasInView = els.wrap.contains(document.activeElement);
      els.browser.inert = true;
      els.canvas.inert = true;
      if (focusWasInView) els.close.focus();
      if (els.wrap.querySelector(".scrim")) return;
      const scrim = document.createElement("div");
      scrim.className = "scrim";
      scrim.addEventListener("click", closeSidebar);
      els.wrap.appendChild(scrim);
    }
  }

  function relChip(ref) {
    const { name, version, kind } = ref;
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
    const kinds = ref.kinds || [kind];
    if (kinds.includes("propagated")) chip.append(badge("P", "p"));
    if (kinds.includes("native")) chip.append(badge("N", "n"));
    chip.addEventListener("click", () => open(ref));
    chip.title = `Open ${name}${ref.id != null ? ` · ID ${ref.id}` : ""}`;
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
    return null;
  }

  function closeSidebar() {
    els.sidebar.classList.remove("open");
    els.sidebar.inert = window.innerWidth <= 1100;
    els.browser.inert = false;
    els.canvas.inert = false;
    const scrim = els.wrap.querySelector(".scrim");
    if (scrim) scrim.remove();
    if (els.sidebar.inert && els.sidebar.contains(document.activeElement)) els.viewBtn.focus();
  }

  /* ---------- search ---------- */
  let searchTimer = null;
  let searchIndex = 0;
  let searchItems = [];
  let searchAbort = null;
  let searchRequest = 0;
  let searchedQuery = null;

  function hideResults() {
    els.results.hidden = true;
    els.search.setAttribute("aria-expanded", "false");
    els.search.removeAttribute("aria-activedescendant");
  }

  function choosePackage(name) {
    hideResults();
    els.tooltip.hidden = true;
    open(name);
  }

  function markCurrentPackage() {
    for (const button of els.packageList.querySelectorAll("button")) {
      if (button.dataset.package === refKey(state.ref)) button.setAttribute("aria-current", "true");
      else button.removeAttribute("aria-current");
    }
  }

  function renderPackageList(data, q) {
    els.packageList.replaceChildren();
    const count = data.items.length;
    els.browseStatus.textContent = count
      ? `${count}${data.capped ? "+" : ""} packages${q ? ` matching “${q}”` : ""}.${data.capped ? " Refine your search to see more." : ""}`
      : `No packages match “${q}”. Try another name or description.`;
    for (const item of data.items) {
      const li = document.createElement("li");
      const button = document.createElement("button");
      button.dataset.package = refKey(item);
      const heading = document.createElement("span");
      heading.className = "package-title";
      const name = spanWithMarks(item.name, item.name_spans);
      name.className = "r-name";
      const version = document.createElement("span");
      version.className = "r-ver";
      version.textContent = item.version || "";
      heading.append(name, version);
      const synopsis = spanWithMarks(item.synopsis || "No description available.", item.synopsis_spans);
      synopsis.className = "package-synopsis";
      const meta = document.createElement("span");
      meta.className = "package-meta";
      meta.textContent = `${item.deps ?? 0} dependencies · ${item.dependents ?? 0} dependents${item.license ? ` · ${item.license}` : ""}`;
      button.append(heading, synopsis, meta);
      button.addEventListener("click", () => choosePackage(item));
      li.append(button);
      els.packageList.append(li);
    }
    markCurrentPackage();
  }

  els.viewBtn.addEventListener("click", () => {
    const show = els.browser.hidden;
    els.browser.hidden = !show;
    els.wrap.classList.toggle("show-packages", show);
    els.viewBtn.setAttribute("aria-pressed", String(show));
    els.viewBtn.textContent = show ? "Graph" : "Packages";
    els.viewBtn.title = show ? "Show dependency graph" : "Show package results";
    hideResults();
    closeSidebar();
    if (show && searchedQuery === null) runSearch(els.search.value.trim(), false);
    if (!show) requestGraphFrame();
  });

  els.search.addEventListener("input", () => {
    clearTimeout(searchTimer);
    if (searchAbort) searchAbort.abort();
    searchRequest += 1;
    searchItems = [];
    hideResults();
    const q = els.search.value.trim();
    if (!q && els.browser.hidden) {
      searchedQuery = null;
      return;
    }
    els.browseStatus.textContent = "Searching packages…";
    searchTimer = setTimeout(() => runSearch(q, !!q), 150);
  });

  async function runSearch(q, showSuggestions = true) {
    if (searchAbort) searchAbort.abort();
    searchAbort = new AbortController();
    const signal = searchAbort.signal;
    const request = ++searchRequest;
    els.browseStatus.textContent = "Searching packages…";
    try {
      const data = await api.get(`/api/v1/search?q=${encodeURIComponent(q)}&limit=100`, signal);
      if (signal.aborted || request !== searchRequest) return;
      searchedQuery = q;
      searchItems = data.items.slice(0, 20);
      searchIndex = 0;
      renderPackageList(data, q);
      if (showSuggestions && els.browser.hidden && document.activeElement === els.search) renderResults();
    } catch (err) {
      if (signal.aborted || request !== searchRequest || err.name === "AbortError") return;
      els.browseStatus.textContent = `Could not search packages. ${err.message}`;
      els.packageList.replaceChildren();
      if (!showSuggestions || !els.browser.hidden) return;
      els.results.hidden = false;
      els.search.setAttribute("aria-expanded", "true");
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
        li.id = `result-${i}`;
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
          choosePackage(item);
        });
        els.results.appendChild(li);
      });
    }
    els.results.hidden = false;
    els.search.setAttribute("aria-expanded", "true");
    if (searchItems.length) {
      els.search.setAttribute("aria-activedescendant", `result-${searchIndex}`);
      els.results.children[searchIndex]?.scrollIntoView({ block: "nearest" });
    } else els.search.removeAttribute("aria-activedescendant");
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
      choosePackage(searchItems[searchIndex]);
    } else if (ev.key === "Escape") {
      hideResults();
    }
  });

  els.search.addEventListener("focus", () => {
    if (searchItems.length && els.search.value.trim() === searchedQuery && els.browser.hidden) renderResults();
  });

  document.addEventListener("click", (ev) => {
    if (!els.search.contains(ev.target) && !els.results.contains(ev.target)) {
      hideResults();
    }
  });

  /* ---------- controls ---------- */
  els.depthPlus.addEventListener("click", () => {
    if (navigation.pending) return;
    if (state.depth < 8) {
      state.depth += 1;
      els.depthVal.textContent = state.depth;
      if (state.ref) open(state.ref, { push: true });
    }
  });
  els.depthMinus.addEventListener("click", () => {
    if (navigation.pending) return;
    if (state.depth > 1) {
      state.depth -= 1;
      els.depthVal.textContent = state.depth;
      if (state.ref) open(state.ref, { push: true });
    }
  });
  els.dirBtn.addEventListener("click", () => {
    if (navigation.pending) return;
    state.dir = state.dir === "deps" ? "reverse" : "deps";
    els.dirBtn.textContent = state.dir === "deps" ? "deps ▾" : "reverse ▾";
    els.dirBtn.classList.toggle("active", state.dir === "reverse");
    if (state.ref) open(state.ref, { push: true });
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
  function followHash() {
    const parsed = parseHash();
    navigation.pending = false;
    if (!parsed || parsed.invalid) {
      navigation.entries = [];
      navigation.index = -1;
      if (state.abort) state.abort.abort();
      state.request += 1;
      state.loading = false;
      state.ref = null;
      state.name = null;
      state.detail = null;
      state.loaded = null;
      state.graphData = null;
      graph.setGraph(null, {});
      closeSidebar();
      if (parsed?.invalid) els.status.textContent = "Invalid package identity. Search again.";
      updateBack();
      return;
    }
    const url = canonicalHash(parsed.ref, parsed.depth, parsed.dir);
    const marker = history.state && history.state.guixvis;
    if (marker && marker.session === navigation.session &&
        Number.isInteger(marker.index) && navigation.entries[marker.index] === url) {
      navigation.index = marker.index;
      updateBack();
    } else writeHistory(url, false);
    if (refKey(parsed.ref) !== refKey(state.ref) || parsed.depth !== state.depth || parsed.dir !== state.dir ||
        (!state.loading && !state.loaded)) {
      state.depth = parsed.depth;
      state.dir = parsed.dir;
      els.depthVal.textContent = state.depth;
      els.dirBtn.textContent = state.dir === "deps" ? "deps ▾" : "reverse ▾";
      els.dirBtn.classList.toggle("active", state.dir === "reverse");
      load(parsed.ref, true);
    }
  }
  window.addEventListener("popstate", followHash);
  window.addEventListener("hashchange", followHash);

  /* ---------- keyboard shortcuts ---------- */
  document.addEventListener("keydown", (ev) => {
    if (ev.target.matches("input, select, textarea, [contenteditable]")) return;
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
    const idx = graph.selected != null ? names.indexOf(graph.selected) : -1;
    if (ev.key === "ArrowRight" || ev.key === "ArrowDown") {
      ev.preventDefault();
      graph.selectNode(names[(idx + 1) % names.length]);
    } else if (ev.key === "ArrowLeft" || ev.key === "ArrowUp") {
      ev.preventDefault();
      graph.selectNode(names[(idx - 1 + names.length) % names.length]);
    } else if (ev.key === "Enter" && graph.selected != null) {
      ev.preventDefault();
      openGraphNode(graph.selected);
    }
  });

  /* ---------- tooltip ---------- */
  let tooltipTimer = null;
  function showTooltip(name, x, y, persist) {
    const node = graph.engine && graph.engine.node(name);
    if (!node) return;
    clearTimeout(tooltipTimer);
    els.tooltip.innerHTML = "";
    const b = document.createElement("b");
    b.textContent = node.name;
    els.tooltip.append(b);
    const syn = document.createElement("div");
    syn.className = "tt-syn";
    syn.textContent = `${node.version || ""}${node.id != null ? ` · ID ${node.id}` : ""} · degree ${node.degree} · depth ${node.depth}`;
    els.tooltip.append(syn);
    els.tooltip.hidden = false;
    const rect = els.tooltip.getBoundingClientRect();
    els.tooltip.style.left = `${Math.min(x + 14, window.innerWidth - rect.width - 10)}px`;
    els.tooltip.style.top = `${Math.min(y + 14, window.innerHeight - rect.height - 10)}px`;
    if (!persist) {
      tooltipTimer = setTimeout(() => {
        els.tooltip.hidden = true;
      }, 1400);
    }
  }

  els.canvas.addEventListener("mousemove", (ev) => {
    if (!graph.engine || graph.hovered == null) return;
    showTooltip(graph.hovered, ev.clientX, ev.clientY, false);
  });
  els.canvas.addEventListener("pointerleave", () => {
    clearTimeout(tooltipTimer);
    els.tooltip.hidden = true;
  });

  /* ---------- boot ---------- */
  async function boot() {
    const health = await api.get("/api/v1/health");
    if (health.packages > 0) {
      els.search.placeholder = `Search ${health.packages.toLocaleString()} packages…`;
      els.commit.textContent = `${health.packages.toLocaleString()} pkgs`;
      if (health.guix_commit) {
        els.commit.textContent += ` · ${health.guix_commit.slice(0, 7)}`;
      }
      if (health.origin_verified === false) els.commit.textContent += " · origin unverified";
    } else if (health.phase === "loading") {
      els.commit.textContent = `indexing… ${health.done}/${health.total || "?"}`;
      setTimeout(boot, 1500);
      return;
    } else {
      els.commit.textContent = health.phase === "failed" ? "index failed" : "no index";
      return;
    }

    const parsed = parseHash();
    if (parsed?.invalid) {
      els.status.textContent = "Invalid package identity. Search again.";
      return;
    }
    if (parsed) {
      state.depth = parsed.depth;
      state.dir = parsed.dir;
      els.depthVal.textContent = parsed.depth;
      els.dirBtn.textContent = state.dir === "deps" ? "deps ▾" : "reverse ▾";
      els.dirBtn.classList.toggle("active", state.dir === "reverse");
      open(parsed.ref, { keepPositions: false, push: false });
    } else {
      // No deep link: open a hub package so the canvas is never empty.
      open("emacs", { keepPositions: false, push: false });
    }
  }

  /* ---------- render loop ---------- */
  function resize() {
    const rect = els.wrap.getBoundingClientRect();
    graph.resize(rect.width, rect.height, window.devicePixelRatio || 1);
    const mobile = window.innerWidth <= 1100;
    const opened = els.sidebar.classList.contains("open");
    els.sidebar.inert = mobile && !opened;
    els.browser.inert = mobile && opened;
    els.canvas.inert = mobile && opened;
    if (!mobile) els.wrap.querySelector(".scrim")?.remove();
  }
  window.addEventListener("resize", resize);
  new ResizeObserver(resize).observe(els.wrap);
  resize();

  function requestGraphFrame() {
    if (animationFrame !== null || document.hidden || !els.browser.hidden) return;
    animationFrame = requestAnimationFrame(() => {
      animationFrame = null;
      if (!document.hidden && els.browser.hidden && graph.frame()) requestGraphFrame();
    });
  }
  document.addEventListener("visibilitychange", requestGraphFrame);
  requestGraphFrame();

  boot().catch((err) => {
    els.status.textContent = `✗ cannot reach API: ${err.message}`;
  });

  // Debug/testing hook (also handy in the browser console).
  window.__guixvis = { graph, state, els };
})();
