#![feature(if_let_guard)]
#![feature(vec_push_within_capacity)]
#![feature(string_from_utf8_lossy_owned)]
#![feature(trim_prefix_suffix)]
#![feature(iter_array_chunks)]

use std::{
    ffi::OsStr,
    fmt::{self, Arguments, Write},
    io::{self, BufRead, BufReader, stdout},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use portable_pty::{ChildKiller, CommandBuilder, ExitStatus, NativePtySystem, PtySystem};
use ratatui::{
    DefaultTerminal, Frame, Terminal, TerminalOptions, Viewport,
    crossterm::event::{self, Event, KeyCode, KeyModifiers},
    layout::{Constraint, Flex, Layout},
    prelude::CrosstermBackend,
    style::{Style, Stylize},
    text::{Line, ToLine},
    widgets::{Block, BorderType, Padding},
};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    config::ConfigRoot,
    widget::{
        CenteredParagraph,
        config_area::{ConfigArea, ConfigAreaState},
        prompt::{CommandStatus, Promt, PromtState},
    },
};

mod build;
mod config;
mod widget;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Tui failed, with error `{error}`")]
    Tui { error: io::Error },
    #[error("Failed to create dir, with error {error}")]
    BuildDirFailed { error: io::Error },
    #[error("Run command, failed with error {error}")]
    RunCommand { error: io::Error },
    #[error("Build error, failed with error {error}")]
    Build {
        #[from]
        error: build::Error,
    },
}

enum AppEvent {
    Ui(Event, Duration),
    Render(Duration),
    Output(String),
    RunningCmd(Option<String>),
}

pub struct App {
    prompt: PromtState,
    event: Receiver<AppEvent>,
    event_sender: Sender<AppEvent>,

    running_cmd_name: Option<String>,
    output_collected: Vec<String>,

    last_command: Option<String>,
    cmd_handle: Option<JoinHandle<Result<(), Error>>>,
    build_error: Option<Error>,

    previous_render_start: Instant,
    main_screen: MainScreen,

    config: Arc<ConfigRoot>,
    config_area: ConfigAreaState,
}

#[derive(Default)]
enum MainScreen {
    #[default]
    None,
    Config,
    Help,
    Error(String),
}

impl App {
    pub fn new() -> Self {
        let (event_sender, event) = channel();
        let config = config::load();

        Self {
            prompt: PromtState::default(),
            running_cmd_name: None,
            event,
            event_sender,
            cmd_handle: None,
            output_collected: Default::default(),
            build_error: None,
            last_command: None,
            previous_render_start: Instant::now(),
            main_screen: MainScreen::None,
            config_area: ConfigAreaState {
                config_staging: config.clone().into(),
                config: config.clone().into(),
                ..Default::default()
            },
            config: config.into(),
        }
    }

    pub fn run(mut self, from_rebuild: bool, mut main_terminal: DefaultTerminal) -> Result<(), Error> {
        let backend = CrosstermBackend::new(stdout());
        let mut repl_terminal =
            Terminal::with_options(backend, TerminalOptions { viewport: Viewport::Inline(4), ..Default::default() })
                .map_err(|error| Error::Tui { error })?;
        repl_terminal
            .insert_before(main_terminal.size().map_err(|error| Error::Tui { error })?.height, |_frame| {})
            .map_err(|error| Error::Tui { error })?;

        if from_rebuild {
            self.eval("build".to_string());
        }

        let mut frame_start;
        let mut last_start = Instant::now();
        let mut event = AppEvent::Render(Duration::from_millis(1));
        loop {
            frame_start = Instant::now();
            let delta = last_start.elapsed();
            self.handle_event(&mut repl_terminal, &mut main_terminal, event);
            event = self.poll_event_timeout(Duration::from_millis(1).saturating_sub(frame_start.elapsed()), delta)?;
            last_start = frame_start;
        }
    }

