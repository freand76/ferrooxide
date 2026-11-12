//! Cranelift JIT: compiles each process's lowered ops to native code.
//!
//! A compiled process is `extern "C" fn(sim, mem, pc)`. It starts at `pc` (through a dispatch
//! switch), runs until it suspends and returns. Single-word ops are emitted inline as machine
//! code operating on the flat memory; everything that touches the scheduler (wait, delay, NBA,
//! change notification) and every wide/generic instruction calls back into the runtime.

use crate::fast::{mk, Op};
use crate::vm::Sim;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, Block, FuncRef, InstBuilder, MemFlags, Signature, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, FuncId, Linkage, Module};

/// (sim, mem, pc, table): `table` holds [proc id, slots.., signals.., event controls..]
pub type ProcFn = unsafe extern "C" fn(*mut Sim, *mut u64, u32, *const u32, *mut crate::vm::Hot);

pub struct Jit {
    // keeps the generated code alive
    _module: JITModule,
    pub fns: Vec<ProcFn>,
    /// per-process tables (see ProcFn), concatenated; `table_off[p]` indexes into it
    pub tables: Vec<u32>,
    pub table_off: Vec<usize>,
    /// number of distinct compiled functions
    pub shapes: usize,
}

/// A process normalised so that identical processes of different instances compare equal.
struct Shape {
    ops: Vec<Op>,
    nslots: u32,
    nsigs: u32,
    nctls: u32,
}

fn normalise(p: u32, ops: &[Op]) -> (Shape, Vec<u32>) {
    use crate::fast::{map_op, Field};
    let mut slots: Vec<u32> = Vec::new();
    let mut sigs: Vec<u32> = Vec::new();
    let mut ctls: Vec<u32> = Vec::new();
    let mut out = Vec::with_capacity(ops.len());
    for &op in ops {
        out.push(map_op(op, &mut |f, x| {
            let v = match f {
                Field::Slot => &mut slots,
                Field::Sig => &mut sigs,
                Field::Ctl => &mut ctls,
            };
            let i = match v.iter().position(|&y| y == x) {
                Some(i) => i as u32,
                None => {
                    v.push(x);
                    (v.len() - 1) as u32
                }
            };
            // slots are encoded as 2*i so that `slot + 1` names the unknown-bits word
            if f == Field::Slot {
                2 * i
            } else {
                i
            }
        }));
    }
    let mut table = vec![p];
    table.extend_from_slice(&slots);
    table.extend_from_slice(&sigs);
    table.extend_from_slice(&ctls);
    (Shape { ops: out, nslots: slots.len() as u32, nsigs: sigs.len() as u32, nctls: ctls.len() as u32 }, table)
}

/// Runtime entry points called from generated code.
pub struct Callbacks {
    pub notify: unsafe extern "C" fn(*mut Sim, u32, u32),
    pub nba: unsafe extern "C" fn(*mut Sim, u32, u32, u64, u64),
    pub gen: unsafe extern "C" fn(*mut Sim, u32, u32) -> u64,
    pub wait: unsafe extern "C" fn(*mut Sim, u32, u32, u32),
    pub delay: unsafe extern "C" fn(*mut Sim, u32, u32, u32),
    pub halt: unsafe extern "C" fn(*mut Sim, u32, u32),
}

struct Imports {
    notify: FuncId,
    nba: FuncId,
    gen: FuncId,
    delay: FuncId,
    halt: FuncId,
}

struct Refs {
    notify: FuncRef,
    nba: FuncRef,
    gen: FuncRef,
    delay: FuncRef,
    halt: FuncRef,
}

