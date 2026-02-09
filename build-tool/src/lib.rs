#![feature(if_let_guard)]
#![feature(vec_push_within_capacity)]
#![feature(string_from_utf8_lossy_owned)]
#![feature(trim_prefix_suffix)]
#![feature(iter_array_chunks)]
#![allow(dead_code)]

use std::{
    collections::VecDeque,
    fmt::{self, Arguments},
    fs::remove_dir_all,
    io::{self, Write},
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use portable_pty::ChildKiller;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::{
        QueueableCommand,
        cursor::{RestorePosition, SavePosition},
        event::{self, Event, KeyCode, KeyModifiers, MouseEvent},
    },
    layout::{Constraint, Flex, Layout},
    prelude::Backend,
    style::{Style, Stylize},
    text::{Line, ToLine},
    widgets::{Block, BorderType, Padding},
};
use thiserror::Error;
use vt100::{Parser, Screen};

use crate::{
    build::build_path,
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
    #[error("Failed to create dir, with error `{error}`")]
    BuildDirFailed { error: io::Error },
    #[error("Run command, failed with error `{error}`")]
    RunCommand { error: io::Error },
    #[error("Io error, failed with `{error}`")]
    GenericIo { error: io::Error },
    #[error("Build error, failed with error `{error}`")]
    Build {
        #[from]
        error: build::Error,
    },
}

#[macro_export]
macro_rules! result_err_ext {
    (
        $method:ident,
        $err_ty:ty,
        $variant:path
    ) => {
        paste::paste! {
            pub trait [<$method:camel ResultExt>]<T> {
                fn $method(self) -> Result<T, Error>;
            }

            impl<T> [<$method:camel ResultExt>]<T> for Result<T, $err_ty> {
                #[inline]
                fn $method(self) -> Result<T, Error> {
                    self.map_err(|error| $variant { error })
                }
            }
        }
    };
}

result_err_ext!(tui_err, std::io::Error, Error::Tui);
result_err_ext!(io_err, std::io::Error, Error::GenericIo);

#[derive(Debug)]
enum AppEvent {
    Ui(Event),
    Render(Duration),
    Output(Vec<u8>),
    RunningCmd(String),
    ChildProcessStarted(Box<dyn ChildKiller + Send + Sync>),
    CmdStopped,
    Terminated,
}

pub struct App {
    prompt: PromtState,
    event: Receiver<AppEvent>,
    event_sender: Sender<AppEvent>,

    running_cmd_name: Option<String>,
    child_process_killer: Option<Box<dyn ChildKiller + Send + Sync>>,

    last_command: Option<String>,
    cmd_handle: Option<JoinHandle<Result<(), Error>>>,
    build_error: Option<Error>,
    prev_output_screen: Option<Screen>,

    current_screen: AppScreen,

    config: Arc<ConfigRoot>,
    config_area: ConfigAreaState,

    collected_output: VecDeque<u8>,
    parser: Parser,
}