    /// Polls for event until timeout
    fn poll_event_timeout(&mut self, timeout: Duration, delta_time: Duration) -> Result<AppEvent, Error> {
        let start = Instant::now();

        loop {
            if let Ok(ev) = self.event.try_recv() {
                return Ok(ev);
            }

            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return Ok(AppEvent::Render(delta_time));
            }

            let remaining = timeout - elapsed;

            if event::poll(remaining).map_err(|error| Error::Tui { error })? {
                let ev = event::read().map_err(|error| Error::Tui { error })?;
                return Ok(AppEvent::Ui(ev, delta_time));
            }
        }
    }

    fn handle_event(
        &mut self,
        repl_terminal: &mut DefaultTerminal,
        main_terminal: &mut DefaultTerminal,
        event: AppEvent,
    ) -> Result<(), Error> {
        match event {
            AppEvent::Ui(event, delta_time) => self.handle_ui_event(repl_terminal, main_terminal, event, delta_time)?,
            AppEvent::Render(delta_time) => {
                self.draw(repl_terminal, main_terminal, delta_time)?;
            }
            AppEvent::RunningCmd(new_running_cmd) => {
                self.running_cmd_name = new_running_cmd;
            }
            AppEvent::Output(line) => {
                self.output_collected.push(line.clone());
                Self::insert_before_lines(&line, repl_terminal)?;
            }
        };
        Ok(())
    }

    fn handle_ui_event(
        &mut self,
        repl_terminal: &mut DefaultTerminal,
        main_terminal: &mut DefaultTerminal,
        event: Event,
        delta_time: Duration,
    ) -> Result<(), Error> {
        match event {
            // We don't do release event
            Event::Key(event) if event.is_release() => return Ok(()),
            Event::Key(key) if matches!(self.main_screen, MainScreen::Config) => {
                if let Some(new_config_root) = self.config_area.key_event(key) {
                    self.main_screen = MainScreen::None;
                    self.last_command = None;
                    self.config = Arc::new(new_config_root);
                    if let Err(err) = config::save(&self.config) {
                        self.main_screen = MainScreen::Error(format!("{err}"));
                    }
                    self.redraw(repl_terminal, main_terminal, delta_time)?;
                }
            }
            Event::Key(_) if matches!(self.main_screen, MainScreen::Error(_) | MainScreen::Help) => {
                self.main_screen = MainScreen::None;
                self.last_command = None;
                self.redraw(repl_terminal, main_terminal, delta_time)?;
            }
            Event::Key(key) => {
                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        todo!();
                    }
                    _ => {}
                }

                if self.cmd_handle.as_ref().is_some_and(|cmd| !cmd.is_finished()) {
                    return Ok(());
                }

                let command = match self.prompt.key_event(key) {
                    Some(command) => command,
                    None => return Ok(()),
                };

                self.eval(command);
                repl_terminal.clear().map_err(|error| Error::Tui { error })?;
            }
            Event::Resize(_col, row) => {
                main_terminal.autoresize().map_err(|error| Error::Tui { error })?;
                repl_terminal.autoresize().map_err(|error| Error::Tui { error })?;
                repl_terminal.insert_before(row, |buf| buf.reset()).unwrap();
                self.redraw(repl_terminal, main_terminal, delta_time)?;
            }
            _ => {}
        };
        Ok(())
    }

    fn eval(&mut self, command: String) {
        let executor = self.executor.clone();

        match command.as_str() {
            "config" | "c" => {
                self.main_screen = MainScreen::Config;
            }
            "build" | "b" if self.cmd_handle.is_none() => {
                self.build_error = None;
                let config = Arc::clone(&self.config);
                self.cmd_handle = Some(thread::spawn(move || {
                    build::build(&mut executor.lock().unwrap(), build::BuildConfig { config })?;
                    Ok(())
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

    fn redraw_child_output(&mut self, repl_terminal: &mut DefaultTerminal) -> Result<(), Error> {
        for line in self.output_collected.iter() {
            Self::insert_before_lines(line, repl_terminal)?;
        }
        Ok(())
    }

    fn redraw(
        &mut self,
        repl_terminal: &mut DefaultTerminal,
        main_terminal: &mut DefaultTerminal,
        delta_time: Duration,
    ) -> Result<(), Error> {
        main_terminal.clear().map_err(|error| Error::Tui { error })?;
        repl_terminal.clear().map_err(|error| Error::Tui { error })?;

        self.redraw_child_output(repl_terminal)?;
        self.draw(repl_terminal, main_terminal, delta_time)?;

        Ok(())
    }

    fn draw(
        &mut self,
        repl_terminal: &mut DefaultTerminal,
        main_terminal: &mut DefaultTerminal,
        delta_time: Duration,
    ) -> Result<(), Error> {
        repl_terminal.draw(|frame| self.draw_repl(frame, delta_time)).map_err(|error| Error::Tui { error })?;

        match self.main_screen {
            MainScreen::Config => {
                main_terminal
                    .draw(|frame| self.draw_config(frame, delta_time))
                    .map_err(|error| Error::Tui { error })?;
            }
            MainScreen::Help => {
                main_terminal.draw(|frame| self.draw_help(frame)).map_err(|error| Error::Tui { error })?;
            }
            MainScreen::Error(ref error) => {
                let error = error.clone();
                main_terminal.draw(|frame| self.draw_error(frame, error)).map_err(|error| Error::Tui { error })?;
            }
            MainScreen::None => {}
        };

        Ok(())
    }

    fn draw_config(&mut self, frame: &mut Frame, delta_time: Duration) {
        let vertical = Layout::vertical([Constraint::Percentage(80)]).flex(Flex::Center);
        let horizontal = Layout::horizontal([Constraint::Percentage(60)]).flex(Flex::Center);
        let [area] = vertical.areas(frame.area());
        let [area] = horizontal.areas(area);

        frame.render_stateful_widget(ConfigArea { delta_time }, area, &mut self.config_area);
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
            Line::from(
                "  `c` or `config` to configure the kernel (not required if you want to just build the project)",
            ),
            Line::from("Also the controls are:"),
            Line::from(
                "  Press `CTRL-C` to quit the build tool, if the build is running this will terminate the build build",
            ),
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

    fn draw_repl(&mut self, frame: &mut Frame, delta_time: Duration) {
        let [status, prompt] = Layout::vertical([Constraint::Length(1), Constraint::Length(3)]).areas(frame.area());

        let command_status = if self.cmd_handle.is_some() || !matches!(self.main_screen, MainScreen::None) {
            CommandStatus::Busy
        } else if self.build_error.is_some() {
            CommandStatus::Errored
        } else {
            CommandStatus::Idle
        };
        let display = self.build_error.as_ref().map(|e| e.to_string());
        let display = display.unwrap_or_else(|| self.running_cmd_name.as_deref().unwrap_or("Idle").to_string());
        let mut display = display.to_line().centered().style(Style::default().bold());
        if matches!(command_status, CommandStatus::Errored) {
            display = display.red();
        }
        frame.render_widget(display, status);
        frame.render_stateful_widget(
            Promt {
                running_cmd: self.last_command.as_deref().unwrap_or(""),
                delta_time,
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

    fn insert_before_lines(line: &str, repl_terminal: &mut DefaultTerminal) -> Result<(), Error> {
        for mut line in line.split("\n") {
            while !line.is_empty() {
                let mut current_width = 0;
                let mut split_idx = 0;

                for (byte_idx, grapheme) in line.grapheme_indices(true) {
                    let w = UnicodeWidthStr::width(grapheme);
                    if current_width + w > repl_terminal.get_frame().area().width.into() {
                        break;
                    }
                    current_width += w;
                    split_idx = byte_idx + grapheme.len();
                }

                if split_idx == 0 {
                    // fallback: at least consume something to avoid infinite loops
                    split_idx = line.len();
                }

                let chunk = &line[..split_idx];

                repl_terminal
                    .insert_before(1, |buf| {
                        buf[(0, 0)].set_symbol(chunk);
                    })
                    .map_err(|error| Error::Tui { error })?;

                line = &line[split_idx..];
            }
        }
        Ok(())
    }
}
