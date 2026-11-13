//! Cranelift JIT: compiles each process's lowered ops to native code.
//!
//! A compiled process is `extern "C" fn(sim, mem, pc)`. It starts at `pc` (through a dispatch
//! switch), runs until it suspends and returns. Single-word ops are emitted inline as machine
//! code operating on the flat memory; everything that touches the scheduler (wait, delay, NBA,
//! change notification) and every wide/generic instruction calls back into the runtime.

use crate::fast::{mk, Op};
use crate::{nw, tmask, Instr};
use cranelift_frontend::Variable;
use std::cell::RefCell;
use std::collections::HashMap;
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
    /// per slot index: kept in SSA variables (a scratch temporary no callback touches)
    promoted: Vec<bool>,
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
    let shape = Shape {
        ops: out,
        promoted: Vec::new(),
        nslots: slots.len() as u32,
        nsigs: sigs.len() as u32,
        nctls: ctls.len() as u32,
    };
    (shape, table)
}

/// Runtime entry points called from generated code.
pub struct Callbacks {
    pub notify: unsafe extern "C" fn(*mut Sim, u32, u32),
    pub nba: unsafe extern "C" fn(*mut Sim, u32, u32, u64, u64),
    /// (sim, signal, value slot, width): non-blocking store of a wide value read from memory
    pub nba_wide: unsafe extern "C" fn(*mut Sim, u32, u32, u32),
    /// (sim, array, index value, index unknown, value, unknown): blocking element store
    pub store_elem: unsafe extern "C" fn(*mut Sim, u32, u64, u64, u64, u64),
    /// (sim, array, index value, index unknown, lsb | w << 8, value, unknown)
    pub store_elem_part: unsafe extern "C" fn(*mut Sim, u32, u64, u64, u32, u64, u64),
    pub nba_elem: unsafe extern "C" fn(*mut Sim, u32, u64, u64, u64, u64),
    /// (sim, signal, slot, w | lsb << 8, words per plane, value, unknown)
    pub store_part_wide: unsafe extern "C" fn(*mut Sim, u32, u32, u32, u32, u64, u64),
    pub gen: unsafe extern "C" fn(*mut Sim, u32, u32) -> u64,
    pub wait: unsafe extern "C" fn(*mut Sim, u32, u32, u32),
    pub delay: unsafe extern "C" fn(*mut Sim, u32, u32, u32),
    pub halt: unsafe extern "C" fn(*mut Sim, u32, u32),
}

struct Imports {
    notify: FuncId,
    nba: FuncId,
    nba_wide: FuncId,
    store_elem: FuncId,
    store_elem_part: FuncId,
    nba_elem: FuncId,
    store_part_wide: FuncId,
    gen: FuncId,
    delay: FuncId,
    halt: FuncId,
}

struct Refs {
    notify: FuncRef,
    nba: FuncRef,
    nba_wide: FuncRef,
    store_elem: FuncRef,
    store_elem_part: FuncRef,
    nba_elem: FuncRef,
    store_part_wide: FuncRef,
    gen: FuncRef,
    delay: FuncRef,
    halt: FuncRef,
}

