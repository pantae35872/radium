#![feature(if_let_guard)]
#![feature(string_from_utf8_lossy_owned)]

use std::{
    ffi::OsStr,
    io::{self, BufRead, BufReader, stdout},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use portable_pty::{CommandBuilder, ExitStatus, NativePtySystem, PtySystem};
use ratatui::{
    DefaultTerminal, Frame, Terminal, TerminalOptions, Viewport,
    crossterm::event::{self, Event, KeyCode, KeyModifiers},
    layout::{Constraint, Layout},
    prelude::CrosstermBackend,
    widgets::Block,
};
use thiserror::Error;

use crate::prompt::{Promt, PromtState};

mod build;
mod prompt;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Tui failed, with error `{error}`")]
    Tui { error: io::Error },
    #[error("Failed to create dir, with error {error}")]
    BuildDirFailed { error: io::Error },
    #[error("Run command, failed with error {error}")]
    RunCommand { error: io::Error },
}

pub struct App {
    promt: PromtState,
    executor: Arc<Mutex<CmdExecutor>>,
    child_output: Receiver<String>,
    child_process_name: Receiver<Option<String>>,
    running_cmd_name: Option<String>,

    build_cmd_handle: Option<JoinHandle<Result<(), build::Error>>>,
}

#[derive(Debug)]
struct CmdExecutor {
    output_stream: Sender<String>,
    running_cmd_name: Sender<Option<String>>,
}

impl CmdExecutor {
    pub fn run(&mut self, command: CommandBuilder) -> Result<ExitStatus, io::Error> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(Default::default()).unwrap();
        let reader = pair.master.try_clone_reader().unwrap();

        let mut process = pair.slave.spawn_command(command.clone()).unwrap();

        let command_display = command.get_argv().join(OsStr::new(" ")).to_str().unwrap_or("").to_string();
        let cmd_cwd = command.get_cwd().and_then(|e| e.to_str()).unwrap_or("unknown");
        let _ = self.output_stream.send(format!("Running `{command_display}` in {cmd_cwd}"));
        let _ = self.running_cmd_name.send(Some(format!("{command_display} in {cmd_cwd}")));

        let stream = self.output_stream.clone();
        thread::spawn(move || {
            let reader = BufReader::new(reader);
            for line in reader.lines().flatten() {
                let _ = stream.send(line);
            }
        });

        let result = process.wait()?;

        let _ = self.running_cmd_name.send(None);

        Ok(result)
    }
}

impl App {
    pub fn new() -> Self {
        let (output_stream, child_output) = channel();
        let (running_cmd_name, child_process_name) = channel();
        let executor = Arc::new(CmdExecutor { output_stream, running_cmd_name }.into());

        Self {
            promt: PromtState::default(),
            executor,
            child_output,
            child_process_name,
            running_cmd_name: None,
            build_cmd_handle: None,
        }
    }

    pub fn run(mut self, mut main_terminal: DefaultTerminal) -> Result<(), Error> {
        let backend = CrosstermBackend::new(stdout());
        let mut repl_terminal =
            Terminal::with_options(backend, TerminalOptions { viewport: Viewport::Inline(3), ..Default::default() })
                .unwrap();

        loop {
            if self.build_cmd_handle.as_ref().is_some_and(|handle| handle.is_finished()) {
                self.build_cmd_handle = None;
            }

            while let Ok(name) = self.child_process_name.try_recv() {
                self.running_cmd_name = name;
            }

            while let Ok(line) = self.child_output.try_recv() {
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

                    if self.build_cmd_handle.as_ref().is_some_and(|cmd| !cmd.is_finished()) {
                        continue;
                    }

                    let command = match self.promt.key_event(key) {
                        Some(command) => command,
                        None => continue,
                    };

                    self.eval(&mut main_terminal, command);
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

    fn eval(&mut self, main_terminal: &mut DefaultTerminal, command: String) {
        let executor = self.executor.clone();

        match command.as_str() {
            "config" => {
                self.run_config_menu(main_terminal);
                main_terminal.clear().unwrap();
            }
            "build" if self.build_cmd_handle.is_none() => {
                self.build_cmd_handle = Some(thread::spawn(move || {
                    build::build(&mut executor.lock().unwrap(), build::BuildConfig { mode: build::BuildMode::Debug })
                }));
            }
            _ => {}
        };
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
                running_cmd: self.running_cmd_name.as_ref().map(|e| e.as_str()).unwrap_or(""),
                ..Default::default()
            },
            promt,
            &mut self.promt,
        );
        self.promt.set_cursor_pos(promt, frame);
    }
}
