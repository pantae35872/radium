use std::{
    env,
    ffi::OsStr,
    fs::{OpenOptions, create_dir, remove_file},
    io::{self, Read, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, mpsc::Sender},
    thread,
};

use packery::Packery;
use portable_pty::{CommandBuilder, ExitStatus, NativePtySystem, PtySystem};
use thiserror::Error;

use crate::{
    AppEvent, AppFormatter,
    build::cargo_project::CargoProject,
    config::{BuildMode, Config, ConfigRoot},
};

mod baker;
mod cargo_project;
mod fat;
mod iso;

pub fn build(event: Sender<AppEvent>, config: BuildConfig) -> Result<(), Error> {
    let current_dir = project_dir()?;

    Builder {
        config,
        root_path: current_dir.clone(),
        build_path: current_dir.join("build"),
        src_path: current_dir.join("src"),
        userland_path: current_dir.join("userland"),

        formatter: AppFormatter::from(&event),
        executor: CmdExecutor::from(&event),
    }
    .build()
}

pub fn project_dir() -> Result<PathBuf, Error> {
    let mut current_dir = env::current_dir().map_err(|error| Error::CurrentDir { error })?;

    while current_dir.file_name().is_some_and(|e| e != OsStr::new("radium")) {
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
    CommandIo { error: io::Error },
    #[error("Command `{command}` in dir `{dir}`, failed with exit status {status}")]
    CommandFailed { command: String, dir: String, status: ExitStatus },
    #[error("Failed to run QEMU, failed with: `{error}`")]
    Qemu { error: io::Error },
    #[error("Failed to download font, failed with: `{error}`")]
    DownloadFont { error: io::Error },
    #[error("Failed to build ovmf, failed with: `{error}`")]
    OvmfFailed { error: io::Error },
}

#[derive(Debug)]
struct Builder {
    config: BuildConfig,
    src_path: PathBuf,
    userland_path: PathBuf,
    build_path: PathBuf,
    root_path: PathBuf,
    executor: CmdExecutor,
    formatter: AppFormatter,
}

impl Builder {
    fn userland(&self, name: &str) -> PathBuf {
        self.userland_path.join(name)
    }

    fn src(&self, name: &str) -> PathBuf {
        self.src_path.join(name)
    }

    fn build(mut self) -> Result<(), Error> {
        make_build_dir()?;
        self.gen_config()?;

        let font_file = self.build_path.join("kernel_font.ttf");
        if !font_file.exists() {
            let mut command = CommandBuilder::new("wget");
            command.cwd(&self.build_path);
            command.args(["-O", "kernel_font.ttf", "https://www.1001fonts.com/download/font/open-sans.regular.ttf"]);
            self.executor.run(command).map_err(|error| Error::DownloadFont { error })?;
        }

        let ovmf_file = self.root_path.join("OVMF.fd");

        if !ovmf_file.exists() {
            let mut command = CommandBuilder::new("git");
            command.cwd(&self.root_path);
            command.args(["submodule", "update", "--init"]);
            self.executor.run(command).map_err(|error| Error::OvmfFailed { error })?;

            let mut command = CommandBuilder::new("git");
            command.cwd(self.root_path.join("vendor").join("edk2"));
            command.args(["submodule", "update", "--init"]);
            self.executor.run(command).map_err(|error| Error::OvmfFailed { error })?;

            let mut command = CommandBuilder::new("bash");
            command.cwd(&self.root_path);
            command.args(["-c", "cd vendor/edk2 && make -C BaseTools && source edksetup.sh && build -a X64 -t GCC5 -p OvmfPkg/OvmfPkgX64.dsc -b RELEASE"]);
            self.executor.run(command).map_err(|error| Error::OvmfFailed { error })?;
            std::fs::copy(
                self.root_path
                    .join("vendor")
                    .join("edk2")
                    .join("Build")
                    .join("OvmfX64")
                    .join("RELEASE_GCC5")
                    .join("FV")
                    .join("OVMF")
                    .with_extension("fd"),
                ovmf_file,
            )
            .map_err(|error| Error::OvmfFailed { error })?;
        }

        let (build_tool, modified) = self.project(&self.root_path.join("build-tool")).build()?;
        if modified && self.config.config.reexec_build_tool {
            ratatui::restore();
            return Err(Error::ReExecFailed { error: Command::new(build_tool).arg("true").exec() });
        }

        let kernel = self.project(&self.src("kernel")).build()?.0;
        let bootloader = self.project(&self.src("bootloader")).build()?.0;
        let init = self.project(&self.userland("init")).build()?.0;
        assert!(kernel.exists() && bootloader.exists() && init.exists() && build_tool.exists());

        let kernel = self.read_file(kernel).map_err(|error| Error::GenIso { error })?;
        let bootloader = self.read_file(bootloader).map_err(|error| Error::GenIso { error })?;
        let init = self.read_file(init).map_err(|error| Error::GenIso { error })?;
        let font_file = self.read_file(font_file).map_err(|error| Error::GenIso { error })?;

        let mut packery = Packery::new();
        packery.push("init", &init);

        let mut root = DirectoryWriter::new();
        root.dir("EFI", |efi| {
            efi.dir("BOOT", |boot| {
                boot.file("BOOTX64.EFI", bootloader);
            });
        });
        root.dir(self.config.config.boot_loader.file_root.as_str().trim_prefix("\\"), |boot| {
            boot.file(self.config.config.boot_loader.dwarf_file.as_str(), baker::bake(&kernel));
            boot.file(self.config.config.boot_loader.packed_file.as_str(), packery.pack());
            boot.file(self.config.config.boot_loader.font_file.as_str(), font_file);
            boot.file(self.config.config.boot_loader.kernel_file.as_str(), kernel);
        });

        let mut formatter = self.formatter.clone();
        self.executor.psudo_cmd::<Result<_, _>>("Generating iso image...", || {
            let _ = writeln!(formatter, "Creating bootable iso image...\r");
            let fat = fat::make(&root.directory);
            let iso = iso::make(&root.directory, Some(fat));
            let _ = writeln!(formatter, "Writing the iso image to disk...\r");
            self.write_file(self.build_path.join("radium.iso"), &iso).map_err(|error| Error::GenIso { error })?;
            let _ = writeln!(formatter, "Done!\r");
            Ok(())
        })?;

        if self.config.config.qemu.run_qemu {
            let mut command = CommandBuilder::new("qemu-system-x86_64");
            command.cwd(self.root_path);
            command.args(["-m", &format!("{}M", self.config.config.qemu.memory)]);
            command.args(["-smp", &format!("cores={}", self.config.config.qemu.core_count)]);
            command.args([
                "-bios",
                "OVMF.fd",
                "-usb",
                "-device",
                "usb-ehci,id=ehci",
                "-device",
                "usb-tablet,bus=usb-bus.0",
                "-no-reboot",
                "-serial",
                "stdio",
                "-display",
                "sdl",
            ]);
            if self.config.config.qemu.enable_kvm {
                command.args(["-enable-kvm", "-cpu", "host,+rdrand,+sse,+mmx"]);
            }
            command.args(["-cdrom", &format!("{}", self.build_path.join("radium.iso").display())]);
            self.executor.run(command.clone()).map_err(|error| Error::Qemu { error })?;
        }

        Ok(())
    }

    fn read_file(&self, path: impl AsRef<Path>) -> Result<Vec<u8>, io::Error> {
        let path = path.as_ref();
        let mut file = OpenOptions::new().read(true).open(path)?;
        let mut file_data = Vec::new();
        file.read_to_end(&mut file_data)?;
        Ok(file_data)
    }

    fn write_file(&self, path: impl AsRef<Path>, data: &[u8]) -> Result<(), io::Error> {
        let path = path.as_ref();
        if path.exists() {
            remove_file(path)?;
        }
        OpenOptions::new().create(true).write(true).truncate(true).open(path).and_then(|mut f| f.write_all(data))
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
        CargoProject::new(path, &self.build_path, &self.config, self.executor.clone())
    }
}

#[derive(Debug)]
pub struct BuildConfig {
    pub config: Arc<ConfigRoot>,
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

#[derive(Debug, Default)]
pub struct DirectoryWriter {
    directory: Vec<Directory>,
}

impl DirectoryWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file(&mut self, name: impl AsRef<str>, data: Vec<u8>) -> &mut Self {
        self.directory.push(Directory::File { name: name.as_ref().to_string(), data });
        self
    }

    pub fn dir(&mut self, name: impl AsRef<str>, dir: impl FnOnce(&mut DirectoryWriter)) -> &mut Self {
        let mut new_dir = DirectoryWriter::new();
        dir(&mut new_dir);
        self.directory.push(Directory::Directory { name: name.as_ref().to_string(), child: new_dir.directory });
        self
    }
}

#[derive(Debug, Clone)]
enum Directory {
    File { name: String, data: Vec<u8> },
    Directory { name: String, child: Vec<Directory> },
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

#[derive(Debug, Clone)]
struct CmdExecutor(Sender<AppEvent>);

impl From<&Sender<AppEvent>> for CmdExecutor {
    fn from(value: &Sender<AppEvent>) -> Self {
        Self(value.clone())
    }
}

impl CmdExecutor {
    pub fn psudo_cmd<R>(&self, name: &str, runner: impl FnOnce() -> R) -> R {
        let _ = self.0.send(AppEvent::RunningCmd(name.to_string()));
        let result = runner();
        let _ = self.0.send(AppEvent::CmdStopped);

        result
    }

    pub fn run(&mut self, command: CommandBuilder) -> Result<ExitStatus, io::Error> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(Default::default()).unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut process = pair.slave.spawn_command(command.clone()).unwrap();

        let command_display = command.get_argv().join(OsStr::new(" ")).to_str().unwrap_or("").to_string();
        let cmd_cwd = command.get_cwd().and_then(|e| e.to_str()).unwrap_or("unknown");
        let _ =
            self.0.send(AppEvent::Output(format!("Running `{command_display}` in {cmd_cwd}\r\n").as_bytes().to_vec()));
        let _ = self.0.send(AppEvent::RunningCmd(format!("`{command_display}` in {cmd_cwd}")));

        let event_stream = self.0.clone();
        thread::spawn(move || {
            let mut buffer = [0u8; 64];
            while let Ok(readed) = reader.read(&mut buffer) {
                if readed == 0 {
                    break;
                }
                let _ = event_stream.send(AppEvent::Output(buffer[0..readed].to_vec()));
            }
        });
        let _ = self.0.send(AppEvent::ChildProcessStarted(process.clone_killer()));

        let result = process.wait()?;

        let _ = self.0.send(AppEvent::CmdStopped);

        Ok(result)
    }
}
