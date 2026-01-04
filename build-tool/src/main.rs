use build_tool::App;
use ratatui::Viewport;

fn main() -> Result<(), build_tool::Error> {
    let terminal = ratatui::init_with_options(ratatui::TerminalOptions { viewport: Viewport::Fullscreen });
    let app_result = App::new().run(terminal);
    ratatui::restore();
    app_result
}
