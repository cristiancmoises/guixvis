# Using Guixvis

Guixvis reads the packages defined by your current Guix channels. It helps you
find a package, inspect its inputs, and understand what depends on it. The
index describes available packages; it does not track what is installed in
each of your profiles.

## Start with a package

Run `guixvis` for the terminal interface or `guixvis web` for the browser at
`http://127.0.0.1:8787`. The web server requires a build with `--features web`.
The first run builds an index; later runs load its binary snapshot. Updating
the verified Guix origin invalidates that snapshot automatically.

Type a name or words from a synopsis. Several words must all match. Name
matches rank ahead of synopsis-only matches. Clear the search to browse
packages with many dependents. The terminal shows up to 500 results. The
browser's Packages view shows up to 100, with 20 suggestions in its dropdown.
Refine the search when the result count reaches the cap.

In the terminal, `Tab` and `Shift+Tab` move between all four tabs without
changing the Overview package. The header shows **Search** or **Navigate**.
Overview starts in Search. Press `/` from any tab to edit its query; every
printable character is input, including `q`, digits, and `+`. `Enter` or
`Esc` ends editing without activating a row or losing the query. Arrow and
page keys work in both modes. `Ctrl+U` clears the current query; `F1` opens
help and `Ctrl+C` quits in either mode.

In Navigate, command letters work even if a filter remains: `d/r/v` and
`1–4` choose tabs, `T` changes theme, `R` rebuilds, and `q` quits.
`Esc` clears a local filter before returning through graph history.

Dependencies and Reverse deps each have their own case-insensitive literal
filter over names and versions. All words must match. These searches cover
every reachable package object in the index, regardless of expanded rows or
tree display depth. They do not search the whole catalog or replace the
Overview query. Selecting a different Overview result resets related views.

The index follows actual Guix package objects, including private variants and
multiple versions with the same name. Inputs (`I`), propagated inputs (`P`),
and native inputs (`N`) are included. They describe declared package relations
for the indexed system, not an installed-store closure or derivation graph.
Extraction diagnostics are surfaced as incomplete-index warnings.

The terminal graph starts at depth 1 with focused edges. `Enter` follows a
node; `+` and `−` change depth from 1 to 8. `g` refocuses on Overview.
Wide terminals show both canvas and package list; narrow terminals show the
list. Complete selected names and versions wrap below it, with `[`/`]`
for scrolling. `e` changes edge mode and `l` toggles canvas labels.

Graph search filters only projected nodes, without moving the root or rerunning
layout. Its 200-node limit includes the root; edges are capped at 3,000.
A separate traversal-work limit can leave totals unknown; the UI marks that
explicitly. Dependency-tab search is not limited by the graph projection.

In the browser, **Packages** keeps the results visible. Choose a row to read
its details, then follow dependency chips or switch to **Graph**. The direction
button switches between dependencies and dependents; `+` and `−` change depth.
The copy-link button copies the exact package ID, snapshot, direction, and depth.
IDs are scoped to a snapshot. If it has changed, search again: the client does
not silently substitute a same-name package. Legacy name-only links still work. Links
refer to your local service; another person needs Guixvis running to open one.

Click a graph node to follow it. Right-click the canvas or press the visible
**Back** button to return one step in Guixvis graph navigation, restoring that
view's package, depth, and direction. At the first in-app view, Back is disabled;
it cannot take you to an earlier website. The
browser's own Back and Forward controls still work for normal page history.
Reloading or editing the address creates a new in-app Back boundary.

**Graph style** offers labeled **Bubbles** and **Rectangles**; the selection is
saved in this browser. Bubble labels prioritize the root, selection, and
high-degree nodes that fit. Rectangles show names inside the shapes, shortening
long names. Labels are measured and drawn at the same font size, avoiding
compressed letters at normal browser zoom. Hover a node for its tooltip,
or focus the canvas and use arrow
keys to select a node, then `Enter` to follow it. Drag a node with the primary
mouse button; drag the background to pan. The wheel zooms, and a two-finger
pinch zooms on a touch screen. Those gestures do not follow a node. On touch,
a brief tap follows one; a long press shows its details. The visible Back
button remains available without a mouse.

## Use the package

The browser detail pane has previews and copy buttons for four commands:

```sh
guix install -- 'emacs@30.2'
guix remove -- 'emacs@30.2'
guix show -- 'emacs@30.2'
guix shell 'emacs@30.2'
```

The version above is an example; previews use the selected package's version.
Private variants and ambiguous name/version pairs carry a warning: a shell
specification may not reproduce that exact object. Review the command in your
own terminal before running it. Installation and
removal affect the Guix profile selected by your shell environment. Guixvis
does not execute these commands and does not request elevated privileges.
The `guix shell` command starts an environment containing the package; `--`
is deliberately omitted because it would start the command portion instead.

Copy actions are available for conventional Guix package names (letters,
digits, `+`, `.`, `_`, and `-`, beginning with a letter or digit). Other names
can still be inspected. If clipboard access is unavailable, the browser
selects the visible command so you can copy it manually.

## Themes

