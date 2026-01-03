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
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, ExitStatus, NativePtySystem, PtySystem};
use ratatui::{
    DefaultTerminal, Frame, Terminal, TerminalOptions, Viewport,
    crossterm::event::{self, Event, KeyCode, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    prelude::CrosstermBackend,
    style::{Style, Stylize},
    text::{Line, ToLine},
    widgets::{Block, BorderType, Padding},
};
use thiserror::Error;

use crate::widget::{
    CenteredParagraph,
    prompt::{CommandStatus, Promt, PromtState},
};

mod build;
mod widget;

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
    prompt: PromtState,
    executor: Arc<Mutex<CmdExecutor>>,
    child_output: Receiver<String>,
    child_process_name: Receiver<Option<String>>,
    running_cmd_name: Option<String>,
    output_collected: Vec<String>,
    vertical_scroll: usize,

    last_command: Option<String>,
    build_cmd_handle: Option<JoinHandle<Result<(), build::Error>>>,
    build_error: Option<build::Error>,

    delta_time: Duration,
    main_screen: MainScreen,
}

#[derive(Default)]
enum MainScreen {
    #[default]
    None,
    Config,
    Help,
    Error(String),
    Scrolling,
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
        let _ = self.running_cmd_name.send(Some(format!("`{command_display}` in {cmd_cwd}")));

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
            prompt: PromtState::default(),
            executor,
            child_output,
            child_process_name,
            running_cmd_name: None,
            build_cmd_handle: None,
            output_collected: Default::default(),
            build_error: None,
            last_command: None,
            vertical_scroll: 0,
            delta_time: Duration::from_millis(1),
            main_screen: MainScreen::None,
        }
    }

    pub fn run(mut self, mut main_terminal: DefaultTerminal) -> Result<(), Error> {
        let backend = CrosstermBackend::new(stdout());
        let mut repl_terminal =
            Terminal::with_options(backend, TerminalOptions { viewport: Viewport::Inline(4), ..Default::default() })
                .unwrap();

        loop {
            let start = Instant::now();
            if self.build_cmd_handle.as_ref().is_some_and(|handle| handle.is_finished()) {
                self.build_error = self.build_cmd_handle.unwrap().join().expect("Builder panicked").err();
                self.build_cmd_handle = None;
                self.last_command = None;
            }

            while let Ok(name) = self.child_process_name.try_recv() {
                self.running_cmd_name = name;
            }

            while let Ok(line) = self.child_output.try_recv() {
                self.output_collected.push(line.clone());
                repl_terminal
                    .insert_before(1, |buf| {
                        buf[(0, 0)].set_symbol(&line);
                    })
                    .unwrap();
                self.vertical_scroll = 0;
            }

            self.draw(&mut repl_terminal, &mut main_terminal)?;
            self.delta_time = Instant::now() - start;

            if !event::poll(Duration::from_millis(1)).map_err(|error| Error::Tui { error })? {
                continue;
            }

            let start = Instant::now();

            match event::read().map_err(|error| Error::Tui { error })? {
                Event::Key(_) if matches!(self.main_screen, MainScreen::Error(_) | MainScreen::Help) => {
                    self.main_screen = MainScreen::None;
                    self.last_command = None;
                    self.redraw(&mut repl_terminal, &mut main_terminal)?;
                }
                Event::Key(key) => {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                        KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.scroll_down(1);
                            continue;
                        }
                        KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.scroll_up(1, &mut main_terminal);
                            continue;
                        }
                        KeyCode::PageUp => {
                            let amount = self.page_amount(&mut main_terminal);
                            self.scroll_up(amount, &mut main_terminal);
                            continue;
                        }
                        KeyCode::PageDown => {
                            let amount = self.page_amount(&mut main_terminal);
                            self.scroll_down(amount);
                            continue;
                        }
                        _ => {}
                    }

                    if self.build_cmd_handle.as_ref().is_some_and(|cmd| !cmd.is_finished()) {
                        continue;
                    }

                    let command = match self.prompt.key_event(key) {
                        Some(command) => command,
                        None => continue,
                    };

                    self.eval(command);
                    repl_terminal.clear().map_err(|error| Error::Tui { error })?;
                }
                Event::Resize(_col, row) => {
                    main_terminal.autoresize().map_err(|error| Error::Tui { error })?;
                    repl_terminal.autoresize().map_err(|error| Error::Tui { error })?;
                    repl_terminal.insert_before(row, |buf| buf.reset()).unwrap();
                    self.redraw(&mut repl_terminal, &mut main_terminal)?;
                }
                _ => {}
            }

            self.delta_time = Instant::now() - start;
        }
    }

    fn page_amount(&mut self, main_terminal: &mut DefaultTerminal) -> usize {
        main_terminal.get_frame().area().height.saturating_sub(5) as usize
    }

    fn scroll_down(&mut self, delta: usize) {
        let prev = self.vertical_scroll;
        self.vertical_scroll = self.vertical_scroll.saturating_sub(delta);
        if self.vertical_scroll != prev {
            self.main_screen = MainScreen::Scrolling;
        }
    }

    fn scroll_up(&mut self, delta: usize, main_terminal: &mut DefaultTerminal) {
        let output = self.output_collected.len().saturating_sub(main_terminal.get_frame().area().height as usize - 4);
        let prev = self.vertical_scroll;
        self.vertical_scroll = (self.vertical_scroll + delta).clamp(0, output);
        if self.vertical_scroll != prev {
            self.main_screen = MainScreen::Scrolling;
        }
    }

    fn eval(&mut self, command: String) {
        let executor = self.executor.clone();

        match command.as_str() {
            "config" | "c" => {
                self.main_screen = MainScreen::Config;
                todo!("Config!");
            }
            "run" | "r" => {
                todo!("Run!");
            }
            "build" | "b" if self.build_cmd_handle.is_none() => {
                self.build_error = None;
                self.build_cmd_handle = Some(thread::spawn(move || {
                    build::build(&mut executor.lock().unwrap(), build::BuildConfig { mode: build::BuildMode::Debug })
                }));
            }
            "help" | "h" => {
                self.main_screen = MainScreen::Help;
            }
            "" => {}
            cmd => {
                self.main_screen = MainScreen::Error(format!("Unknown command `{cmd}` type `help` for more info."));
            }
        };
        self.last_command = Some(command);
    }

    fn output_scrolled(&self) -> impl IntoIterator<Item = &String> + use<'_> {
        self.output_collected.iter().rev().skip(self.vertical_scroll).rev()
    }

    fn redraw_child_output(&mut self, repl_terminal: &mut DefaultTerminal) -> Result<(), Error> {
        for line in self.output_scrolled() {
            repl_terminal
                .insert_before(1, |buf| {
                    buf[(0, 0)].set_symbol(&line);
                })
                .map_err(|error| Error::Tui { error })?;
        }
        Ok(())
    }

    fn redraw(
        &mut self,
        repl_terminal: &mut DefaultTerminal,
        main_terminal: &mut DefaultTerminal,
    ) -> Result<(), Error> {
        main_terminal.clear().map_err(|error| Error::Tui { error })?;
        repl_terminal.clear().map_err(|error| Error::Tui { error })?;

        self.redraw_child_output(repl_terminal)?;
        self.draw(repl_terminal, main_terminal)?;

        Ok(())
    }

    fn draw(&mut self, repl_terminal: &mut DefaultTerminal, main_terminal: &mut DefaultTerminal) -> Result<(), Error> {
        repl_terminal.draw(|frame| self.draw_repl(frame)).map_err(|error| Error::Tui { error })?;

        match self.main_screen {
            MainScreen::Config => {
                main_terminal.draw(|frame| self.draw_config(frame)).map_err(|error| Error::Tui { error })?;
            }
            MainScreen::Help => {
                main_terminal.draw(|frame| self.draw_help(frame)).map_err(|error| Error::Tui { error })?;
            }
            MainScreen::Error(ref error) => {
                let error = error.clone();
                main_terminal.draw(|frame| self.draw_error(frame, error)).map_err(|error| Error::Tui { error })?;
            }
            MainScreen::Scrolling => {
                self.redraw_child_output(repl_terminal)?;
                self.main_screen = MainScreen::None;
            }
            MainScreen::None => {}
        };

        Ok(())
    }

    fn draw_config(&mut self, frame: &mut Frame) {
        let [layout] = Layout::default().constraints([Constraint::Fill(1)]).margin(4).areas(frame.area());
        frame.render_widget(Block::bordered(), layout);
    }

    fn draw_help(&mut self, frame: &mut Frame) {
        let [layout] = Layout::default().constraints([Constraint::Fill(1)]).margin(4).areas(frame.area());
        let text = vec![
            Line::from(
                "This is the build tool for this project, it's a simple REPL, that you can type commands in the prompt,",
            ),
            Line::from("and press enter to evaluate."),
            Line::from("The avaiables commands are:"),
            Line::from("  `h` or `help` to show this popup"),
            Line::from("  `b` or `build` to build the project"),
            Line::from("  `r` or `run` to run with qemu"),
            Line::from(
                "  `c` or `config` to configure the kernel (not required if you want to just build the project)",
            ),
            Line::from("Also the controls are:"),
            Line::from("  Press `PAGE UP` or `CRTL-UP` to scroll up the console"),
            Line::from("  Press `PAGE DOWN` or `CRTL-DOWN` to scroll up the console"),
            Line::from("  Press `CTRL-C` to quit the build tool"),
            Line::from("  and the prompt controls is similar to a normal readline control"),
        ];

        frame.render_widget(
            CenteredParagraph::new(text).block(
                Block::bordered()
                    .border_style(Style::default().light_blue().bold())
                    .border_type(BorderType::Rounded)
                    .padding(Padding::symmetric(3, 1))
                    .title(Line::from("Help").centered().bold())
                    .title_bottom(Line::from("Press any key to close this popup").centered().bold()),
            ),
            layout,
        );
    }

    fn draw_error(&mut self, frame: &mut Frame, error: String) {
        let [layout] = Layout::default().constraints([Constraint::Fill(1)]).areas(frame.area());
        frame.render_widget(
            CenteredParagraph::new(error.to_line()).block(
                Block::bordered()
                    .border_style(Style::default().red().bold())
                    .border_type(BorderType::Rounded)
                    .padding(Padding::uniform(1))
                    .title(Line::from("Error").centered().bold())
                    .title_bottom(Line::from("Press any key to close this popup").centered().bold()),
            ),
            layout,
        );
    }

    fn draw_repl(&mut self, frame: &mut Frame) {
        let [status, prompt] = Layout::vertical([Constraint::Length(1), Constraint::Length(3)]).areas(frame.area());

        let command_status = if self.build_cmd_handle.is_some() {
            CommandStatus::Busy
        } else if self.build_error.is_some() {
            CommandStatus::Errored
        } else {
            CommandStatus::Idle
        };
        let display = self.build_error.as_ref().map(|e| e.to_string());
        let display =
            display.unwrap_or_else(|| self.running_cmd_name.as_ref().map(|e| e.as_str()).unwrap_or("Idle").to_string());
        let mut display = display.to_line().centered().style(Style::default().bold());
        if matches!(command_status, CommandStatus::Errored) {
            display = display.red();
        }
        frame.render_widget(display, status);
        frame.render_stateful_widget(
            Promt {
                running_cmd: self.last_command.as_ref().map(|e| e.as_str()).unwrap_or(""),
                delta_time: self.delta_time,
                command_status,
                ..Default::default()
            },
            prompt,
            &mut self.prompt,
        );

        if matches!(self.main_screen, MainScreen::None) {
            self.prompt.set_cursor_pos(prompt, frame);
        }
    }
}
