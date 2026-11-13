//! ferrooxide — parse, elaborate and simulate Verilog in one step.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

mod repl;

const USAGE: &str = "usage: ferrooxide [options] <files...> [+plusarg...]
options:
  -s <top>          top-level module (may be repeated; default: all uninstantiated modules)
  -D <name>[=val]   define a preprocessor macro
  -I <dir>          add an include directory
  --max-time <t>    stop simulation after time t
  --stats           print timing and scheduler statistics to stderr
  --dump-code       print the compiled bytecode to stderr
  --no-jit          interpret the bytecode instead of compiling it to native code
  --no-fuse         do not fuse combinational processes
  -g2005            Verilog-2005 rules where SystemVerilog differs (default: SystemVerilog)
  -gspecify         apply specify path delays (ignored by default)
  -i, --interactive commands on stdin: run, step, print, list, set, force, release (help)
  --lint            check the design (incomplete sensitivity lists) and exit; status 1 on findings
  +<arg>            plusarg for $test$plusargs / $value$plusargs
  -h, --help        show this help";

fn main() -> ExitCode {
    let mut files = Vec::new();
    let mut plusargs = Vec::new();
    let mut tops = Vec::new();
    let mut opts = ferrooxide_front::Options::default();
    let mut stats = false;
    let mut dump_code = false;
    let mut jit = true;
    let mut fuse = true;
    let mut lint = false;
    let mut interactive = false;
    let mut dialect = ferrooxide_compile::Dialect::default();
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
            "--no-jit" => jit = false,
            "--no-fuse" => fuse = false,
            "--lint" => lint = true,
            "-i" | "--interactive" => interactive = true,
            "-g2005" | "-g2001" | "-g1995" => dialect.v2005 = true,
            "-g2005-sv" | "-g2009" | "-g2012" | "-g2017" => dialect.v2005 = false,
            "-gspecify" => dialect.specify = true,
            _ if a.starts_with("-D") => {
                let v = &a[2..];
                let (k, v) = v.split_once('=').unwrap_or((v, "1"));
                opts.defines.insert(k.to_string(), v.to_string());
            }
            _ if a.starts_with("-I") => opts.incdirs.push(PathBuf::from(&a[2..])),
            _ if a.starts_with('+') => plusargs.push(a[1..].to_string()),
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
    let (mut prog, warnings, findings) = match ferrooxide_compile::compile_dialect(&modules, &tops, lint, dialect) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    for w in &warnings {
        eprintln!("{w}");
    }
    if lint {
        for f in &findings {
            println!("{f}");
        }
        println!("ferrooxide lint: {} finding{}", findings.len(), if findings.len() == 1 { "" } else { "s" });
        return if findings.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) };
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
    prog.no_fuse = !fuse;
    let nsig = prog.signals.len();
    if stats {
        eprintln!("ferrooxide: memory {} words", prog.mem.len());
    }
    let nproc = prog.procs.len();
    let precision = prog.precision;
    if interactive {
        // any signal may be forced from the command line: stores take the masked path
        for s in prog.signals.iter_mut() {
            if s.depth == 0 {
                s.forced = true;
            }
        }
    }
    let mut sim = ferrooxide_rt::Sim::new(prog);
    sim.set_plusargs(plusargs);
    if jit {
        if let Err(e) = sim.enable_jit() {
            eprintln!("ferrooxide: JIT compilation failed ({e}), interpreting");
        }
    }
    let t2b = Instant::now();
    if interactive {
        repl::run(&mut sim, precision);
    } else {
        sim.run(max_time);
    }
    let t3 = Instant::now();
    if stats {
        eprintln!(
            "ferrooxide: parse {:.3}s, elaborate {:.3}s, jit {:.3}s, simulate {:.3}s; {} signals, {} processes",
            (t1 - t0).as_secs_f64(),
            (t2 - t1).as_secs_f64(),
            (t2b - t2).as_secs_f64(),
            (t3 - t2b).as_secs_f64(),
            nsig,
            nproc
        );
        eprintln!("ferrooxide: final time {}, {:?}", sim.time, sim.stats);
    }
    ExitCode::SUCCESS
}