/// `proc_size`: size of the runtime's process record (pc at offset 0, wait at offset 4).
/// `orig`: the original instructions (for the slots generic callbacks use); `scratch`: size of the
/// scratch region of temporaries at the start of memory.
pub fn compile(
    procs: &[Vec<Op>],
    orig: &[Vec<Instr>],
    scratch: u32,
    cb: &Callbacks,
    proc_size: u32,
) -> Result<Jit, String> {
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
    jb.symbol("fx_nba_wide", cb.nba_wide as *const u8);
    jb.symbol("fx_store_elem", cb.store_elem as *const u8);
    jb.symbol("fx_store_elem_part", cb.store_elem_part as *const u8);
    jb.symbol("fx_nba_elem", cb.nba_elem as *const u8);
    jb.symbol("fx_store_part_wide", cb.store_part_wide as *const u8);
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
    let s_store_elem = mk_sig(&module, &[ptr, i32t, i64t, i64t, i64t, i64t], None);
    let s_store_elem_part = mk_sig(&module, &[ptr, i32t, i64t, i64t, i32t, i64t, i64t], None);
    let s_store_part_wide = mk_sig(&module, &[ptr, i32t, i32t, i32t, i32t, i64t, i64t], None);
    let imports = Imports {
        notify: decl(&mut module, "fx_notify", &s_notify)?,
        nba: decl(&mut module, "fx_nba", &s_nba)?,
        nba_wide: decl(&mut module, "fx_nba_wide", &s_wait)?,
        store_elem: decl(&mut module, "fx_store_elem", &s_store_elem)?,
        store_elem_part: decl(&mut module, "fx_store_elem_part", &s_store_elem_part)?,
        nba_elem: decl(&mut module, "fx_nba_elem", &s_store_elem)?,
        store_part_wide: decl(&mut module, "fx_store_part_wide", &s_store_part_wide)?,
        gen: decl(&mut module, "fx_gen", &s_gen)?,
        delay: decl(&mut module, "fx_delay", &s_wait)?,
        halt: decl(&mut module, "fx_halt", &s_halt)?,
    };
    let proc_sig = mk_sig(&module, &[ptr, ptr, i32t, ptr, ptr], None);

    let mut ctx = module.make_context();
    let mut fbc = FunctionBuilderContext::new();
    let mut shape_ids: HashMap<(Vec<Op>, Vec<bool>), FuncId> = HashMap::new();
    let mut ids = Vec::with_capacity(procs.len());
    let mut tables = Vec::new();
    let mut table_off = Vec::with_capacity(procs.len());
    for (p, ops) in procs.iter().enumerate() {
        let (mut shape, table) = normalise(p as u32, ops);
        // slots the runtime reads or writes in memory on behalf of this process
        let mut in_mem: Vec<u32> = Vec::new();
        for (pc, op) in ops.iter().enumerate() {
            match *op {
                // ops that may fall back to the generic path use memory, not registers
                Op::Gen | Op::GenJ | Op::ArrRead1 { .. } | Op::DynSlice1 { .. } => {
                    in_mem.extend(crate::instr_slots(&orig[p][pc]))
                }
                Op::Delay { s } | Op::WNba { s, .. } => in_mem.push(s),
                _ => {}
            }
        }
        let slots = &table[1..1 + shape.nslots as usize];
        shape.promoted = slots.iter().map(|&s| s < scratch && !in_mem.contains(&s)).collect();
        table_off.push(tables.len());
        tables.extend_from_slice(&table);
        let key = (shape.ops.clone(), shape.promoted.clone());
        if let Some(&id) = shape_ids.get(&key) {
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
                nba_wide: module.declare_func_in_func(imports.nba_wide, b.func),
                store_elem: module.declare_func_in_func(imports.store_elem, b.func),
                store_elem_part: module.declare_func_in_func(imports.store_elem_part, b.func),
                nba_elem: module.declare_func_in_func(imports.nba_elem, b.func),
                store_part_wide: module.declare_func_in_func(imports.store_part_wide, b.func),
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
        shape_ids.insert(key, id);
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
    // Slot fields are encoded table indices 2k (+1 for the unknown-bits word of a narrow value).
    // Word j of slot k lives in memory at addr[k] + 8j, or in an SSA variable if k is promoted.
    let acc = RefCell::new(Acc { addr, promoted: shape.promoted.clone(), vars: HashMap::new() });
    let ldw = |b: &mut FunctionBuilder, f: u32, j: u32| -> Value { acc.borrow_mut().ld(b, f >> 1, (f & 1) + j) };
    let stw = |b: &mut FunctionBuilder, f: u32, j: u32, v: Value| acc.borrow_mut().st(b, f >> 1, (f & 1) + j, v);
    let ld = |b: &mut FunctionBuilder, slot: u32| -> Value { ldw(b, slot, 0) };
    let st = |b: &mut FunctionBuilder, slot: u32, v: Value| stw(b, slot, 0, v);
    let c64 = |b: &mut FunctionBuilder, v: u64| -> Value { b.ins().iconst(i64t, v as i64) };
    let c32 = |b: &mut FunctionBuilder, v: u32| -> Value { b.ins().iconst(i32t, v as i64) };
    let b2i = |b: &mut FunctionBuilder, c: Value| -> Value { b.ins().uextend(i64t, c) };

    // call the generic path for the instruction at `pc` and continue at `next` (or return if it
    // suspended)
    let gen_call = |b: &mut FunctionBuilder, pc: u32, next: Block| {
        let pcv = b.ins().iconst(i32t, pc as i64);
        let call = b.ins().call(r.gen, &[sim, p_v, pcv]);
        let ret = b.inst_results(call)[0];
        let susp = b.ins().icmp_imm(IntCC::Equal, ret, -1);
        b.ins().brif(susp, exit, &[], next, &[]);
    };
    for (i, &op) in ops.iter().enumerate() {
        b.switch_to_block(blocks[i]);
        let next = if i + 1 < n { blocks[i + 1] } else { exit };
        let goto_next = |b: &mut FunctionBuilder| {
            b.ins().jump(next, &[]);
        };
        match op {
            Op::Gen | Op::GenJ => {
                let pcv = c32(b, i as u32);
                let call = b.ins().call(r.gen, &[sim, p_v, pcv]);
                let ret = b.inst_results(call)[0];
                let susp = b.ins().icmp_imm(IntCC::Equal, ret, -1);
                let cont = b.create_block();
                b.ins().brif(susp, exit, &[], cont, &[]);
                b.switch_to_block(cont);
                if matches!(op, Op::GenJ) {
                    let npc = b.ins().ireduce(i32t, ret);
                    b.ins().jump(dispatch, &[npc.into()]);
                } else {
                    b.ins().jump(next, &[]);
                }
            }
            Op::ArrRead1 { d, base, ix, depth } => {
                let (e, xe) = (ld(b, ix), ld(b, ix + 1));
                let known = b.ins().icmp_imm(IntCC::Equal, xe, 0);
                let inr = b.ins().icmp_imm(IntCC::UnsignedLessThan, e, depth as i64);
                let ok = b.ins().band(known, inr);
                let (fast, slow) = (b.create_block(), b.create_block());
                b.ins().brif(ok, fast, &[], slow, &[]);
                b.switch_to_block(fast);
                let base_addr = acc.borrow().addr[(base >> 1) as usize];
                let off = b.ins().ishl_imm(e, 4);
                let ea = b.ins().iadd(base_addr, off);
                let va = b.ins().load(i64t, fl, ea, 0);
                let vb = b.ins().load(i64t, fl, ea, 8);
                st(b, d, va);
                st(b, d + 1, vb);
                b.ins().jump(next, &[]);
                b.switch_to_block(slow);
                gen_call(b, i as u32, next);
            }
            Op::DynSlice1 { d, s, ix, w, sw } => {
                let (o, xo) = (ld(b, ix), ld(b, ix + 1));
                let known = b.ins().icmp_imm(IntCC::Equal, xo, 0);
                let inr = b.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, o, (sw - w) as i64);
                let ok = b.ins().band(known, inr);
                let (fast, slow) = (b.create_block(), b.create_block());
                b.ins().brif(ok, fast, &[], slow, &[]);
                b.switch_to_block(fast);
                let m = c64(b, mk(w));
                for k in 0..2 {
                    let v = ld(b, s + k);
                    let v = b.ins().ushr(v, o);
                    let v = b.ins().band(v, m);
                    st(b, d + k, v);
                }
                b.ins().jump(next, &[]);
                b.switch_to_block(slow);
                gen_call(b, i as u32, next);
            }
            Op::LNot1 { d, s } => {
                let (a, x) = (ld(b, s), ld(b, s + 1));
                let k1 = b.ins().band_not(a, x);
                let t = b.ins().icmp_imm(IntCC::NotEqual, k1, 0);
                let known = b.ins().icmp_imm(IntCC::Equal, x, 0);
                let (one, zero) = (c64(b, 1), c64(b, 0));
                // a known 1 bit: 0; all known 0: 1; else x
                let uv = one;
                let ux = b.ins().select(known, zero, one);
                let rv = b.ins().select(t, zero, uv);
                let rx = b.ins().select(t, zero, ux);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::LtS1 { d, a, b: bb, w } | Op::LeS1 { d, a, b: bb, w } => {
                let (va, xa, vb, xb) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                let sh = 64 - w as i64;
                let (sa, sb) = if sh > 0 {
                    let x = b.ins().ishl_imm(va, sh);
                    let y = b.ins().ishl_imm(vb, sh);
                    (b.ins().sshr_imm(x, sh), b.ins().sshr_imm(y, sh))
                } else {
                    (va, vb)
                };
                let cc = if matches!(op, Op::LtS1 { .. }) { IntCC::SignedLessThan } else { IntCC::SignedLessThanOrEqual };
                let r = b.ins().icmp(cc, sa, sb);
                let r = b2i(b, r);
                let x = b.ins().bor(xa, xb);
                let known = b.ins().icmp_imm(IntCC::Equal, x, 0);
                let (one, zero) = (c64(b, 1), c64(b, 0));
                let rv = b.ins().select(known, r, one);
                let rx = b.ins().select(known, zero, one);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::Resolve1 { d, a, b: bb, w } => {
                let (va, xa, vb, xb) = (ld(b, a), ld(b, a + 1), ld(b, bb), ld(b, bb + 1));
                let (rv, rx) = resolve(b, va, xa, vb, xb, mk(w));
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::WResolve { d, a, b: bb, w } => {
                let n = nw(w as u32) as u32;
                for i in 0..n {
                    let (va, xa, vb, xb) = (ldw(b, a, i), ldw(b, a, n + i), ldw(b, bb, i), ldw(b, bb, n + i));
                    let m = if i == n - 1 { tmask(w as u32) } else { !0 };
                    let (rv, rx) = resolve(b, va, xa, vb, xb, m);
                    stw(b, d, i, rv);
                    stw(b, d, n + i, rx);
                }
                goto_next(b);
            }
            Op::StorePart1 { sig, s, slot, lsb, w } => {
                let m = mk(w) << lsb;
                let (oa, ox) = (ld(b, slot), ld(b, slot + 1));
                let mut nv = Vec::new();
                for (k, old) in [(0, oa), (1, ox)] {
                    let v = ld(b, s + k);
                    let v = b.ins().ishl_imm(v, lsb as i64);
                    let v = b.ins().band_imm(v, m as i64);
                    let o = b.ins().band_imm(old, !m as i64);
                    nv.push(b.ins().bor(o, v));
                }
                let da = b.ins().bxor(nv[0], oa);
                let dx = b.ins().bxor(nv[1], ox);
                let dd = b.ins().bor(da, dx);
                let changed = b.create_block();
                b.ins().brif(dd, changed, &[], next, &[]);
                b.switch_to_block(changed);
                st(b, slot, nv[0]);
                st(b, slot + 1, nv[1]);
                let la = b.ins().band_imm(oa, 1);
                let lx = b.ins().band_imm(ox, 1);
                let lx = b.ins().ishl_imm(lx, 1);
                let old = b.ins().bor(la, lx);
                let old = b.ins().ireduce(i32t, old);
                b.ins().call(r.notify, &[sim, sig_v[sig as usize], old]);
                b.ins().jump(next, &[]);
            }
            Op::StoreElem1 { sig, s, ix } | Op::NbaElem1 { sig, s, ix } => {
                let (ea, eb, va, vb) = (ld(b, ix), ld(b, ix + 1), ld(b, s), ld(b, s + 1));
                let f = if matches!(op, Op::StoreElem1 { .. }) { r.store_elem } else { r.nba_elem };
                b.ins().call(f, &[sim, sig_v[sig as usize], ea, eb, va, vb]);
                goto_next(b);
            }
            Op::StoreElemPart1 { sig, s, ix, lsb, w } => {
                let (ea, eb, va, vb) = (ld(b, ix), ld(b, ix + 1), ld(b, s), ld(b, s + 1));
                let lw = c32(b, lsb as u32 | (w as u32) << 8);
                b.ins().call(r.store_elem_part, &[sim, sig_v[sig as usize], ea, eb, lw, va, vb]);
                goto_next(b);
            }
            Op::StorePartW { sig, s, slot, lsb, w, n } => {
                let (va, vb) = (ld(b, s), ld(b, s + 1));
                let sl = b.ins().ireduce(i32t, slot_no[(slot >> 1) as usize]);
                let lw = c32(b, w as u32 | lsb << 8);
                let nv = c32(b, n as u32);
                b.ins().call(r.store_part_wide, &[sim, sig_v[sig as usize], sl, lw, nv, va, vb]);
                goto_next(b);
            }
            Op::Ins1 { d, s, off, w } => {
                let m = mk(w);
                for k in 0..2 {
                    let dv = ld(b, d + k);
                    let sv = ld(b, s + k);
                    let sv = b.ins().band_imm(sv, m as i64);
                    let sv = b.ins().ishl_imm(sv, off as i64);
                    let dv = b.ins().band_imm(dv, !(m << off) as i64);
                    let v = b.ins().bor(dv, sv);
                    st(b, d + k, v);
                }
                goto_next(b);
            }
            Op::WCopy { d, s, w } => {
                for j in 0..2 * nw(w as u32) as u32 {
                    let v = ldw(b, s, j);
                    stw(b, d, j, v);
                }
                goto_next(b);
            }
            Op::WAnd { d, a, b: bb, w } | Op::WOr { d, a, b: bb, w } | Op::WXor { d, a, b: bb, w } => {
                let n = nw(w as u32) as u32;
                for i in 0..n {
                    let (va, xa, vb, xb) = (ldw(b, a, i), ldw(b, a, n + i), ldw(b, bb, i), ldw(b, bb, n + i));
                    let (rv, rx) = match op {
                        Op::WXor { .. } => {
                            let x = b.ins().bor(xa, xb);
                            let v = b.ins().bxor(va, vb);
                            (b.ins().bor(v, x), x)
                        }
                        _ => and_or(b, matches!(op, Op::WAnd { .. }), va, xa, vb, xb),
                    };
                    stw(b, d, i, rv);
                    stw(b, d, n + i, rx);
                }
                goto_next(b);
            }
            Op::WNot { d, s, w } => {
                let n = nw(w as u32) as u32;
                for i in 0..n {
                    let (va, xa) = (ldw(b, s, i), ldw(b, s, n + i));
                    let m = if i == n - 1 { tmask(w as u32) } else { !0 };
                    let nv = b.ins().bnot(va);
                    let v = b.ins().bor(nv, xa);
                    let v = b.ins().band_imm(v, m as i64);
                    stw(b, d, i, v);
                    stw(b, d, n + i, xa);
                }
                goto_next(b);
            }
            Op::WAdd { d, a, b: bb, w } | Op::WSub { d, a, b: bb, w } => {
                let n = nw(w as u32) as u32;
                let add = matches!(op, Op::WAdd { .. });
                let mut anyx = c64(b, 0);
                let mut carry = c64(b, 0);
                let mut res = Vec::new();
                for i in 0..n {
                    let (va, xa, vb, xb) = (ldw(b, a, i), ldw(b, a, n + i), ldw(b, bb, i), ldw(b, bb, n + i));
                    let x = b.ins().bor(xa, xb);
                    anyx = b.ins().bor(anyx, x);
                    let (r, c) = if add {
                        let s1 = b.ins().iadd(va, vb);
                        let c1 = b.ins().icmp(IntCC::UnsignedLessThan, s1, va);
                        let s2 = b.ins().iadd(s1, carry);
                        let c2 = b.ins().icmp(IntCC::UnsignedLessThan, s2, s1);
                        let c = b.ins().bor(c1, c2);
                        (s2, c)
                    } else {
                        let d1 = b.ins().isub(va, vb);
                        let b1 = b.ins().icmp(IntCC::UnsignedLessThan, va, vb);
                        let d2 = b.ins().isub(d1, carry);
                        let b2 = b.ins().icmp(IntCC::UnsignedLessThan, d1, carry);
                        let c = b.ins().bor(b1, b2);
                        (d2, c)
                    };
                    carry = b2i(b, c);
                    res.push(r);
                }
                let known = b.ins().icmp_imm(IntCC::Equal, anyx, 0);
                let zero = c64(b, 0);
                for i in 0..n {
                    let m = c64(b, if i == n - 1 { tmask(w as u32) } else { !0 });
                    let r = b.ins().band(res[i as usize], m);
                    let rv = b.ins().select(known, r, m);
                    let rx = b.ins().select(known, zero, m);
                    stw(b, d, i, rv);
                    stw(b, d, n + i, rx);
                }
                goto_next(b);
            }
            Op::WEq { d, a, b: bb, w } | Op::WNe { d, a, b: bb, w } => {
                let n = nw(w as u32) as u32;
                let ne = matches!(op, Op::WNe { .. }) as u64;
                let (mut anyx, mut diff, mut kd) = (c64(b, 0), c64(b, 0), c64(b, 0));
                for i in 0..n {
                    let (va, xa, vb, xb) = (ldw(b, a, i), ldw(b, a, n + i), ldw(b, bb, i), ldw(b, bb, n + i));
                    let x = b.ins().bor(xa, xb);
                    anyx = b.ins().bor(anyx, x);
                    let dv = b.ins().bxor(va, vb);
                    diff = b.ins().bor(diff, dv);
                    let k = b.ins().band_not(dv, x);
                    kd = b.ins().bor(kd, k);
                }
                let known = b.ins().icmp_imm(IntCC::Equal, anyx, 0);
                let eq = b.ins().icmp_imm(IntCC::Equal, diff, 0);
                let eq = b2i(b, eq);
                let rk = b.ins().bxor_imm(eq, ne as i64);
                let kd = b.ins().icmp_imm(IntCC::NotEqual, kd, 0);
                let (one, zero, nev) = (c64(b, 1), c64(b, 0), c64(b, ne));
                let uv = b.ins().select(kd, nev, one);
                let ux = b.ins().select(kd, zero, one);
                let rv = b.ins().select(known, rk, uv);
                let rx = b.ins().select(known, zero, ux);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::WCaseEq { d, a, b: bb, w } | Op::WCaseNe { d, a, b: bb, w } => {
                let n = nw(w as u32) as u32;
                let mut dd = c64(b, 0);
                for j in 0..2 * n {
                    let (va, vb) = (ldw(b, a, j), ldw(b, bb, j));
                    let x = b.ins().bxor(va, vb);
                    dd = b.ins().bor(dd, x);
                }
                let cc = if matches!(op, Op::WCaseEq { .. }) { IntCC::Equal } else { IntCC::NotEqual };
                let r = b.ins().icmp_imm(cc, dd, 0);
                let r = b2i(b, r);
                let zero = c64(b, 0);
                st(b, d, r);
                st(b, d + 1, zero);
                goto_next(b);
            }
            Op::WLt { d, a, b: bb, w } | Op::WLe { d, a, b: bb, w } => {
                let n = nw(w as u32) as u32;
                let mut anyx = c64(b, 0);
                // fold from the least significant word: acc = a_i < b_i || (a_i == b_i && acc)
                let mut acc = c64(b, matches!(op, Op::WLe { .. }) as u64);
                for i in 0..n {
                    let (va, xa, vb, xb) = (ldw(b, a, i), ldw(b, a, n + i), ldw(b, bb, i), ldw(b, bb, n + i));
                    let x = b.ins().bor(xa, xb);
                    anyx = b.ins().bor(anyx, x);
                    let lt = b.ins().icmp(IntCC::UnsignedLessThan, va, vb);
                    let lt = b2i(b, lt);
                    let eq = b.ins().icmp(IntCC::Equal, va, vb);
                    let eq = b2i(b, eq);
                    let e = b.ins().band(eq, acc);
                    acc = b.ins().bor(lt, e);
                }
                let known = b.ins().icmp_imm(IntCC::Equal, anyx, 0);
                let (one, zero) = (c64(b, 1), c64(b, 0));
                let rv = b.ins().select(known, acc, one);
                let rx = b.ins().select(known, zero, one);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::WSlice { d, s, lsb, sw, w } => {
                let (ns, nd) = (nw(sw as u32) as u32, nw(w as u32) as u32);
                for p in 0..2 {
                    for i in 0..nd {
                        let bit = lsb + 64 * i;
                        let len = (w as u32 - 64 * i).min(64);
                        let (idx, off) = (bit / 64, bit % 64);
                        let mut v = ldw(b, s, p * ns + idx);
                        if off > 0 {
                            v = b.ins().ushr_imm(v, off as i64);
                            if off + len > 64 && idx + 1 < ns {
                                let hi = ldw(b, s, p * ns + idx + 1);
                                let hi = b.ins().ishl_imm(hi, (64 - off) as i64);
                                v = b.ins().bor(v, hi);
                            }
                        }
                        if len < 64 {
                            v = b.ins().band_imm(v, mk(len as u8) as i64);
                        }
                        stw(b, d, p * nd + i, v);
                    }
                }
                goto_next(b);
            }
            Op::WResize { d, s, sw, dw, sign } => {
                let (sw, dw) = (sw as u32, dw as u32);
                let (ns, nd) = (nw(sw) as u32, nw(dw) as u32);
                let ext = sign && dw > sw;
                for p in 0..2 {
                    let fill = if ext {
                        let top = ldw(b, s, p * ns + ns - 1);
                        let msb = (sw - 1) % 64;
                        let t = b.ins().ishl_imm(top, (63 - msb) as i64);
                        Some(b.ins().sshr_imm(t, 63))
                    } else {
                        None
                    };
                    for i in 0..nd {
                        let mut v = if i < ns {
                            let v = ldw(b, s, p * ns + i);
                            match fill {
                                Some(f) if i == ns - 1 && tmask(sw) != !0 => {
                                    let hi = b.ins().band_imm(f, !tmask(sw) as i64);
                                    b.ins().bor(v, hi)
                                }
                                _ => v,
                            }
                        } else {
                            match fill {
                                Some(f) => f,
                                None => c64(b, 0),
                            }
                        };
                        if i == nd - 1 && tmask(dw) != !0 {
                            v = b.ins().band_imm(v, tmask(dw) as i64);
                        }
                        stw(b, d, p * nd + i, v);
                    }
                }
                goto_next(b);
            }
            Op::WIns { d, s, off, dw, sw } => {
                let (nd, ns) = (nw(dw as u32) as u32, nw(sw as u32) as u32);
                let len = sw as u32;
                for p in 0..2 {
                    let mut i = 0;
                    while i < len {
                        let l = (len - i).min(64);
                        let m = mk(l as u8);
                        let v = ldw(b, s, p * ns + i / 64);
                        let pos = off + i;
                        let (idx, o) = (pos / 64, pos % 64);
                        let dv = ldw(b, d, p * nd + idx);
                        let dv = b.ins().band_imm(dv, !(m << o) as i64);
                        let sv = b.ins().ishl_imm(v, o as i64);
                        let nv = b.ins().bor(dv, sv);
                        stw(b, d, p * nd + idx, nv);
                        if o != 0 && o + l > 64 {
                            let dv = ldw(b, d, p * nd + idx + 1);
                            let dv = b.ins().band_imm(dv, !(m >> (64 - o)) as i64);
                            let sv = b.ins().ushr_imm(v, (64 - o) as i64);
                            let nv = b.ins().bor(dv, sv);
                            stw(b, d, p * nd + idx + 1, nv);
                        }
                        i += l;
                    }
                }
                goto_next(b);
            }
            Op::WMux { d, c, t, f, w } => {
                let n = nw(w as u32) as u32;
                let (ca, cb) = (ld(b, c), ld(b, c + 1));
                let c1 = b.ins().band_not(ca, cb);
                let c1 = b.ins().icmp_imm(IntCC::NotEqual, c1, 0);
                let c0 = b.ins().icmp_imm(IntCC::Equal, cb, 0);
                for i in 0..n {
                    let (ta, tb, fa, fb) = (ldw(b, t, i), ldw(b, t, n + i), ldw(b, f, i), ldw(b, f, n + i));
                    let dx = b.ins().bxor(tb, fb);
                    let df = b.ins().bxor(ta, fa);
                    let diff = b.ins().bor(dx, df);
                    let mv = b.ins().bor(ta, diff);
                    let x = b.ins().bor(tb, diff);
                    let v0 = b.ins().select(c0, fa, mv);
                    let x0 = b.ins().select(c0, fb, x);
                    let rv = b.ins().select(c1, ta, v0);
                    let rx = b.ins().select(c1, tb, x0);
                    stw(b, d, i, rv);
                    stw(b, d, n + i, rx);
                }
                goto_next(b);
            }
            Op::WBool { d, s, w } => {
                let n = nw(w as u32) as u32;
                let (mut k1, mut anyx) = (c64(b, 0), c64(b, 0));
                for i in 0..n {
                    let (a, x) = (ldw(b, s, i), ldw(b, s, n + i));
                    let k = b.ins().band_not(a, x);
                    k1 = b.ins().bor(k1, k);
                    anyx = b.ins().bor(anyx, x);
                }
                let t = b.ins().icmp_imm(IntCC::NotEqual, k1, 0);
                let known0 = b.ins().icmp_imm(IntCC::Equal, anyx, 0);
                let (one, zero) = (c64(b, 1), c64(b, 0));
                let uv = b.ins().select(known0, zero, one);
                let rv = b.ins().select(t, one, uv);
                let rx = b.ins().select(t, zero, uv);
                st(b, d, rv);
                st(b, d + 1, rx);
                goto_next(b);
            }
            Op::WStore { sig, s, slot, w } => {
                let n = nw(w as u32) as u32;
                let mut newv = Vec::new();
                let mut oldv = Vec::new();
                let mut dd = c64(b, 0);
                for j in 0..2 * n {
                    let nv = ldw(b, s, j);
                    let ov = ldw(b, slot, j);
                    let x = b.ins().bxor(nv, ov);
                    dd = b.ins().bor(dd, x);
                    newv.push(nv);
                    oldv.push(ov);
                }
                let changed = b.create_block();
                b.ins().brif(dd, changed, &[], next, &[]);
                b.switch_to_block(changed);
                for j in 0..2 * n {
                    stw(b, slot, j, newv[j as usize]);
                }
                let la = b.ins().band_imm(oldv[0], 1);
                let lx = b.ins().band_imm(oldv[n as usize], 1);
                let lx = b.ins().ishl_imm(lx, 1);
                let old = b.ins().bor(la, lx);
                let old = b.ins().ireduce(i32t, old);
                b.ins().call(r.notify, &[sim, sig_v[sig as usize], old]);
                b.ins().jump(next, &[]);
            }
            Op::WNba { sig, s, w } => {
                let sv = b.ins().ireduce(i32t, slot_no[(s >> 1) as usize]);
                let wv = c32(b, w as u32);
                b.ins().call(r.nba_wide, &[sim, sig_v[sig as usize], sv, wv]);
                goto_next(b);
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
                let dx = b.ins().bxor(tb, fb);
                let df = b.ins().bxor(ta, fa);
                let diff = b.ins().bor(dx, df);
                let mv = b.ins().bor(ta, diff);
                let x = b.ins().bor(tb, diff);
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
                // inline: record.pc = next, record.wait = ctl, record.wseq = hot.wseq++
                let np = c32(b, i as u32 + 1);
                b.ins().store(fl, np, prec, 0);
                b.ins().store(fl, ctl_v[ctl as usize], prec, 4);
                let ws = b.ins().load(i64t, fl, hot, 32);
                b.ins().store(fl, ws, prec, 8);
                let ws1 = b.ins().iadd_imm(ws, 1);
                b.ins().store(fl, ws1, hot, 32);
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

/// 4-state AND / OR of one word: (value, unknown)
fn and_or(b: &mut FunctionBuilder, is_and: bool, va: Value, xa: Value, vb: Value, xb: Value) -> (Value, Value) {
    let nxa = b.ins().bnot(xa);
    let nxb = b.ins().bnot(xb);
    let a1 = b.ins().band(va, nxa);
    let b1 = b.ins().band(vb, nxb);
    let a0 = b.ins().bor(va, xa);
    let a0 = b.ins().bnot(a0);
    let b0 = b.ins().bor(vb, xb);
    let b0 = b.ins().bnot(b0);
    let (k1, k0) = if is_and { (b.ins().band(a1, b1), b.ins().bor(a0, b0)) } else { (b.ins().bor(a1, b1), b.ins().band(a0, b0)) };
    let x = b.ins().bor(xa, xb);
    let k = b.ins().bor(k0, k1);
    let rx = b.ins().band_not(x, k);
    (b.ins().bor(k1, rx), rx)
}

/// Access to the words of slot k: memory, or SSA variables for promoted temporaries.
struct Acc {
    addr: Vec<Value>,
    promoted: Vec<bool>,
    vars: HashMap<(u32, u32), Variable>,
}

impl Acc {
    fn var(&mut self, b: &mut FunctionBuilder, k: u32, j: u32) -> Variable {
        *self.vars.entry((k, j)).or_insert_with(|| b.declare_var(types::I64))
    }
    fn ld(&mut self, b: &mut FunctionBuilder, k: u32, j: u32) -> Value {
        if self.promoted[k as usize] {
            let v = self.var(b, k, j);
            return b.use_var(v);
        }
        b.ins().load(types::I64, MemFlags::trusted(), self.addr[k as usize], (j * 8) as i32)
    }
    fn st(&mut self, b: &mut FunctionBuilder, k: u32, j: u32, val: Value) {
        if self.promoted[k as usize] {
            let v = self.var(b, k, j);
            b.def_var(v, val);
            return;
        }
        b.ins().store(MemFlags::trusted(), val, self.addr[k as usize], (j * 8) as i32);
    }
}

/// wire resolution of one word (see fast::resolve1)
fn resolve(b: &mut FunctionBuilder, va: Value, xa: Value, vb: Value, xb: Value, mask: u64) -> (Value, Value) {
    let za = b.ins().band_not(xa, va);
    let zb = b.ins().band_not(xb, vb);
    let take_a = b.ins().band_not(zb, za);
    let take_b = b.ins().band_not(za, zb);
    let nz = b.ins().bor(za, zb);
    let both = b.ins().bnot(nz);
    let dv = b.ins().bxor(va, vb);
    let dx = b.ins().bxor(xa, xb);
    let ne = b.ins().bor(dv, dx);
    let eq = b.ins().bnot(ne);
    let a_eq = b.ins().band(va, eq);
    let v_both = b.ins().bor(a_eq, ne);
    let v_both = b.ins().band(both, v_both);
    let ta = b.ins().band(va, take_a);
    let tb = b.ins().band(vb, take_b);
    let rv = b.ins().bor(ta, tb);
    let rv = b.ins().bor(rv, v_both);
    let ax_eq = b.ins().band(xa, eq);
    let x_both = b.ins().bor(ax_eq, ne);
    let x_both = b.ins().band(both, x_both);
    let tax = b.ins().band(xa, take_a);
    let tbx = b.ins().band(xb, take_b);
    let zz = b.ins().band(za, zb);
    let rx = b.ins().bor(tax, tbx);
    let rx = b.ins().bor(rx, zz);
    let rx = b.ins().bor(rx, x_both);
    let rv = b.ins().band_imm(rv, mask as i64);
    let rx = b.ins().band_imm(rx, mask as i64);
    (rv, rx)
}
