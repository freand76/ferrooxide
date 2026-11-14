//! ferrooxide — parse, elaborate and simulate Verilog in one step.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

mod cdsp;
mod mcp;
mod repl;
mod session;

const USAGE: &str = "usage: ferrooxide [options] <files...> [+plusarg...]
options:
  -s <top>          top-level module (may be repeated; default: all uninstantiated modules)
  -D <name>[=val]   define a preprocessor macro
  -I <dir>          add an include directory
  --max-time <t>    stop simulation after time t
  -q, --quiet       no startup summary (parse/elaborate/compile times) on stderr
  --stats           print timing and scheduler statistics to stderr
  --dump-code       print the compiled bytecode to stderr
  --no-jit          interpret the bytecode instead of compiling it to native code
  --no-fuse         do not fuse combinational processes
  -g2005            Verilog-2005 rules where SystemVerilog differs (default: SystemVerilog)
  -gspecify         apply specify path delays (ignored by default)
  -u                every file is a compilation unit of its own (macros, `timescale,
                    `default_nettype and file-scope declarations end with the file)
  --vcd <file>      write a VCD of the signals selected by --trace (all without --trace),
                    without $dumpvars in the testbench
  --trace <regex>   trace the signals whose hierarchical name matches (repeatable; implies
                    --vcd dump.vcd)
  -i, --interactive commands on stdin: run, step, print, list, set, force, release (help)
  --mcp             serve the simulator as MCP tools (JSON-RPC on stdio) for AI agents; files
                    on the command line are loaded at start, else with the load_design tool
  --cdsp [host:]port  serve the simulation over the CXXRTL debug server protocol (waveform
                    viewers attach live: surfer cxxrtl+tcp://127.0.0.1:port); starts paused
  --surfer          start the Surfer waveform viewer on a debug server and show the --trace
                    signals (else the top modules' signals); exits with Surfer (FX_SURFER:
                    the Surfer executable); with -i the console runs next to the viewer
  --lint            check the design (incomplete sensitivity lists, SystemVerilog keywords
                    as names) and exit; status 1 on findings
  +<arg>            plusarg for $test$plusargs / $value$plusargs
  -h, --help        show this help
environment: FX_MEM_LIMIT_MB (address-space cap, default 4096, 0: none), FX_MAX_MEM_MB (design
memory, default 2048), FX_HISTORY_MB (value history of --cdsp, default 512)";

/// Cap the process's address space (default 4 GB, `FX_MEM_LIMIT_MB`, 0: no cap): a runaway
/// design or bug then fails with an allocation error instead of exhausting the machine.
fn limit_memory() {
    let mb: u64 = std::env::var("FX_MEM_LIMIT_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(4096);
    if mb == 0 {
        return;
    }
    let bytes = mb.saturating_mul(1 << 20) as libc::rlim_t;
    unsafe {
        let mut cur = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_AS, &mut cur) == 0 {
            let soft = if cur.rlim_max == libc::RLIM_INFINITY { bytes } else { bytes.min(cur.rlim_max) };
            if cur.rlim_cur == libc::RLIM_INFINITY || soft < cur.rlim_cur {
                let lim = libc::rlimit { rlim_cur: soft, rlim_max: cur.rlim_max };
                libc::setrlimit(libc::RLIMIT_AS, &lim);
            }
        }
    }
}

fn main() -> ExitCode {
    limit_memory();
    let mut files = Vec::new();
    let mut plusargs = Vec::new();
    let mut tops = Vec::new();
    let mut opts = ferrooxide_front::Options::default();
    let mut stats = false;
    let mut quiet = false;
    let mut dump_code = false;
    let mut jit = true;
    let mut fuse = true;
    let mut lint = false;
    let mut interactive = false;
    let mut mcp_mode = false;
    let mut cdsp_addr: Option<String> = None;
    let mut surfer = false;
    let mut vcd: Option<String> = None;
    let mut traces: Vec<String> = Vec::new();
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
            "-q" | "--quiet" => quiet = true,
            "--vcd" => match val("--vcd") {
                Some(v) => vcd = Some(v),
                None => return ExitCode::from(2),
            },
            "--trace" => match val("--trace") {
                Some(v) => traces.push(v),
                None => return ExitCode::from(2),
            },
            "--dump-code" => dump_code = true,
            "--no-jit" => jit = false,
            "--no-fuse" => fuse = false,
            "--lint" => lint = true,
            "-i" | "--interactive" => interactive = true,
            "--mcp" => mcp_mode = true,
            "--surfer" => surfer = true,
            "--cdsp" => match val("--cdsp") {
                Some(v) => cdsp_addr = Some(v),
                None => return ExitCode::from(2),
            },
            "-g2005" | "-g2001" | "-g1995" => dialect.v2005 = true,
            "-g2005-sv" | "-g2009" | "-g2012" | "-g2017" => dialect.v2005 = false,
            "-gspecify" => dialect.specify = true,
            "-u" => opts.separate_units = true,
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
    if mcp_mode {
        let initial = (!files.is_empty()).then(|| session::LoadOptions {
            files: files.clone(),
            defines: opts.defines.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            incdirs: opts.incdirs.clone(),
            tops: tops.clone(),
            plusargs: plusargs.clone(),
            dialect,
            jit,
            fuse,
        });
        mcp::serve(initial);
        return ExitCode::SUCCESS;
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
    let nmod = modules.len();
    let mem_mb = prog.mem.len() as f64 * 8.0 / (1024.0 * 1024.0);
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
    if vcd.is_some() || (!traces.is_empty() && !surfer) {
        let path = vcd.clone().unwrap_or_else(|| "dump.vcd".into());
        let set = match regex::RegexSet::new(&traces) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ferrooxide: bad --trace pattern: {e}");
                return ExitCode::from(2);
            }
        };
        let all = traces.is_empty();
        if sim.trace_start(&path, &|n| all || set.is_match(n)).is_err() {
            return ExitCode::from(1);
        }
        if stats {
            eprintln!("ferrooxide: tracing {} signals to {path}", sim.traced_signals());
        }
    }
    let t2b = Instant::now();
    if !quiet {
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        eprintln!(
            "ferrooxide: {} files, {nmod} modules parsed in {:.0} ms; elaborated in {:.0} ms ({nsig} signals, {nproc} processes, {mem_mb:.1} MB); {} in {:.0} ms",
            files.len(),
            ms(t1 - t0),
            ms(t2 - t1),
            if jit { "native code compiled" } else { "interpreted, prepared" },
            ms(t2b - t2),
        );
    }
    if cdsp_addr.is_some() || surfer {
        let addr = cdsp_addr.clone().unwrap_or_else(|| "0".into());
        let viewer = surfer.then(|| cdsp::Viewer {
            bin: std::env::var("FX_SURFER").unwrap_or_else(|_| "surfer".into()),
            traces: traces.clone(),
        });
        if let Err(e) = cdsp::serve(sim, &addr, viewer, interactive) {
            eprintln!("ferrooxide: {e}");
            return ExitCode::from(1);
        }
        return ExitCode::SUCCESS;
    }
    if interactive {
        let mut ses = session::Session::from_sim(sim, precision, String::new());
        repl::run(&mut ses);
        sim = ses.sim;
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
