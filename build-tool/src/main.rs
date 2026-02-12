use std::env::args;

use build_tool::App;

fn main() -> Result<(), build_tool::Error> {
    let terminal = ratatui::init();
    let from_rebuild = args().nth(1);
    let app_result = App::new().run(from_rebuild, terminal);
    ratatui::restore();
    println!();
    app_result
}
