use std::{
    ffi::OsStr,
    fs::read_to_string,
    path::{Path, PathBuf},
};

use portable_pty::CommandBuilder;
use toml::Table;

use crate::build::{BuildConfig, CmdExecutor, detect_change};

#[derive(Debug)]
pub struct CargoProject<'a> {
    path: &'a Path,
    build_path: &'a Path,
    config: &'a BuildConfig,
    executor: CmdExecutor,
}

impl<'a> CargoProject<'a> {
    pub fn new(path: &'a Path, build_path: &'a Path, config: &'a BuildConfig, executor: CmdExecutor) -> Self {
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

        let name = toml.get("package").and_then(|build| build.get("name")).and_then(|target| target.as_str())?;

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
        let target = toml.get("build").and_then(|build| build.get("target")).and_then(|target| target.as_str())?;

        Path::new(target).with_extension("").file_name().and_then(|e| e.to_str()).map(|e| e.to_string())
    }

    /// Get the output dir, ex. target/release/, target/x86_64/release
    pub fn target_dir(&self) -> PathBuf {
        let Some(name) = self.target_name() else {
            return self.build_path.join(self.config.config.build_mode.dir_name());
        };

        self.build_path.join(name).join(self.config.config.build_mode.dir_name())
    }

    /// Build the binary at the provided path with cargo build,
    /// and return the executable bin path, and if the executeable has changed!
    pub fn build(&mut self) -> Result<(PathBuf, bool), super::Error> {
        let package_name = self.package_name().expect("Invalid cargo project name");
        let target = self.target_dir().join(package_name);

        let mut command = CommandBuilder::new("cargo");
        command.cwd(self.path);
        command.arg("build");
        self.config.into_command(&mut command);
        let status = self.executor.run(command.clone()).map_err(|error| super::Error::CommandIo { error })?;

        if !status.success() {
            let command_display = command.get_argv().join(OsStr::new(" ")).to_str().unwrap_or("").to_string();
            return Err(super::Error::CommandFailed {
                command: command_display,
                dir: self.path.display().to_string(),
                status,
            });
        }

        let built_executeable = find_executable(&target)?;
        let changed = detect_change(&built_executeable)?;
        Ok((built_executeable, changed))
    }
}

fn find_executable(base: &Path) -> Result<PathBuf, super::Error> {
    if base.exists() {
        return Ok(base.to_path_buf());
    }

    let so = base.with_extension("so");
    let efi = base.with_extension("efi");

    match (so.exists(), efi.exists()) {
        (true, false) => Ok(so),
        (false, true) => Ok(efi),
        (true, true) => Err(super::Error::AmbiguousExecutable { exe: base.display().to_string() }),
        (false, false) => panic!("Cargo built no known executable: {}(.so|.efi)", base.display()),
    }
}
