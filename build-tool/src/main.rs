use std::io::stdout;

use build_tool::App;
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    crossterm::{
        event, execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen},
    },
    prelude::CrosstermBackend,
    widgets::Block,
};

fn main() -> Result<(), build_tool::Error> {
    execute!(stdout(), EnterAlternateScreen).unwrap();
    let terminal = ratatui::init_with_options(ratatui::TerminalOptions { viewport: Viewport::Fullscreen });
    let app_result = App::new().run(terminal);
    ratatui::restore();
    app_result
}