/// `proc_size`: size of the runtime's process record (pc at offset 0, wait at offset 4).
pub fn compile(procs: &[Vec<Op>], cb: &Callbacks, proc_size: u32) -> Result<Jit, String> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    flags.set("use_colocated_libcalls", "false").map_err(|e| e.to_string())?;
    flags.set("is_pic", "false").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let mut jb = JITBuilder::with_isa(isa, default_libcall_names());
    jb.symbol("fx_notify", cb.notify as *const u8);
    jb.symbol("fx_nba", cb.nba as *const u8);
    jb.symbol("fx_gen", cb.gen as *const u8);
    jb.symbol("fx_wait", cb.wait as *const u8);
    jb.symbol("fx_delay", cb.delay as *const u8);
    jb.symbol("fx_halt", cb.halt as *const u8);
    let mut module = JITModule::new(jb);
    let ptr = module.target_config().pointer_type();
    let i32t = types::I32;
    let i64t = types::I64;

    let mk_sig = |m: &JITModule, params: &[types::Type], ret: Option<types::Type>| -> Signature {
        let mut s = m.make_signature();
        for &p in params {
            s.params.push(AbiParam::new(p));
        }
        if let Some(r) = ret {
            s.returns.push(AbiParam::new(r));
        }
        s
    };
    let decl = |m: &mut JITModule, name: &str, s: &Signature| -> Result<FuncId, String> {
        m.declare_function(name, Linkage::Import, s).map_err(|e| e.to_string())
    };
    let s_notify = mk_sig(&module, &[ptr, i32t, i32t], None);
    let s_nba = mk_sig(&module, &[ptr, i32t, i32t, i64t, i64t], None);
    let s_gen = mk_sig(&module, &[ptr, i32t, i32t], Some(i64t));
    let s_wait = mk_sig(&module, &[ptr, i32t, i32t, i32t], None);
    let s_halt = mk_sig(&module, &[ptr, i32t, i32t], None);
    let imports = Imports {
        notify: decl(&mut module, "fx_notify", &s_notify)?,
        nba: decl(&mut module, "fx_nba", &s_nba)?,
        gen: decl(&mut module, "fx_gen", &s_gen)?,
        delay: decl(&mut module, "fx_delay", &s_wait)?,
        halt: decl(&mut module, "fx_halt", &s_halt)?,
    };
    let proc_sig = mk_sig(&module, &[ptr, ptr, i32t, ptr, ptr], None);

    let mut ctx = module.make_context();
    let mut fbc = FunctionBuilderContext::new();
    let mut shape_ids: std::collections::HashMap<Vec<Op>, FuncId> = std::collections::HashMap::new();
    let mut ids = Vec::with_capacity(procs.len());
    let mut tables = Vec::new();
    let mut table_off = Vec::with_capacity(procs.len());
    for (p, ops) in procs.iter().enumerate() {
        let (shape, table) = normalise(p as u32, ops);
        table_off.push(tables.len());
        tables.extend_from_slice(&table);
        if let Some(&id) = shape_ids.get(&shape.ops) {
            ids.push(id);
            continue;
        }
        ctx.func.signature = proc_sig.clone();
        let id = module
            .declare_function(&format!("shape{}", shape_ids.len()), Linkage::Local, &proc_sig)
            .map_err(|e| e.to_string())?;
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let refs = Refs {
                notify: module.declare_func_in_func(imports.notify, b.func),
                nba: module.declare_func_in_func(imports.nba, b.func),
                gen: module.declare_func_in_func(imports.gen, b.func),
                delay: module.declare_func_in_func(imports.delay, b.func),
                halt: module.declare_func_in_func(imports.halt, b.func),
            };
            emit_proc(&mut b, &refs, &shape, proc_size);
            b.seal_all_blocks();
            b.finalize();
        }
        module.define_function(id, &mut ctx).map_err(|e| format!("proc {p}: {e:?}"))?;
        module.clear_context(&mut ctx);
        shape_ids.insert(shape.ops, id);
        ids.push(id);
    }
    module.finalize_definitions().map_err(|e| e.to_string())?;
    let fns = ids
        .iter()
        .map(|&id| {
            let f = module.get_finalized_function(id);
            // SAFETY: the function was declared with the ProcFn signature
            unsafe { std::mem::transmute::<*const u8, ProcFn>(f) }
        })
        .collect();
    Ok(Jit { _module: module, fns, tables, table_off, shapes: shape_ids.len() })
}

