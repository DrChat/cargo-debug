use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use clap::Parser;
use log::{error, info, trace, warn};

use cargo_metadata::Message;

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
enum Debugger {
    Gdb,
    Gdbserver,
    Lldb,
    Devenv,
    Windbg,
}

impl std::default::Default for Debugger {
    fn default() -> Self {
        if cfg!(unix) {
            Debugger::Gdb
        } else if cfg!(windows) {
            Debugger::Devenv
        } else {
            panic!("no default debugger");
        }
    }
}

#[derive(clap::Args)]
#[command(author, version, about, long_about = None)]
struct Args {
    debugger: Option<Debugger>,
    #[clap(long)]
    release: bool,
    #[clap(long = "manifest-path")]
    manifest: Option<String>,
    #[clap(long = "example")]
    example: Option<String>,
    #[clap(long = "bin")]
    bin: Option<String>,
    #[clap(last = true)]
    options: Vec<String>,
}

#[derive(Parser)]
#[command(name = "cargo")]
#[command(bin_name = "cargo")]
enum CargoCli {
    Debug(Args),
}

fn main() -> Result<()> {
    // TermLogger::init(log::LevelFilter::Debug, simplelog::Config::default()).unwrap();

    let CargoCli::Debug(args) = CargoCli::parse();

    let options = args.options;

    trace!("building cargo command");

    // Build and execute cargo command
    let cargo_bin = env::var("CARGO").unwrap_or(String::from("cargo"));
    let mut cargo_cmd = Command::new(cargo_bin);

    cargo_cmd
        .args(["build", "--message-format=json"])
        .stdout(Stdio::piped());

    if args.release {
        cargo_cmd.arg("--release");
    }

    if let Some(manifest) = args.manifest {
        cargo_cmd.args(["--manifest-path", &manifest]);
    }

    if let Some(bin) = &args.bin {
        cargo_cmd.args(["--bin", bin]);
    }

    if let Some(example) = &args.example {
        cargo_cmd.args(["--example", example]);
    }

    trace!("synthesized cargo command: {:?}", cargo_cmd);

    trace!("launching cargo command");
    let mut handle = cargo_cmd.spawn().expect("error starting cargo command");

    // Log all output artifacts
    let mut artifacts = vec![];
    let reader = std::io::BufReader::new(handle.stdout.take().unwrap());
    for message in Message::parse_stream(reader) {
        match message.expect("Invalid cargo JSON message") {
            Message::CompilerArtifact(artifact) => {
                artifacts.push(artifact);
            }
            _ => (),
        }
    }

    // Await command completion
    let status = handle
        .wait()
        .expect("cargo command failed, try running the command directly");

    if let Some(code) = status.code() {
        if code != 0 {
            std::process::exit(code);
        }
    }

    trace!("command executed");

    // Find the output(s) we care about
    trace!("found {} artifacts: {:?}", artifacts.len(), artifacts);

    let binaries = artifacts
        .into_iter()
        .filter_map(|a| {
            if let Some(executable) = a.executable {
                Some((a.target, executable))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let bin = if let Some(binary) = &args.bin {
        if let Some(bin) = binaries.iter().find_map(|(target, exe)| {
            if target.name == **binary {
                Some(exe.clone())
            } else {
                None
            }
        }) {
            bin.to_string()
        } else {
            println!("Could not find binary artifact {binary}");
            std::process::exit(1);
        }
    } else {
        // Try and find the first binary. If more than one, return an error.
        if binaries.len() == 1 {
            binaries[0].1.clone().to_string()
        } else {
            println!(
                "More than one binary artifact produced, please explicitly specify the binary."
            );
            std::process::exit(1);
        }
    };

    info!("selected binary: {:?}", bin);

    let debugger = args.debugger.unwrap_or_default();

    let debug_path: PathBuf;
    let mut debug_args: Vec<String> = vec![];

    match debugger {
        Debugger::Gdb => {
            debug_path = PathBuf::from("gdb");

            // Prepare GDB to accept child options
            if !options.is_empty() {
                debug_args.push("--args".to_string());
            }

            // Append command file if provided
            /*
            if let Some(command_file) = o.command_file {
                debug_args.push("--command".to_string());
                debug_args.push(command_file);
            }
            */

            // Specify file to be debugged
            debug_args.push(bin.clone());

            // Append child options
            debug_args.extend(options.iter().cloned());
        }
        Debugger::Lldb => {
            debug_path = PathBuf::from("lldb");

            // Specify file to be debugged
            debug_args.push("--file".to_string());
            debug_args.push(bin.clone());

            // Append command file if provided
            /*
            if let Some(command_file) = o.command_file {
                debug_args.push("--source".to_string());
                debug_args.push(command_file);
            }
            */

            // Append child options
            if !options.is_empty() {
                debug_args.push("--".to_string());
                debug_args.extend(options.iter().cloned());
            }
        }
        Debugger::Gdbserver => {
            debug_path = PathBuf::from("gdbserver");

            /*
            if let Some(address) = o.address {
                debug_args.push(address);
            } else {
                error!("--address is required when gdbserver is used");
                std::process::exit(1);
            }
            */
            // Specify file to be debugged
            debug_args.push(bin.clone());

            // Append child options
            if !options.is_empty() {
                debug_args.extend(options.iter().cloned());
            }
        }
        Debugger::Devenv => {
            #[cfg(target_os = "windows")]
            {
                // Find the path to devenv
                let install_info = vswhere::Config::new()
                    .only_latest_versions(true)
                    .run_default_path()
                    .unwrap();

                let info = install_info.iter().find(|m| {
                    m.product_id()
                        .starts_with("Microsoft.VisualStudio.Product.")
                });

                if let Some(info) = info {
                    debug_path = info.product_path().to_owned();
                    debug_args.push("/DebugExe".to_string());

                    // Specify file to be debugged
                    debug_args.push(bin.clone());

                    // Append child options
                    if !options.is_empty() {
                        debug_args.extend(options.iter().cloned());
                    }
                } else {
                    error!("Could not find a compatible version of Visual Studio :(");
                    std::process::exit(1);
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                panic!("devenv is only available on Windows");
            }
        }
        Debugger::Windbg => {
            debug_path = PathBuf::from("windbgx");

            debug_args.push("-o".to_string());

            // Specify file to be debugged
            debug_args.push(bin.clone());

            // Append child options
            if !options.is_empty() {
                debug_args.extend(options.iter().cloned());
            }
        }
    }

    trace!("synthesized debug arguments: {:?}", debug_args);

    /*
    if o.no_run {
        trace!("no-run selected, exiting");
        println!("Debug command: ");
        println!("{} {}", debug_path.display(), debug_args.join(" "));
        std::process::exit(0);
    }
    */

    let b = Arc::new(Mutex::new(SystemTime::now()));

    // Override ctrl+c handler to avoid premature exit
    // TODO: this... doesn't stop the rust process exiting..?
    ctrlc::set_handler(move || {
        warn!("CTRL+C");
        let mut then = b.lock().unwrap();
        let now = SystemTime::now();
        if now.duration_since(*then).unwrap() > Duration::from_secs(1) {
            std::process::exit(0);
        } else {
            *then = now;
        }
    })
    .expect("Error setting Ctrl-C handler");

    let mut debug_cmd = Command::new(&debug_path);
    debug_cmd.args(debug_args);

    trace!("synthesized debug command: {:?}", debug_cmd);

    debug_cmd.status().expect("error running debug command");

    trace!("debug command done");

    Ok(())
}

#[cfg(test)]
mod test {
    #[test]
    fn fake_test() {
        assert!(true);
    }
}
