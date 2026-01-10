use std::{
    env,
    ffi::OsStr,
    fs::{OpenOptions, create_dir, remove_file},
    io::{self, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use portable_pty::{CommandBuilder, ExitStatus};
use thiserror::Error;

use crate::{
    CmdExecutor,
    build::{
        cargo_project::CargoProject,
        iso::{
            Iso, PrimaryVolumeDescriptor, TerminatorDescriptor,
            writer::{DateTime, IsoStrA, IsoStrD},
        },
    },
    config::{BuildMode, Config, ConfigRoot},
};

mod cargo_project;
mod iso;

pub fn build(executor: &mut CmdExecutor, config: BuildConfig) -> Result<(), Error> {
    let current_dir = project_dir()?;
    Builder {
        config,
        executor,
        root_path: current_dir.clone(),
        build_path: current_dir.join("build"),
        src_path: current_dir.join("src"),
        userland_path: current_dir.join("userland"),
    }
    .build()
}

pub fn project_dir() -> Result<PathBuf, Error> {
    let mut current_dir = env::current_dir().map_err(|error| Error::CurrentDir { error })?;

    while !current_dir.file_name().is_some_and(|e| e == OsStr::new("radium")) {
        current_dir = current_dir.parent().expect("Failed to get to the project root!").to_path_buf();
    }

    Ok(current_dir)
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to re-execute the build tool, failed with: `{error}`")]
    ReExecFailed { error: io::Error },
    #[error("Failed to get current dir, this must be exected in the project root, failed with: `{error}`")]
    CurrentDir { error: io::Error },
    #[error("Failed to create dir failed with error: {error}")]
    CreateDir { error: io::Error },
    #[error("Failed to generate config.rs file, failed with error: {error}")]
    GenConfig { error: io::Error },
    #[error("Failed to generate iso file, failed with error: {error}")]
    GenIso { error: io::Error },
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
    root_path: PathBuf,
}

impl Builder<'_> {
    fn userland(&self, name: &str) -> PathBuf {
        self.userland_path.join(name)
    }

    fn src(&self, name: &str) -> PathBuf {
        self.src_path.join(name)
    }

    fn build(mut self) -> Result<(), Error> {
        make_build_dir()?;
        self.gen_config()?;

        let build_tool = self.project(&self.root_path.join("build-tool")).build()?;
        let kernel = self.project(&self.src("kernel")).build()?;
        let bootloader = self.project(&self.src("bootloader")).build()?;
        let init = self.project(&self.userland("init")).build()?;
        assert!(kernel.exists() && bootloader.exists() && init.exists() && build_tool.exists());
        self.gen_iso()?;

        if self.config.reexec_build_tool {
            ratatui::restore();
            return Err(Error::ReExecFailed { error: Command::new(build_tool).exec() });
        }

        Ok(())
    }

    fn gen_iso(&self) -> Result<(), Error> {
        let mut iso = Iso::new();
        iso.add_descriptor(PrimaryVolumeDescriptor {
            system_identifier: IsoStrA::new(""),
            volume_identifier: IsoStrD::new("Radium disk image"),
            volume_space_size: 16,
            volume_set_size: 1,
            volume_sequence_number: 0,
            logical_block_size: 2048,
            path_table_size: 0,
            l_lba_path_table_location: 0,
            l_lba_optional_path_table_location: 0,
            m_lba_path_table_location: 0,
            m_lba_optional_path_table_location: 0,
            root_directory_entry: [0; _],
            volume_set_identifier: IsoStrD::new("A"),
            publisher_identifier: IsoStrA::new(""),
            data_preparer_identifier: IsoStrA::new("I WROTE MY OWN"),
            application_identifier: IsoStrA::new(""),
            copyright_file_identifier: IsoStrD::new(""),
            abstract_file_identifier: IsoStrD::new(""),
            bibliographic_file_identifier: IsoStrD::new(""),
            volume_creation_date: DateTime::now(),
            volume_modification: DateTime::now(),
            volume_expiration_date: DateTime::empty(),
            volume_effective_date: DateTime::empty(),
            application_used: [0; _],
        });
        iso.add_descriptor(TerminatorDescriptor);
        let iso_bytes = iso.build();

        let out_iso = self.build_path.join("radium.iso");
        if out_iso.exists() {
            remove_file(&out_iso).map_err(|error| Error::GenConfig { error })?;
        }
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(out_iso)
            .and_then(|mut f| f.write_all(&iso_bytes))
            .map_err(|error| Error::GenIso { error })?;

        Ok(())
    }

    fn gen_config(&self) -> Result<(), Error> {
        let config = format!(
            r#"
// GENERATED FILE. DO NOT EDIT, USE `config` COMMAND IN THE BUILD TOOL REPL TO CONFIG.

pub const CONFIG: ConfigRoot = {};

pub const fn config() -> ConfigRoot {{
    return CONFIG;
}}
{}
"#,
            self.config.config.into_const_rust(),
            self.config.config.into_const_rust_types()
        );
        let config_rs = self.build_path.join("config.rs");
        if config_rs.exists() {
            remove_file(config_rs).map_err(|error| Error::GenConfig { error })?;
        }
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.build_path.join("config.rs"))
            .and_then(|mut f| f.write_all(config.trim().as_bytes()))
            .map_err(|error| Error::GenConfig { error })?;

        Ok(())
    }

    fn project<'a>(&'a mut self, path: &'a Path) -> CargoProject<'a> {
        CargoProject::new(path, &self.build_path, &self.config, &mut self.executor)
    }
}

#[derive(Debug)]
pub struct BuildConfig {
    pub config: Arc<ConfigRoot>,
    pub reexec_build_tool: bool,
}

impl BuildConfig {
    fn into_command(&self, builder: &mut CommandBuilder) {
        self.config.build_mode.into_command(builder);
    }
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

pub fn build_path() -> Result<PathBuf, Error> {
    Ok(project_dir()?.join("build"))
}

pub fn make_build_dir() -> Result<PathBuf, Error> {
    let build_path = build_path()?;
    if !build_path.exists() {
        create_dir(&build_path).map_err(|error| Error::CreateDir { error })?;
    }

    let mut gitignore = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(build_path.join(".gitignore"))
        .map_err(|error| Error::CreateDir { error })?;
    gitignore.write_all("*".as_bytes()).map_err(|error| Error::CreateDir { error })?;
    gitignore.flush().map_err(|error| Error::CreateDir { error })?;

    Ok(build_path)
}
