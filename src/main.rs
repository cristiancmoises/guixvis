//! guixvis entry point: terminal setup, event loop, clean shutdown.

use std::time::Duration;

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::DefaultTerminal;

use guixvis::app::App;
use guixvis::{ui, VERSION};

#[derive(Parser, Debug)]
#[command(
    name = "guixvis",
    version = VERSION,
    about = "Interactive package explorer and dependency visualizer for GNU Guix",
    long_about = "Indexes every GNU Guix package through `guix repl`, caches the result,\nand offers fuzzy search, package details, dependency/reverse-dependency\ntrees, and a force-directed dependency graph."
)]
struct Cli {
    /// Rebuild the package index even if the cache is fresh
    #[arg(short, long, global = true)]
    rebuild: bool,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
enum Cmd {
    /// Serve the local web UI on 127.0.0.1 (requires the `web` cargo feature)
    Web {
        /// Port to bind on 127.0.0.1
        #[arg(long, default_value_t = 8787)]
        port: u16,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Cmd::Web { port }) => run_web(cli.rebuild, port),
        None => run_tui(cli.rebuild),
    }
}

#[cfg(feature = "web")]
fn run_web(rebuild: bool, port: u16) -> anyhow::Result<()> {
    let state = guixvis::web::AppState::start(rebuild);
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(guixvis::web::serve(state, port))
}

#[cfg(not(feature = "web"))]
fn run_web(_rebuild: bool, _port: u16) -> anyhow::Result<()> {
    eprintln!(
        "guixvis was built without the web UI.\n\
         Reinstall with: cargo install --features web --path ."
    );
    std::process::exit(1);
}

fn run_tui(rebuild: bool) -> anyhow::Result<()> {
    let mut app = App::new(rebuild);
    let mut terminal: DefaultTerminal = ratatui::init();

    // Restore the terminal even if a panic escapes the event loop.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        default_hook(info);
    }));

    let result = run(&mut terminal, &mut app);

    ratatui::restore();
    app.shutdown();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    app.size = terminal
        .size()
        .map(|s| (s.width, s.height))
        .unwrap_or((80, 24));
    loop {
        let poll = if app.animating() || app.graph_dirty {
            Duration::from_millis(33)
        } else {
            Duration::from_millis(250)
        };

        if event::poll(poll)? {
            match event::read()? {
                Event::Key(key) => {
                    // Ignore release/repeat events; only Press matters.
                    if key.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    // Shift+Tab arrives as BackTab on most terminals.
                    if key.code == KeyCode::BackTab {
                        app.on_key(crossterm::event::KeyEvent::new(
                            KeyCode::Tab,
                            KeyModifiers::SHIFT,
                        ));
                    } else {
                        app.on_key(key);
                    }
                }
                Event::Resize(w, h) => {
                    app.size = (w, h);
                    app.dirty = true;
                }
                _ => {}
            }
        }

        app.on_tick();
        app.clamp_cursor();
        if app.pump_events() || app.pump_search() {
            app.dirty = true;
        }

        if app.quit {
            break;
        }

        if app.dirty || app.animating() {
            terminal.draw(|f| ui::draw(f, app))?;
            app.dirty = false;
            app.graph_dirty = false;
        }
    }
    Ok(())
}
