use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
};

use portable_pty::CommandBuilder;
use toml::Table;

use crate::{CmdExecutor, build::BuildConfig};

#[derive(Debug)]
pub struct CargoProject<'a> {
    path: &'a Path,
    build_path: &'a Path,
    config: &'a BuildConfig,
    executor: &'a mut CmdExecutor,
}

impl<'a> CargoProject<'a> {
    pub fn new(path: &'a Path, build_path: &'a Path, config: &'a BuildConfig, executor: &'a mut CmdExecutor) -> Self {
        Self { path, build_path, config, executor }
    }

    /// Get the package name specified in Cargo.toml
    pub fn package_name(&self) -> Option<String> {
        let Ok(config) = read_to_string(self.path.join("Cargo.toml")) else {
            return None;
        };
        let Ok(toml) = config.parse::<Table>() else {
            return None;
        };

        let Some(name) = toml.get("package").and_then(|build| build.get("name")).and_then(|target| target.as_str())
        else {
            return None;
        };

        Some(name.to_string())
    }

    /// Get the target name specified in .cargo/config.toml
    pub fn target_name(&self) -> Option<String> {
        let Ok(config) = read_to_string(self.path.join(".cargo").join("config.toml")) else {
            return None;
        };
        let Ok(toml) = config.parse::<Table>() else {
            return None;
        };
        let Some(target) = toml.get("build").and_then(|build| build.get("target")).and_then(|target| target.as_str())
        else {
            return None;
        };

        Path::new(target).with_extension("").file_name().and_then(|e| e.to_str()).map(|e| e.to_string())
    }

    /// Get the output dir, ex. target/release/, target/x86_64/release
    pub fn target_dir(&self) -> PathBuf {
        let Some(name) = self.target_name() else {
            return self.build_path.join(self.config.mode.dir_name());
        };

        self.build_path.join(name).join(self.config.mode.dir_name())
    }

    /// Build the binary at the provided path with cargo build,
    /// and return the executable bin path
    pub fn build(&mut self) -> Result<PathBuf, super::Error> {
        let mut command = CommandBuilder::new("cargo");
        command.cwd(self.path);
        command.arg("build");
        self.config.into_command(&mut command);
        let status = self.executor.run(command).map_err(|error| super::Error::CommandIoError { error })?;

        if !status.success() {
            return Err(super::Error::CargoBuild { status });
        }

        // FIXME: What to do with dynlib? .so?
        let target_name = self.target_dir().join(self.package_name().expect("Invalid cargo project name"));

        Ok(target_name)
    }
}