fn emit_proc(b: &mut FunctionBuilder, r: &Refs, shape: &Shape, proc_size: u32) {
    let ops = &shape.ops[..];
    let i64t = types::I64;
    let i32t = types::I32;
    let fl = MemFlags::trusted();
    let n = ops.len();
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    let blocks: Vec<Block> = (0..n).map(|_| b.create_block()).collect();
    let dispatch = b.create_block();
    b.append_block_param(dispatch, i32t);
    let exit = b.create_block();

    b.switch_to_block(entry);
    let params = b.block_params(entry).to_vec();
    let (sim, mem, pc0, table, hot) = (params[0], params[1], params[2], params[3], params[4]);
    // Hot layout: nba_ptr @0, nba_len @8, nba_cap @16, procs @24
    let procs = b.ins().load(i64t, MemFlags::trusted(), hot, 24);
    // load the process's table: proc id, slot addresses, signal ids, control ids
    let tfl = MemFlags::trusted().with_readonly();
    let p_v = b.ins().load(i32t, tfl, table, 0);
    // address of this process's record
    let prec = {
        let p64 = b.ins().uextend(i64t, p_v);
        let off = b.ins().imul_imm(p64, proc_size as i64);
        b.ins().iadd(procs, off)
    };
    let mut addr: Vec<Value> = Vec::new();
    let mut slot_no: Vec<Value> = Vec::new();
    for k in 0..shape.nslots {
        let v = b.ins().uload32(tfl, table, ((1 + k) * 4) as i32);
        let off = b.ins().ishl_imm(v, 3);
        addr.push(b.ins().iadd(mem, off));
        slot_no.push(v);
    }
    let mut sig_v: Vec<Value> = Vec::new();
    for k in 0..shape.nsigs {
        sig_v.push(b.ins().load(i32t, tfl, table, ((1 + shape.nslots + k) * 4) as i32));
    }
    let mut ctl_v: Vec<Value> = Vec::new();
    for k in 0..shape.nctls {
        ctl_v.push(b.ins().load(i32t, tfl, table, ((1 + shape.nslots + shape.nsigs + k) * 4) as i32));
    }
    b.ins().jump(dispatch, &[pc0.into()]);

    // dispatch: jump to the block of an arbitrary pc (entry and after generic instructions)
    b.switch_to_block(dispatch);
    let dpc = b.block_params(dispatch)[0];
    let mut sw = Switch::new();
    for (i, &blk) in blocks.iter().enumerate() {
        sw.set_entry(i as u128, blk);
    }
    sw.emit(b, dpc, exit);

    b.switch_to_block(exit);
    b.ins().return_(&[]);

    let _ = mem;
    // slot fields are table indices; `k + 1` addresses the unknown-bits word of slot k
    let ld = |b: &mut FunctionBuilder, slot: u32| -> Value { b.ins().load(i64t, fl, addr[(slot >> 1) as usize], ((slot & 1) * 8) as i32) };
    let st = |b: &mut FunctionBuilder, slot: u32, v: Value| {
        b.ins().store(fl, v, addr[(slot >> 1) as usize], ((slot & 1) * 8) as i32);
    };
    let c64 = |b: &mut FunctionBuilder, v: u64| -> Value { b.ins().iconst(i64t, v as i64) };
    let c32 = |b: &mut FunctionBuilder, v: u32| -> Value { b.ins().iconst(i32t, v as i64) };
    let b2i = |b: &mut FunctionBuilder, c: Value| -> Value { b.ins().uextend(i64t, c) };

    for (i, &op) in ops.iter().enumerate() {
        b.switch_to_block(blocks[i]);
        let next = if i + 1 < n { blocks[i + 1] } else { exit };
        let goto_next = |b: &mut FunctionBuilder| {
            b.ins().jump(next, &[]);
        };
        match op {
            Op::Gen => {
                let pcv = c32(b, i as u32);
                let call = b.ins().call(r.gen, &[sim, p_v, pcv]);
                let ret = b.inst_results(call)[0];
                let susp = b.ins().icmp_imm(IntCC::Equal, ret, -1);
                let cont = b.create_block();
                b.ins().brif(susp, exit, &[], cont, &[]);
                b.switch_to_block(cont);
                let npc = b.ins().ireduce(i32t, ret);
                b.ins().jump(dispatch, &[npc.into()]);
            }
            Op::Copy1 { d, s } => {
                let (a, x) = (ld(b, s), ld(b, s + 1));
                st(b, d, a);
                st(b, d + 1, x);
                goto_next(b);
            }
            Op::Slice1 { d, s, sh, w } => {
                let m = c64(b, mk(w));
                for k in 0..2 {
                    let v = ld(b, s + k);
                    let v = b.ins().ushr_imm(v, sh as i64);
                    let v = b.ins().band(v, m);
                    st(b, d + k, v);
                }
                goto_next(b);
            }
            Op::Trunc1 { d, s, w } => {
                let m = c64(b, mk(w));
                for k in 0..2 {
                    let v = ld(b, s + k);
                    let v = b.ins().band(v, m);
                    st(b, d + k, v);
                }
                goto_next(b);
            }
            Op::Sext1 { d, s, sw, w } => {
                let m = c64(b, mk(w));
                let sh = 64 - sw as i64;
                for k in 0..2 {
                    let v = ld(b, s + k);
                    let v = if sh > 0 {
                        let v = b.ins().ishl_imm(v, sh);
                        b.ins().sshr_imm(v, sh)
                    } else {
                        v
                    };
                    let v = b.ins().band(v, m);
                    st(b, d + k, v);
                }
                goto_next(b);
            }
            Op::Not1 { d, s, w } => {
                let m = c64(b, mk(w));
                let (a, x) = (ld(b, s), ld(b, s + 1));
                let na = b.ins().bnot(a);
                let r = b.ins().bor(na, x);
                let r = b.ins().band(r, m);
                st(b, d, r);
                st(b, d + 1, x);
                goto_next(b);
            }
            Op::And1 { d, a, b: bb } | Op::Or1 { d, a, b: bb } => {
                let is_and = matches!(op, Op::And1 { .. });
                let (va, xa, vb, xb) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                // known 1 / known 0 bits of each operand
                let nxa = b.ins().bnot(xa);
                let nxb = b.ins().bnot(xb);
                let a1 = b.ins().band(va, nxa);
                let b1 = b.ins().band(vb, nxb);
                let a0 = b.ins().bor(va, xa);
                let a0 = b.ins().bnot(a0);
                let b0 = b.ins().bor(vb, xb);
                let b0 = b.ins().bnot(b0);
                let (k1, k0) = if is_and {
                    (b.ins().band(a1, b1), b.ins().bor(a0, b0))
                } else {
                    (b.ins().bor(a1, b1), b.ins().band(a0, b0))
                };
                let x = b.ins().bor(xa, xb);
                let k = b.ins().bor(k0, k1);
                let rx = b.ins().band_not(x, k);
                let rv = b.ins().bor(k1, rx);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::Xor1 { d, a, b: bb } => {
                let (va, xa, vb, xb) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                let x = b.ins().bor(xa, xb);
                let v = b.ins().bxor(va, vb);
                let v = b.ins().bor(v, x);
                st(b, d, v);
                st(b, d + 1, x);
                goto_next(b);
            }
            Op::Xnor1 { d, a, b: bb, w } => {
                let m = c64(b, mk(w));
                let (va, xa, vb, xb) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                let x = b.ins().bor(xa, xb);
                let v = b.ins().bxor(va, vb);
                let v = b.ins().bnot(v);
                let v = b.ins().bor(v, x);
                let v = b.ins().band(v, m);
                st(b, d, v);
                st(b, d + 1, x);
                goto_next(b);
            }
            Op::Add1 { d, a, b: bb, w } | Op::Sub1 { d, a, b: bb, w } | Op::Mul1 { d, a, b: bb, w } => {
                let m = c64(b, mk(w));
                let (va, xa, vb, xb) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                let x = b.ins().bor(xa, xb);
                let r = match op {
                    Op::Add1 { .. } => b.ins().iadd(va, vb),
                    Op::Sub1 { .. } => b.ins().isub(va, vb),
                    _ => b.ins().imul(va, vb),
                };
                let r = b.ins().band(r, m);
                let known = b.ins().icmp_imm(IntCC::Equal, x, 0);
                let zero = c64(b, 0);
                let rv = b.ins().select(known, r, m);
                let rx = b.ins().select(known, zero, m);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::Eq1 { d, a, b: bb } | Op::Ne1 { d, a, b: bb } => {
                let ne = matches!(op, Op::Ne1 { .. }) as u64;
                let (va, xa, vb, xb) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                let x = b.ins().bor(xa, xb);
                let known = b.ins().icmp_imm(IntCC::Equal, x, 0);
                let eq = b.ins().icmp(IntCC::Equal, va, vb);
                let eq = b2i(b, eq);
                let rk = b.ins().bxor_imm(eq, ne as i64);
                let diff = b.ins().bxor(va, vb);
                let diff = b.ins().band_not(diff, x);
                let kd = b.ins().icmp_imm(IntCC::NotEqual, diff, 0);
                let one = c64(b, 1);
                let zero = c64(b, 0);
                let nev = c64(b, ne);
                let unk_v = b.ins().select(kd, nev, one);
                let unk_x = b.ins().select(kd, zero, one);
                let rv = b.ins().select(known, rk, unk_v);
                let rx = b.ins().select(known, zero, unk_x);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::CaseEq1 { d, a, b: bb } | Op::CaseNe1 { d, a, b: bb } => {
                let (va, xa, vb, xb) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                let dv = b.ins().bxor(va, vb);
                let dx = b.ins().bxor(xa, xb);
                let dd = b.ins().bor(dv, dx);
                let cc = if matches!(op, Op::CaseEq1 { .. }) { IntCC::Equal } else { IntCC::NotEqual };
                let r = b.ins().icmp_imm(cc, dd, 0);
                let r = b2i(b, r);
                let zero = c64(b, 0);
                st(b, d, r);
                st(b, d + 1, zero);
                goto_next(b);
            }
            Op::Lt1 { d, a, b: bb } | Op::Le1 { d, a, b: bb } => {
                let (va, xa, vb, xb) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                let x = b.ins().bor(xa, xb);
                let known = b.ins().icmp_imm(IntCC::Equal, x, 0);
                let cc = if matches!(op, Op::Lt1 { .. }) { IntCC::UnsignedLessThan } else { IntCC::UnsignedLessThanOrEqual };
                let r = b.ins().icmp(cc, va, vb);
                let r = b2i(b, r);
                let one = c64(b, 1);
                let zero = c64(b, 0);
                let rv = b.ins().select(known, r, one);
                let rx = b.ins().select(known, zero, one);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::Shl1 { d, a, b: bb, w } | Op::Shr1 { d, a, b: bb, w } => {
                let m = c64(b, mk(w));
                let (va, xa, k, xk) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                let big = b.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, k, w as i64);
                let unk = b.ins().icmp_imm(IntCC::NotEqual, xk, 0);
                let zero = c64(b, 0);
                let (sa, sx) = if matches!(op, Op::Shl1 { .. }) {
                    let sa = b.ins().ishl(va, k);
                    let sx = b.ins().ishl(xa, k);
                    (b.ins().band(sa, m), b.ins().band(sx, m))
                } else {
                    (b.ins().ushr(va, k), b.ins().ushr(xa, k))
                };
                let sa = b.ins().select(big, zero, sa);
                let sx = b.ins().select(big, zero, sx);
                let rv = b.ins().select(unk, m, sa);
                let rx = b.ins().select(unk, m, sx);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::Cat2 { d, hi, lo, sh } => {
                for k in 0..2 {
                    let h = ld(b, hi + k);
                    let l = ld(b, lo + k);
                    let h = b.ins().ishl_imm(h, sh as i64);
                    let v = b.ins().bor(h, l);
                    st(b, d + k, v);
                }
                goto_next(b);
            }
            Op::Mux1 { d, c, t, f } => {
                let (ca, cb) = (ld(b, c), ld(b, c + 1));
                let (ta, tb, fa, fb) = (ld(b, t), ld(b, t + 1), ld(b, f), ld(b, f + 1));
                let c1 = b.ins().band_not(ca, cb);
                let c1 = b.ins().icmp_imm(IntCC::NotEqual, c1, 0);
                let c0 = b.ins().icmp_imm(IntCC::Equal, cb, 0);
                let x = b.ins().bor(tb, fb);
                let df = b.ins().bxor(ta, fa);
                let x = b.ins().bor(x, df);
                let mv = b.ins().bor(ta, x);
                let v0 = b.ins().select(c0, fa, mv);
                let x0 = b.ins().select(c0, fb, x);
                let rv = b.ins().select(c1, ta, v0);
                let rx = b.ins().select(c1, tb, x0);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::Bool1 { d, s } => {
                let (a, x) = (ld(b, s), ld(b, s + 1));
                let k1 = b.ins().band_not(a, x);
                let t = b.ins().icmp_imm(IntCC::NotEqual, k1, 0);
                let known0 = b.ins().icmp_imm(IntCC::Equal, x, 0);
                let one = c64(b, 1);
                let zero = c64(b, 0);
                // true: (1,0); all known 0: (0,0); else (1,1)
                let uv = b.ins().select(known0, zero, one);
                let rv = b.ins().select(t, one, uv);
                let rx = b.ins().select(t, zero, uv);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::RedAnd1 { d, s, w } => {
                let m = c64(b, mk(w));
                let (a, x) = (ld(b, s), ld(b, s + 1));
                let ax = b.ins().bor(a, x);
                let z = b.ins().bnot(ax);
                let z = b.ins().band(z, m);
                let has0 = b.ins().icmp_imm(IntCC::NotEqual, z, 0);
                let unk = b.ins().icmp_imm(IntCC::NotEqual, x, 0);
                let one = c64(b, 1);
                let zero = c64(b, 0);
                let ux = b.ins().select(unk, one, zero);
                let rv = b.ins().select(has0, zero, one);
                let rx = b.ins().select(has0, zero, ux);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::RedXor1 { d, s } => {
                let (a, x) = (ld(b, s), ld(b, s + 1));
                let pc = b.ins().popcnt(a);
                let par = b.ins().band_imm(pc, 1);
                let unk = b.ins().icmp_imm(IntCC::NotEqual, x, 0);
                let one = c64(b, 1);
                let zero = c64(b, 0);
                let rv = b.ins().select(unk, one, par);
                let rx = b.ins().select(unk, one, zero);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::Jmp { to } => {
                let t = if (to as usize) < n { blocks[to as usize] } else { exit };
                b.ins().jump(t, &[]);
            }
            Op::Jz1 { c, to } => {
                let (a, x) = (ld(b, c), ld(b, c + 1));
                let t = b.ins().band_not(a, x);
                let target = if (to as usize) < n { blocks[to as usize] } else { exit };
                b.ins().brif(t, next, &[], target, &[]);
            }
            Op::Store1 { sig, s, slot } => {
                let (a, x) = (ld(b, s), ld(b, s + 1));
                let (oa, ox) = (ld(b, slot), ld(b, slot + 1));
                let da = b.ins().bxor(a, oa);
                let dx = b.ins().bxor(x, ox);
                let dd = b.ins().bor(da, dx);
                let changed = b.create_block();
                b.ins().brif(dd, changed, &[], next, &[]);
                b.switch_to_block(changed);
                st(b, slot, a);
                st(b, slot + 1, x);
                let la = b.ins().band_imm(oa, 1);
                let lx = b.ins().band_imm(ox, 1);
                let lx = b.ins().ishl_imm(lx, 1);
                let old = b.ins().bor(la, lx);
                let old = b.ins().ireduce(i32t, old);
                b.ins().call(r.notify, &[sim, sig_v[sig as usize], old]);
                b.ins().jump(next, &[]);
            }
            Op::Nba1 { sig, s, slot } => {
                let (a, x) = (ld(b, s), ld(b, s + 1));
                let sl = b.ins().ireduce(i32t, slot_no[(slot >> 1) as usize]);
                let sg = sig_v[sig as usize];
                // inline append to the NBA queue; call out only when it is full
                let len = b.ins().load(i64t, fl, hot, 8);
                let cap = b.ins().load(i64t, fl, hot, 16);
                let room = b.ins().icmp(IntCC::UnsignedLessThan, len, cap);
                let fast = b.create_block();
                let slow = b.create_block();
                b.ins().brif(room, fast, &[], slow, &[]);
                b.switch_to_block(fast);
                let base = b.ins().load(i64t, fl, hot, 0);
                let off = b.ins().imul_imm(len, 24);
                let e = b.ins().iadd(base, off);
                b.ins().store(fl, sg, e, 0);
                b.ins().store(fl, sl, e, 4);
                b.ins().store(fl, a, e, 8);
                b.ins().store(fl, x, e, 16);
                let len1 = b.ins().iadd_imm(len, 1);
                b.ins().store(fl, len1, hot, 8);
                b.ins().jump(next, &[]);
                b.switch_to_block(slow);
                b.ins().call(r.nba, &[sim, sg, sl, a, x]);
                b.ins().jump(next, &[]);
            }
            Op::Wait { ctl } => {
                // inline: record.pc = next, record.wait = ctl
                let np = c32(b, i as u32 + 1);
                b.ins().store(fl, np, prec, 0);
                b.ins().store(fl, ctl_v[ctl as usize], prec, 4);
                b.ins().jump(exit, &[]);
            }
            Op::Delay { s } => {
                let sv = b.ins().ireduce(i32t, slot_no[(s >> 1) as usize]);
                let np = c32(b, i as u32 + 1);
                b.ins().call(r.delay, &[sim, p_v, sv, np]);
                b.ins().jump(exit, &[]);
            }
            Op::Halt => {
                let np = c32(b, i as u32);
                b.ins().call(r.halt, &[sim, p_v, np]);
                b.ins().jump(exit, &[]);
            }
        }
    }
}