The terminal starts with its saved palette or `dark`. In Navigate mode,
press `T` to cycle and save a choice. `guixvis --theme nord` overrides that choice for one run; cycling
with `T` still saves the newly selected palette. The preference lives in
`$XDG_CONFIG_HOME/guixvis/theme`, falling back to `~/.config/guixvis/theme`.
Missing or invalid preferences use `dark`; an unwritable configuration
directory leaves the current session usable. `NO_COLOR` takes precedence over
all palette choices and uses the existing grayscale rendering.

The browser defaults to **system**, which follows the operating system's
light/dark setting. Choosing a named palette saves it in browser storage and
overrides system changes. Selecting **system** restores automatic switching.
Storage is specific to the browser and service address. Browser and terminal
preferences are independent.

Both interfaces offer `dark`, `one`, `light`, `dracula`, `nord`, `gruvbox-dark`,
`tokyo-night`, `catppuccin-mocha`, and `tron`. The browser also respects reduced
motion, pauses graph animation when hidden, and stops redrawing a settled graph.

## Emacs

Use Emacs 27.1 or newer and add the included Lisp directory:

```elisp
(add-to-list 'load-path "/path/to/guixvis/elisp")
(require 'guixvis)
```

Start `guixvis web` separately. Then run `M-x guixvis-search` and enter a
query, or `M-x guixvis-package` and enter an exact package name. Requests run
asynchronously; a slow service does not block editing. New requests supersede
old ones, and failures explain how to start or configure the service.

| Key | Search buffer | Package buffer |
| --- | --- | --- |
| `RET` | Open the package at point | Follow the package button at point |
| `s` | Search again | Start a search |
| `g` | Refresh results | Refresh details |
| `w` | Choose a command to copy | Choose a command to copy |
| `b` | Open the browser | Open the browser |
| `q` | Quit the window | Quit the window |

Use `TAB` and `Shift+TAB` to move between related-package buttons. Copied
commands go to the kill ring; paste them into a shell when you want to run
them. On a related-package button, `w` copies a command for that package;
elsewhere it uses the current package. `/` also starts a search from the
results buffer. Results can be sorted using the table headers, including numeric
dependency counts. Native buffers inherit your Emacs theme.

Search rows and related-package buttons retain exact IDs and snapshot tokens.
Refresh never replaces an expired reference with another same-name variant.
A stale snapshot clears the old details and asks you to search again. Details
also show the selected Guix origin and whether it could be verified.

Settings are available through `M-x customize-group RET guixvis`:

| Setting | Default | Purpose |
| --- | --- | --- |
| `guixvis-program` | `guixvis` from `exec-path` | Executable for the TUI |
| `guixvis-arguments` | empty | Extra TUI arguments, for example `("--theme" "nord")` |
| `guixvis-web-url` | `http://127.0.0.1:8787` | Local HTTP API and browser address |
| `guixvis-search-limit` | `100` | Requested results, from 1 to 500 |
| `guixvis-request-timeout` | `10` | Seconds to wait before reporting a timeout |

`guixvis-web-url` accepts loopback HTTP(S) addresses. The bundled server uses
HTTP. For a different port, run `guixvis web --port 8899` and set the URL to
`http://127.0.0.1:8899`.

`M-x guixvis` opens the terminal UI and reuses a live `*guixvis*` buffer. Quit
the running TUI before launching it with different arguments. A prefix argument
prompts for extra arguments and supports quoted values. `M-x guixvis-web`
opens the website. If you use Emacs-Guix, `(guixvis-popup-install)` adds the
existing `v` and `V` launcher entries to its popup.

## Local API and cache

Search rows, details, and graph nodes expose `id` and `snapshot`. For an exact
request, send both query parameters to `/api/v1/package/<name>` or
`/api/v1/graph/<name>`. Names remain readable labels; they are not unique IDs.
Name-only requests are retained for compatibility.

The API returns JSON errors: 400 for malformed references, 404 for a missing
ID, 409 for an expired snapshot, and 503 while no index is available.
Responses include completeness and diagnostic counts; health also reports
origin verification. A snapshot token survives a cache reload but changes
when the indexed content or origin changes.

The v5 cache preserves the old v4 file and rebuilds on first use. Guix selection
is `GUIX`, then `PATH`, then standard profile locations. The selected launcher,
system, and every channel commit form its origin. Mutable `GUIX_PACKAGE_PATH`
modules or failed origin probes are shown as unverified, not assumed fresh.

## When something looks wrong

- **The browser says the index is loading:** wait for the first build. Its
  duration depends on whether Guile's package modules are already compiled.
- **Emacs cannot connect:** check that `guixvis web` is running with the web
  feature enabled, then compare the port with `guixvis-web-url`.
- **A package is missing:** check your Guix channels and refine the query;
  capped results do not include every match. Restart after `guix pull`.
- **The graph omits packages:** depth and node limits keep it usable. Follow a
  package to inspect its neighborhood, or use the dependency lists.
- **Colors do not change in the terminal:** check whether `NO_COLOR` is set.
- **A rebuild fails:** the error usually comes from the local Guix command.
  Inspect that error before removing caches. Corrupt snapshots are quarantined
  automatically; `guixvis --rebuild` requests a fresh index.
