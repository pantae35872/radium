use std::env;

use portable_pty::CommandBuilder;
use ratatui::DefaultTerminal;

use crate::App;

pub fn eval(app: &mut App, main_terminal: &mut DefaultTerminal, command: String) {
    if command == "config" {
        app.run_config_menu(main_terminal);
        main_terminal.clear().unwrap();
    } else {
        let mut cmd = CommandBuilder::new("cargo");
        cmd.arg("build");
        cmd.cwd(env::current_dir().unwrap().join("src/kernel"));
        app.run_command(cmd).unwrap();
    }
}
