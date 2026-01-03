use std::{
    env,
    fs::{OpenOptions, create_dir},
    io::{self, Write},
    path::{Path, PathBuf, absolute},
};

use portable_pty::{CommandBuilder, ExitStatus};
use thiserror::Error;

use crate::{CmdExecutor, build::cargo_project::CargoProject};

mod cargo_project;

pub fn build(executor: &mut CmdExecutor, config: BuildConfig) -> Result<(), Error> {
    let current_dir = env::current_dir().map_err(|error| Error::CurrentDir { error })?;
    Builder {
        config,
        executor,
        build_path: absolute(Path::new("build")).map_err(|error| Error::CurrentDir { error })?,
        src_path: current_dir.join("src"),
        userland_path: current_dir.join("userland"),
    }
    .build()
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to get current dir, this must be exected in the project root, failed with: `{error}`")]
    CurrentDir { error: io::Error },
    #[error("Failed to create dir failed with error: {error}")]
    CreateDir { error: io::Error },
    #[error("Failed to execute command with io error, {error}")]
    CommandIoError { error: io::Error },
    #[error("Command `{command}` in dir `{dir}`, failed with exit status {status}")]
    CommandFailed { command: String, dir: String, status: ExitStatus },
}

#[derive(Debug)]
struct Builder<'a> {
    executor: &'a mut CmdExecutor,

    config: BuildConfig,
    src_path: PathBuf,
    userland_path: PathBuf,
    build_path: PathBuf,
}

impl Builder<'_> {
    fn userland(&self, name: &str) -> PathBuf {
        self.userland_path.join(name)
    }

    fn src(&self, name: &str) -> PathBuf {
        self.src_path.join(name)
    }

    fn build(mut self) -> Result<(), Error> {
        self.make_build_dir()?;

        let kernel = self.project(&self.src("kernel")).build()?;
        let bootloader = self.project(&self.src("bootloader")).build()?;
        let init = self.project(&self.userland("init")).build()?;
        assert!(kernel.exists() && bootloader.exists() && init.exists());

        Ok(())
    }

    fn project<'a>(&'a mut self, path: &'a Path) -> CargoProject<'a> {
        CargoProject::new(path, &self.build_path, &self.config, &mut self.executor)
    }

    fn make_build_dir(&self) -> Result<(), Error> {
        if !self.build_path.exists() {
            create_dir(&self.build_path).map_err(|error| Error::CreateDir { error })?;
        }

        let mut gitignore = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.build_path.join(".gitignore"))
            .map_err(|error| Error::CreateDir { error })?;
        gitignore.write_all("*".as_bytes()).map_err(|error| Error::CreateDir { error })?;
        gitignore.flush().map_err(|error| Error::CreateDir { error })?;

        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct BuildConfig {
    pub mode: BuildMode,
}

impl BuildConfig {
    fn into_command(&self, builder: &mut CommandBuilder) {
        self.mode.into_command(builder);
    }
}

#[derive(Default, Debug)]
pub enum BuildMode {
    #[default]
    Debug,
    Release,
}

impl BuildMode {
    pub fn into_command(&self, builder: &mut CommandBuilder) {
        match self {
            Self::Release => builder.arg("--release"),
            Self::Debug => {}
        }
    }

    pub fn dir_name(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}