#[derive(Default)]
enum AppScreen {
    #[default]
    None,
    Config,
    Help,
    Error(String),
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
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
            build_error: None,
            last_command: None,
            prev_output_screen: None,
            child_process_killer: None,
            current_screen: AppScreen::None,
            config_area: ConfigAreaState {
                config_staging: config.clone().into(),
                config: config.clone().into(),
                ..Default::default()
            },
            collected_output: VecDeque::with_capacity(config.build_tool.max_scrollback_size as usize),
            config: config.into(),
            parser: Parser::new(100, 100, 0),
        }
    }

    pub fn run(mut self, from_rebuild: bool, mut main_terminal: DefaultTerminal) -> Result<(), Error> {
        self.parser = vt100::Parser::new(
            main_terminal.get_frame().area().height - 4,
            main_terminal.get_frame().area().width,
            self.config.build_tool.max_scrollback_size as usize,
        );
        for _ in 0..self.parser.screen().size().0 {
            self.collected_output.extend("\r\n".as_bytes().iter().copied());
            self.parser.process("\r\n".as_bytes());
        }

        if from_rebuild {
            self.eval("build".to_string())?;
        }

        let mut frame_start;
        let mut last_start = Instant::now();
        let mut events = Vec::new();
        loop {
            frame_start = Instant::now();
            let delta = last_start.elapsed();

            for event in events {
                let should_terminated = matches!(event, AppEvent::Terminated);

                match self.handle_event(&mut main_terminal, event) {
                    Ok(_) => {}
                    Err(err) => {
                        self.current_screen = AppScreen::Error(format!("`{err}`"));
                        break;
                    }
                }

                if should_terminated {
                    return Ok(());
                }
            }

            events = self.poll_event_timeout(Duration::from_millis(16).saturating_sub(frame_start.elapsed()), delta)?;
            last_start = frame_start;
        }
    }

    /// Polls for event until timeout
    fn poll_event_timeout(&mut self, timeout: Duration, delta_time: Duration) -> Result<Vec<AppEvent>, Error> {
        let start = Instant::now();
        let mut events = Vec::new();

        while let Ok(ev) = self.event.try_recv() {
            events.push(ev);
        }

        if self.cmd_handle.as_ref().is_some_and(|handle| handle.is_finished()) {
            self.build_error = self.cmd_handle.take().unwrap().join().expect("Builder panicked").err();
            self.cmd_handle = None;
            self.last_command = None;
        }

        if event::poll(timeout.saturating_sub(start.elapsed())).tui_err()? {
            events.push(AppEvent::Ui(event::read().tui_err()?));
        }

        events.push(AppEvent::Render(delta_time));
        Ok(events)
    }

    fn handle_event(&mut self, main_terminal: &mut DefaultTerminal, event: AppEvent) -> Result<(), Error> {
        match event {
            AppEvent::Ui(event) => self.handle_ui_event(main_terminal, event)?,
            AppEvent::Render(delta_time) => {
                self.draw(main_terminal, delta_time)?;
            }
            AppEvent::RunningCmd(new_running_cmd) => {
                self.running_cmd_name = Some(new_running_cmd);
            }
            AppEvent::ChildProcessStarted(killer) => {
                self.child_process_killer = Some(killer);
            }
            AppEvent::CmdStopped => {
                self.running_cmd_name = None;
            }
            AppEvent::Output(line) => {
                self.collected_output.extend(line.iter().copied());
                while self.collected_output.len()
                    > self.config.build_tool.max_scrollback_size as usize
                        * main_terminal.get_frame().area().width as usize
                {
                    self.collected_output.pop_front();
                }
                self.parser.process(&line);
            }
            AppEvent::Terminated => {}
        };
        Ok(())
    }

    fn draw_output(&mut self, main_terminal: &mut DefaultTerminal) -> Result<(), Error> {
        let screen = self.parser.screen();
        let main_area = main_terminal.get_frame().area();
        let backend = main_terminal.backend_mut();
        backend.hide_cursor().tui_err()?;
        backend.queue(SavePosition).tui_err()?;
        let rows: Vec<Vec<u8>> = match &self.prev_output_screen {
            Some(prev) => screen.rows_diff(prev, 0, main_area.width).collect(),
            None => screen.rows_formatted(0, main_area.width).collect(),
        };
        for (i, row) in rows.iter().enumerate() {
            backend.set_cursor_position((0, i as u16)).tui_err()?;
            backend.write_all(row).tui_err()?;
        }
        self.prev_output_screen = Some(screen.clone());
        backend.queue(RestorePosition).tui_err()?;
        backend.show_cursor().tui_err()?;
        Ok(())
    }

    fn handle_ui_event(&mut self, main_terminal: &mut DefaultTerminal, event: Event) -> Result<(), Error> {
        match event {
            // We don't do release event
            Event::Key(event) if event.is_release() => return Ok(()),
            Event::Mouse(MouseEvent { kind: event::MouseEventKind::ScrollUp, .. }) => {
                let scroll_back = self.parser.screen().scrollback() + 1;
                self.parser.screen_mut().set_scrollback(scroll_back);
            }
            Event::Mouse(MouseEvent { kind: event::MouseEventKind::ScrollDown, .. }) => {
                let scroll_back = self.parser.screen().scrollback().saturating_sub(1);
                self.parser.screen_mut().set_scrollback(scroll_back);
            }
            Event::Key(key) if matches!(self.current_screen, AppScreen::Config) => {
                if let Some(new_config_root) = self.config_area.key_event(key) {
                    self.current_screen = AppScreen::None;
                    self.last_command = None;
                    self.config = Arc::new(new_config_root);
                    if let Err(err) = config::save(&self.config) {
                        self.current_screen = AppScreen::Error(format!("{err}"));
                    }

                    self.scheduled_redraw(main_terminal)?;
                }
            }
            Event::Key(_) if matches!(self.current_screen, AppScreen::Error(_) | AppScreen::Help) => {
                self.current_screen = AppScreen::None;
                self.last_command = None;

                self.scheduled_redraw(main_terminal)?;
            }
            Event::Key(key) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let (Some(_handle), Some(killer)) =
                        (self.cmd_handle.as_ref(), self.child_process_killer.as_mut())
                    {
                        let _ = killer.kill();
                    } else {
                        let _ = self.event_sender.send(AppEvent::Terminated);
                    }
                }
                KeyCode::PageUp => {
                    let scroll_back =
                        self.parser.screen().scrollback() + main_terminal.get_frame().area().height as usize;
                    self.parser.screen_mut().set_scrollback(scroll_back);
                }
                KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let scroll_back = self.parser.screen().scrollback() + 1;
                    self.parser.screen_mut().set_scrollback(scroll_back);
                }
                KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let scroll_back = self.parser.screen().scrollback().saturating_sub(1);
                    self.parser.screen_mut().set_scrollback(scroll_back);
                }
                KeyCode::PageDown => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.parser.screen_mut().set_scrollback(0);
                        return Ok(());
                    }

                    let scroll_back = self
                        .parser
                        .screen()
                        .scrollback()
                        .saturating_sub(main_terminal.get_frame().area().height as usize);
                    self.parser.screen_mut().set_scrollback(scroll_back);
                }
                _ => {
                    if self.cmd_handle.as_ref().is_some_and(|cmd| !cmd.is_finished()) {
                        return Ok(());
                    }

                    let command = match self.prompt.key_event(key) {
                        Some(command) => command,
                        None => return Ok(()),
                    };
                    self.eval(command)?;
                }
            },
            Event::Resize(_, _) => {
                main_terminal.autoresize().tui_err()?;
                self.parser = vt100::Parser::new(
                    main_terminal.get_frame().area().height - 4,
                    main_terminal.get_frame().area().width,
                    self.config.build_tool.max_scrollback_size as usize,
                );
                self.parser.process(self.collected_output.make_contiguous());
                self.scheduled_redraw(main_terminal)?;
            }
            _ => {}
        };
        Ok(())
    }

    fn eval(&mut self, command: String) -> Result<(), Error> {
        match command.as_str() {
            "config" | "c" => {
                self.current_screen = AppScreen::Config;
            }
            "clean" => {
                remove_dir_all(build_path()?).io_err()?;
            }
            "build" | "b" if self.cmd_handle.is_none() => {
                self.build_error = None;
                let config = Arc::clone(&self.config);
                let event = self.event_sender.clone();
                self.cmd_handle = Some(thread::spawn(move || {
                    build::build(event, build::BuildConfig { config })?;
                    Ok(())
                }));
            }
            "help" | "h" => {
                self.current_screen = AppScreen::Help;
            }
            "" => {}
            cmd => {
                self.current_screen = AppScreen::Error(format!("Unknown command `{cmd}` type `help` for more info."));
            }
        };

        self.last_command = Some(command);
        Ok(())
    }

    fn scheduled_redraw(&mut self, main_terminal: &mut DefaultTerminal) -> Result<(), Error> {
        self.prev_output_screen = None;

        main_terminal.clear().tui_err()?;

        Ok(())
    }

    fn draw(&mut self, main_terminal: &mut DefaultTerminal, delta_time: Duration) -> Result<(), Error> {
        self.draw_output(main_terminal)?;

        main_terminal
            .draw(|frame| match self.current_screen {
                AppScreen::Config => {
                    self.draw_repl(frame, delta_time);
                    self.draw_config(frame, delta_time);
                }
                AppScreen::Help => {
                    self.draw_repl(frame, delta_time);
                    self.draw_help(frame);
                }
                AppScreen::Error(ref error) => {
                    let error = error.clone();
                    self.draw_repl(frame, delta_time);
                    self.draw_error(frame, error);
                }
                AppScreen::None => {
                    self.draw_repl(frame, delta_time);
                }
            })
            .tui_err()?;
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
            Line::from("  Press `PAGE-UP` `PAGE-DOWN` ↑↓ to scroll up and down, CTRL+PAGE-DOWN to go back down"),
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
        let [_, status, prompt] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1), Constraint::Length(3)]).areas(frame.area());

        let command_status = if self.cmd_handle.is_some() || !matches!(self.current_screen, AppScreen::None) {
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

        if matches!(self.current_screen, AppScreen::None) {
            self.prompt.set_cursor_pos(prompt, frame);
        }
    }
}

#[derive(Debug, Clone)]
struct AppFormatter(pub Sender<AppEvent>);

impl From<&Sender<AppEvent>> for AppFormatter {
    fn from(value: &Sender<AppEvent>) -> Self {
        Self(value.clone())
    }
}

impl fmt::Write for AppFormatter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.send(AppEvent::Output(s.to_string().as_bytes().to_vec())).map_err(|_| fmt::Error)?;
        Ok(())
    }
}

impl AppFormatter {
    pub fn write_fmt(&mut self, args: Arguments) -> std::fmt::Result {
        fmt::Write::write_fmt(self, args)
    }
}
