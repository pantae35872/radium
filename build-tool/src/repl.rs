use std::{
    fs::remove_dir_all,
    io,
    iter::Peekable,
    sync::Arc,
    thread::{self, JoinHandle},
};

use thiserror::Error;

use crate::{
    App, AppScreen,
    build::{self, build_path},
    config::{self, Config, ConfigRoot},
    result_err_ext,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Config overwrite failed, failed with `{error}`")]
    ConfigOverwrite {
        #[from]
        error: config::Error,
    },
    #[error("Io error, failed with `{error}`")]
    GenericIo { error: io::Error },
    #[error("Overwriting the `{0}` config requires value argument")]
    ConfigNoValue(String),
    #[error("Unknown command `{0}` try `help` for more info")]
    UnknownCommand(String),
    #[error("Invalid command `{0}` try `help` for more info")]
    InvalidCommand(String),
    #[error("Build error, failed with error `{error}`")]
    Build {
        #[from]
        error: build::Error,
    },
}

result_err_ext!(io_err, std::io::Error, Error::GenericIo);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Config,
    Clean,
    Help,
    Build,
}

#[derive(Debug)]
pub struct ExecutionToken {
    command: String,
    handle: Option<JoinHandle<Result<(), Error>>>,
}

impl ExecutionToken {
    pub fn is_finished(&self) -> bool {
        self.handle.is_none()
    }

    pub fn poll(&mut self) -> Option<Result<(), Error>> {
        if self.handle.as_ref().is_some_and(|handle| handle.is_finished()) {
            return Some(self.handle.take().unwrap().join().expect("Builder panicked"));
        }
        None
    }

    pub fn original_command(&self) -> &str {
        &self.command
    }
}

pub fn eval(app: &mut App, command: &str) -> Result<Option<ExecutionToken>, Error> {
    let mut temporary_config = app.config.as_ref().clone();
    let Some(parsed_command) = parse_command(command, &mut temporary_config)? else {
        return Ok(None);
    };
    run_command(app, parsed_command, command.to_string(), Arc::new(temporary_config))
}

// build -build_mode release -qemu.run false

fn parse_command(command: &str, config: &mut ConfigRoot) -> Result<Option<Command>, Error> {
    let mut tokens = command.split_whitespace().peekable();
    let parsed_command = match tokens.next() {
        Some("config" | "c") => Command::Config,
        Some("build" | "b") => Command::Build,
        Some("help" | "h") => Command::Help,
        Some("clean") => Command::Clean,
        Some(unknown) => return Err(Error::UnknownCommand(unknown.to_string())),
        None => return Ok(None),
    };
    parse_config_modifier(&mut tokens, config)?;
    if tokens.next().is_some() {
        return Err(Error::InvalidCommand(command.to_string()));
    }

    Ok(Some(parsed_command))
}

fn run_command(
    app: &mut App,
    command: Command,
    original_command: String,
    config: Arc<ConfigRoot>,
) -> Result<Option<ExecutionToken>, Error> {
    let mut handle = None;
    match command {
        Command::Help => {
            app.current_screen = AppScreen::Help;
        }
        Command::Build => {
            app.build_error = None;
            let event = app.event_sender.clone();
            let command = original_command.clone();
            handle = Some(thread::spawn(move || {
                build::build(event, build::BuildConfig { config }, command)?;
                Ok(())
            }));
        }
        Command::Config => {
            app.current_screen = AppScreen::Config;
        }
        Command::Clean => {
            remove_dir_all(build_path()?).io_err()?;
        }
    };
    Ok(Some(ExecutionToken { command: original_command, handle }))
}

fn parse_config_modifier<'a>(
    tokens: &mut Peekable<impl Iterator<Item = &'a str>>,
    config: &mut ConfigRoot,
) -> Result<(), Error> {
    while let Some(config_tokens) = tokens.peek()
        && config_tokens.chars().next().is_some_and(|c| c == '-')
    {
        let config_tokens = config_tokens.to_string();
        tokens.next();
        let value = match tokens.next() {
            Some(config) => config,
            None => return Err(Error::ConfigNoValue(config_tokens)),
        };

        config.modifier_config(config_tokens[1..].split("."), value)?;
    }

    Ok(())
}
