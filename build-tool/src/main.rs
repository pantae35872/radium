use std::env::args;

use build_tool::App;
use ratatui::{
    Viewport,
    crossterm::{self, event::EnableMouseCapture},
};

fn main() -> Result<(), build_tool::Error> {
    let terminal = ratatui::init_with_options(ratatui::TerminalOptions { viewport: Viewport::Fullscreen });
    let from_rebuild = args().nth(1).map(|a| a.parse::<bool>().unwrap_or(false)).unwrap_or(false);
    crossterm::execute!(std::io::stdout(), EnableMouseCapture).unwrap();
    let app_result = App::new().run(from_rebuild, terminal);
    ratatui::restore();
    println!();
    app_result
}
