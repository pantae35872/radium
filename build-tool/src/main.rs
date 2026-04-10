use std::{
    env::{args, var},
    io::{IsTerminal, stdout},
    process::exit,
};

use build_tool::{App, run_no_tui};

fn main() -> Result<(), build_tool::Error> {
    let start_command = args().skip(1).collect::<Vec<String>>().join(" ");
    if var("RADIUM_BUILD_TOOL_NO_TUI").is_ok_and(|e| !matches!(e.as_str(), "false" | "0")) || !stdout().is_terminal() {
        let _ = run_no_tui(&start_command).inspect_err(|error| {
            eprintln!("{error}");
            exit(1);
        });
        return Ok(());
    }

    let terminal = ratatui::init();
    let app_result = App::new().run(start_command, terminal);
    ratatui::restore();
    println!();
    app_result
}
