use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use clap::builder::PossibleValuesParser;
use clap::Arg;
use log::{error, info, trace, warn};

use cargo_metadata::Message;
// use simplelog::TermLogger;

/*
#[derive(StructOpt)]
#[structopt(
    name = "cargo-debug",
    about = "Cargo debug subcommand, wraps cargo invocations and launches a debugger"
)]
struct Options {
    #[structopt(default_value = "build")]
    /// Subcommand to invoke within cargo
    subcommand: String,

    #[cfg_attr(
        target_os = "windows",
        structopt(long = "debugger", default_value = "devenv")
    )]
    #[cfg_attr(
        target_os = "unix",
        structopt(long = "debugger", default_value = "gdb")
    )]
    /// Debugger to launch as a subprocess
    debugger: String,

    #[structopt(long = "command-file")]
    /// Command file to be passed to debugger
    command_file: Option<String>,

    #[structopt(long = "address")]
    /// Address to be passed to gdbserver. Required only for gdbserver
    address: Option<String>,

    #[structopt(long = "filter")]
    /// Filter to match against multiple output files
    filter: Option<String>,

    #[structopt(long = "no-run")]
    /// Print the debug command to the terminal and exit without running
    no_run: bool,

    #[structopt(long = "log-level", default_value = "info")]
    /// Enable verbose logging
    level: LevelFilter,
}
*/

fn main() -> Result<()> {
    // TermLogger::init(log::LevelFilter::Debug, simplelog::Config::default()).unwrap();

    let matches = clap::Command::new("cargo")
        .bin_name("cargo")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Justin Moore")
        .about("Build and run a binary under a debugger")
        .subcommand(
            clap::Command::new("debug").args([
                Arg::new("debugger")
                    .num_args(1)
                    .value_parser(PossibleValuesParser::new(["gdb", "devenv"])),
                Arg::new("release")
                    .long("release")
                    .action(clap::ArgAction::SetTrue),
                Arg::new("manifest").long("manifest-path").num_args(1),
                Arg::new("example").long("example").num_args(1),
                Arg::new("bin").long("bin").num_args(1),
                Arg::new("options").num_args(1..).trailing_var_arg(true),
            ]),
        )
        .get_matches();

    let matches = match matches.subcommand() {
        Some(("debug", matches)) => matches,
        _ => unreachable!("invalid subcommand"),
    };

    let options = matches.get_many::<String>("options");

    trace!("building cargo command");

    // Build and execute cargo command
    let cargo_bin = env::var("CARGO").unwrap_or(String::from("cargo"));
    let mut cargo_cmd = Command::new(cargo_bin);

    cargo_cmd
        .args(["build", "--message-format=json"])
        .stdout(Stdio::piped());

    if matches.get_flag("release") {
        cargo_cmd.arg("--release");
    }

    if let Some(manifest) = matches.get_one::<String>("manifest") {
        cargo_cmd.args(["--manifest-path", manifest]);
    }

    let bin = matches.get_one::<String>("bin");
    if let Some(bin) = &bin {
        cargo_cmd.args(["--bin", bin]);
    }

    let example = matches.get_one::<String>("example");
    if let Some(example) = &example {
        cargo_cmd.args(["--example", example]);
    }

    trace!("synthesized cargo command: {:?}", cargo_cmd);

    trace!("launching cargo command");
    let mut handle = cargo_cmd.spawn().expect("error starting cargo command");

    // Log all output artifacts
    let mut artifacts = vec![];
    for message in cargo_metadata::parse_messages(handle.stdout.take().unwrap()) {
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

    let bin = if let Some(binary) = &bin {
        if let Some(bin) = binaries.iter().find_map(|(target, exe)| {
            if target.name == **binary {
                Some(exe.clone())
            } else {
                None
            }
        }) {
            bin
        } else {
            println!("Could not find binary artifact {binary}");
            std::process::exit(1);
        }
    } else {
        // Try and find the first binary. If more than one, return an error.
        if binaries.len() == 1 {
            binaries[0].1.clone()
        } else {
            println!(
                "More than one binary artifact produced, please explicitly specify the binary."
            );
            std::process::exit(1);
        }
    };

    info!("selected binary: {:?}", bin);

    let debugger = if let Some(dbg) = matches.get_one::<String>("debugger") {
        dbg.as_str()
    } else {
        if cfg!(unix) {
            "gdb"
        } else if cfg!(windows) {
            "devenv"
        } else {
            panic!("no default debugger");
        }
    };

    let debug_path: PathBuf;
    let mut debug_args: Vec<String> = vec![];

    match debugger {
        "gdb" => {
            debug_path = PathBuf::from("gdb");

            // Prepare GDB to accept child options
            if options.is_some() {
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
            debug_args.push(bin.clone().to_str().unwrap().to_string());

            // Append child options
            if let Some(opts) = options {
                debug_args.extend(opts.cloned());
            }
        }
        "lldb" => {
            debug_path = PathBuf::from("lldb");

            // Specify file to be debugged
            debug_args.push("--file".to_string());
            debug_args.push(bin.clone().to_str().unwrap().to_string());

            // Append command file if provided
            /*
            if let Some(command_file) = o.command_file {
                debug_args.push("--source".to_string());
                debug_args.push(command_file);
            }
            */

            // Append child options
            if let Some(opts) = options {
                debug_args.push("--".to_string());
                debug_args.extend(opts.cloned());
            }
        }
        "gdbserver" => {
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
            debug_args.push(bin.clone().to_str().unwrap().to_string());

            // Append child options
            if let Some(opts) = options {
                debug_args.extend(opts.cloned());
            }
        }
        "devenv" => {
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
                debug_args.push(bin.clone().to_str().unwrap().to_string());

                // Append child options
                if let Some(opts) = options {
                    debug_args.extend(opts.cloned());
                }
            } else {
                error!("Could not find a compatible version of Visual Studio :(");
                std::process::exit(1);
            }
        }
        _ => {
            error!("unsupported or unrecognised debugger {}", debugger);
            std::process::exit(1);
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
