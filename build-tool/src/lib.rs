#![feature(if_let_guard)]
#![feature(string_from_utf8_lossy_owned)]

use std::{
    env,
    ffi::OsStr,
    fs::create_dir,
    io::{self, BufRead, BufReader, stdout},
    path::{Path, absolute},
    process::Termination,
    sync::mpsc::{Receiver, channel},
    thread,
    time::Duration,
};

use portable_pty::{Child, CommandBuilder, NativePtySystem, PtySystem};
use ratatui::{
    DefaultTerminal, Frame, Terminal, TerminalOptions, Viewport,
    crossterm::{
        event::{self, Event, KeyCode, KeyModifiers},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Columns, Constraint, Layout, Rect},
    prelude::CrosstermBackend,
    widgets::{Block, Paragraph},
};
use thiserror::Error;

use crate::prompt::{Promt, PromtState};

mod prompt;
mod repl;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Tui failed, with error `{error}`")]
    Tui { error: io::Error },
    #[error("Failed to create dir, with error {error}")]
    BuildDirFailed { error: io::Error },
    #[error("Run command, failed with error {error}")]
    RunCommand { error: io::Error },
}

fn make_build_dir() -> Result<(), Error> {
    let build_dir = absolute(Path::new("build")).map_err(|error| Error::BuildDirFailed { error })?;

    if !build_dir.exists() {
        create_dir(build_dir).map_err(|error| Error::BuildDirFailed { error })?;
    }

    Ok(())
}

#[derive(Debug, Default)]
pub struct App {
    promt: PromtState,
    child: Option<Receiver<String>>,
    child_process: Option<(CommandBuilder, Box<dyn Child + Send + Sync>)>,
    outputs: Vec<String>,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn run_command(&mut self, command: CommandBuilder) -> Result<(), Error> {
        let (tx, rx) = channel();
        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(Default::default()).unwrap();

        self.child_process = Some((command.clone(), pair.slave.spawn_command(command).unwrap()));
        let reader = pair.master.try_clone_reader().unwrap();
        thread::spawn(move || {
            let reader = BufReader::new(reader);
            for line in reader.lines().flatten() {
                let _ = tx.send(line);
            }
        });
        self.child = Some(rx);

        Ok(())
    }

    pub fn run(mut self, mut main_terminal: DefaultTerminal) -> Result<(), Error> {
        let backend = CrosstermBackend::new(stdout());
        let mut repl_terminal =
            Terminal::with_options(backend, TerminalOptions { viewport: Viewport::Inline(3), ..Default::default() })
                .unwrap();
        loop {
            if let Some(_status) =
                self.child_process.as_mut().and_then(|(_name, child)| child.try_wait().ok()).flatten()
            {
                self.child_process = None;
                self.child = None;
            }

            while let Some(line) = self.child.as_ref().and_then(|child| child.try_recv().ok()) {
                repl_terminal
                    .insert_before(1, |buf| {
                        buf[(0, 0)].set_symbol(&line);
                    })
                    .unwrap();
            }

            repl_terminal.draw(|frame| self.draw(frame)).map_err(|error| Error::Tui { error })?;

            if !event::poll(Duration::from_millis(1)).map_err(|error| Error::Tui { error })? {
                continue;
            }
            match event::read().map_err(|error| Error::Tui { error })? {
                Event::Key(key) => {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                        _ => {}
                    }

                    let command = match self.promt.key_event(key) {
                        Some(command) => command,
                        None => continue,
                    };

                    repl::eval(&mut self, &mut main_terminal, command);
                    repl_terminal.clear().map_err(|error| Error::Tui { error })?;
                }
                Event::Resize(_col, row) => {
                    main_terminal.autoresize().map_err(|error| Error::Tui { error })?;
                    repl_terminal.autoresize().map_err(|error| Error::Tui { error })?;
                    repl_terminal.insert_before(row, |buf| buf.reset()).map_err(|error| Error::Tui { error })?;
                }
                _ => {}
            }
        }
    }

    fn run_config_menu(&mut self, main_terminal: &mut DefaultTerminal) {
        main_terminal.draw(|frame| self.draw_config(frame)).unwrap();
        let _ = event::read();
    }

    fn draw_config(&mut self, frame: &mut Frame) {
        let [layout] = Layout::default().constraints([Constraint::Fill(1)]).margin(4).areas(frame.area());
        frame.render_widget(Block::bordered(), layout);
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [promt] = Layout::vertical([Constraint::Length(3)]).areas(frame.area());
        frame.render_stateful_widget(
            Promt {
                running_cmd: self
                    .child_process
                    .as_ref()
                    .and_then(|(name, ..)| Some(name.get_argv().join(OsStr::new(" "))))
                    .as_ref()
                    .and_then(|s| s.to_str())
                    .unwrap_or(""),
                ..Default::default()
            },
            promt,
            &mut self.promt,
        );
        self.promt.set_cursor_pos(promt, frame);
    }
}

pub fn eval(line: &str) -> Result<(), Error> {
    //run_command(&mut Command::new("make").arg("release"))?;
    Ok(())
}
