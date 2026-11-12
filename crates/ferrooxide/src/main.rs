//! ferrooxide — parse, elaborate and simulate Verilog in one step.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage: ferrooxide [options] <files...>
options:
  -s <top>          top-level module (may be repeated; default: all uninstantiated modules)
  -D <name>[=val]   define a preprocessor macro
  -I <dir>          add an include directory
  --max-time <t>    stop simulation after time t
  --stats           print timing and scheduler statistics to stderr
  --dump-code       print the compiled bytecode to stderr
  -h, --help        show this help";

fn main() -> ExitCode {
    let mut files = Vec::new();
    let mut tops = Vec::new();
    let mut opts = ferrooxide_front::Options::default();
    let mut stats = false;
    let mut dump_code = false;
    let mut max_time = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = |name: &str| -> Option<String> {
            let v = args.next();
            if v.is_none() {
                eprintln!("ferrooxide: {name} needs an argument");
            }
            v
        };
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-s" => match val("-s") {
                Some(v) => tops.push(v),
                None => return ExitCode::from(2),
            },
            "-I" => match val("-I") {
                Some(v) => opts.incdirs.push(PathBuf::from(v)),
                None => return ExitCode::from(2),
            },
            "-D" => match val("-D") {
                Some(v) => {
                    let (k, v) = v.split_once('=').unwrap_or((&v, "1"));
                    opts.defines.insert(k.to_string(), v.to_string());
                }
                None => return ExitCode::from(2),
            },
            "--max-time" => match val("--max-time").and_then(|v| v.parse().ok()) {
                Some(v) => max_time = Some(v),
                None => return ExitCode::from(2),
            },
            "--stats" => stats = true,
            "--dump-code" => dump_code = true,
            _ if a.starts_with("-D") => {
                let v = &a[2..];
                let (k, v) = v.split_once('=').unwrap_or((v, "1"));
                opts.defines.insert(k.to_string(), v.to_string());
            }
            _ if a.starts_with("-I") => opts.incdirs.push(PathBuf::from(&a[2..])),
            _ if a.starts_with('-') => {
                eprintln!("ferrooxide: unknown option '{a}'\n{USAGE}");
                return ExitCode::from(2);
            }
            _ => files.push(PathBuf::from(a)),
        }
    }
    if files.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    let t0 = Instant::now();
    let modules = match ferrooxide_front::parse_files(&files, &opts) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let t1 = Instant::now();
    let (prog, warnings) = match ferrooxide_compile::compile(&modules, &tops) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    for w in &warnings {
        eprintln!("{w}");
    }
    if dump_code {
        for (i, p) in prog.procs.iter().enumerate() {
            eprintln!("proc {i} {} ({:?})", p.name, p.class);
            for (k, ins) in p.code.iter().enumerate() {
                eprintln!("  {k:4}: {ins:?}");
            }
        }
    }
    let t2 = Instant::now();
    let nsig = prog.signals.len();
    let nproc = prog.procs.len();
    let mut sim = ferrooxide_rt::Sim::new(prog);
    sim.run(max_time);
    let t3 = Instant::now();
    if stats {
        eprintln!(
            "ferrooxide: parse {:.3}s, elaborate {:.3}s, simulate {:.3}s; {} signals, {} processes",
            (t1 - t0).as_secs_f64(),
            (t2 - t1).as_secs_f64(),
            (t3 - t2).as_secs_f64(),
            nsig,
            nproc
        );
        eprintln!("ferrooxide: final time {}, {:?}", sim.time, sim.stats);
    }
    ExitCode::SUCCESS
}
