# Using Guixvis

Guixvis reads the packages defined by your current Guix channels. It helps you
find a package, inspect its inputs, and understand what depends on it. The
index describes available packages; it does not track what is installed in
each of your profiles.

## Start with a package

Run `guixvis` for the terminal interface or `guixvis web` for the browser at
`http://127.0.0.1:8787`. The web server requires a build with `--features web`.
The first run builds an index; later runs load its binary snapshot. Updating
your Guix channel invalidates that snapshot automatically.

Type a name or words from a synopsis. Several words must all match. Name
matches rank ahead of synopsis-only matches. Clear the search to browse
packages with many dependents. The terminal shows up to 500 results. The
browser's Packages view shows up to 100, with 20 suggestions in its dropdown.
Refine the search when the result count reaches the cap.

In the terminal, `Tab` moves between overview, dependencies, reverse
dependencies, and graph. Arrow keys work while searching. Letter shortcuts
such as `d`, `r`, and `v` act when the search is empty. Press `?` for help.

In the browser, **Packages** keeps the results visible. Choose a row to read
its details, then follow dependency chips or switch to **Graph**. The direction
button switches between dependencies and dependents; `+` and `−` change depth.
The copy-link button copies the current package, direction, and depth. Links
refer to your local service; another person needs Guixvis running to open one.

## Use the package

The browser detail pane has previews and copy buttons for four commands:

```sh
guix install -- 'emacs'
guix remove -- 'emacs'
guix show -- 'emacs'
guix shell 'emacs'
```

Review the command in your own terminal before running it. Installation and
removal affect the Guix profile selected by your shell environment. Guixvis
does not execute these commands and does not request elevated privileges.
The `guix shell` command starts an environment containing the package; `--`
is deliberately omitted because it would start the command portion instead.

Copy actions are available for conventional Guix package names (letters,
digits, `+`, `.`, `_`, and `-`, beginning with a letter or digit). Other names
can still be inspected. If clipboard access is unavailable, the browser
selects the visible command so you can copy it manually.

## Themes

The terminal starts with its saved palette or `dark`. Press `T` to cycle and
save a choice. `guixvis --theme nord` overrides that choice for one run; cycling
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
