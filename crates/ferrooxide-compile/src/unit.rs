//! Shared subroutines: a function that cannot wait is compiled once into a unit
//! (`Program::units`) that every process calls (`SaveUnit`, `CallUnit`), instead of a copy
//! of its body per call path in each process. Its temporaries live in memory of its own (not
//! the scratch region of the calling process); a recursive call saves the unit's frame.

use crate::expr::SUB_DEPTH;
use crate::codegen::merge_ranges;
use crate::types::Type;
use crate::*;

/// The `proc` of a shared subroutine's code buffer.
const UNIT_PROC: u32 = NONE - 1;

impl<'a> Compiler<'a> {
    /// A call of `f` (declared in scope `fs_def`) through its shared subroutine, if it gets
    /// one: a method, or a recursive or deeply nested call, of a function whose body cannot
    /// wait, fork or start processes. None: the call is compiled as before.
    pub(crate) fn unit_call(
        &mut self,
        f: &'a SubroutineDecl,
        fs_def: usize,
        this: Option<u32>,
        args: &[Expr],
        dflt: &[bool],
        sid: usize,
    ) -> R<Option<(Option<u32>, u32, bool)>> {
        if self.subr.no_units || f.is_task || self.code.pb.last().is_none_or(|p| p.proc == NONE) {
            return Ok(None);
        }
        let method = self.classes.of_scope.contains_key(&fs_def);
        let base = self.subr.proc_base.last().copied().unwrap_or(0).min(self.subr.inline_stack.len());
        let recursive = self.subr.inline_stack[base..].iter().any(|x| x.0 == fs_def && x.1 == f.name);
        if !(method || recursive || self.subr.call_depth >= SUB_DEPTH) || !self.unit_ok(f) {
            return Ok(None);
        }
        let key = (fs_def, if f.automatic { format!("{}#unit", f.name) } else { f.name.clone() });
        let u = match self.subr.units.get(&key) {
            Some(&u) => u,
            None => match self.compile_unit(f, &key)? {
                Some(u) => u,
                None => return Ok(None),
            },
        };
        self.sub_call(f, key, this, args, dflt, sid, false, false, Some(u)).map(Some)
    }

    /// Whether `f`'s body can be a shared subroutine (cached).
    fn unit_ok(&mut self, f: &SubroutineDecl) -> bool {
        let k = f as *const SubroutineDecl;
        if let Some(&ok) = self.subr.unit_ok.get(&k) {
            return ok;
        }
        // (an unpacked array result is returned through its variable: inlined)
        let ok = (f.ret_ty.unpacked.is_empty() || f.ret_ty.heap.is_some()) && unit_stmt(&f.body);
        self.subr.unit_ok.insert(k, ok);
        ok
    }

    /// Compile the shared subroutine of instance `key` of `f`. None: it cannot be one (its
    /// code would wait, or it has unpacked array outputs).
    fn compile_unit(&mut self, f: &'a SubroutineDecl, key: &(usize, String)) -> R<Option<u32>> {
        let id = self.prog.units.len() as u32;
        let name = format!("{}.{}", self.scopes[key.0].name, f.name);
        self.prog.units.push(rt::UnitDef { name: name.clone(), code: vec![Instr::RetUnit { srcs: Box::new([]) }], frame: Vec::new() });
        self.subr.units.insert(key.clone(), id);
        let saved_file = self.file.clone();
        let saved_line = self.line;
        let depth = std::mem::replace(&mut self.subr.call_depth, 0);
        let forks = std::mem::take(&mut self.code.forks);
        let blocks = std::mem::take(&mut self.code.open_blocks);
        let scratch = std::mem::replace(&mut self.code.scratch_top, 0);
        self.code.pb.push(PState { proc: UNIT_PROC, unit: true, ..Default::default() });
        self.subr.proc_base.push(self.subr.inline_stack.len());
        let (start, holes) = (self.prog.mem.len() as u32, self.subr.static_mem.len());
        let mut lo = start;
        let r = self.unit_body(f, key, &mut lo);
        self.subr.proc_base.pop();
        let st = self.pop_pb();
        self.file = saved_file;
        self.line = saved_line;
        self.subr.call_depth = depth;
        self.code.forks = forks;
        self.code.open_blocks = blocks;
        self.code.scratch_top = scratch;
        match r {
            Ok(true) if !st.bad => {}
            r => {
                self.subr.units.remove(key);
                if matches!(r, Ok(_)) {
                    self.subr.unit_ok.insert(f as *const SubroutineDecl, false);
                }
                return r.map(|_| None);
            }
        };
        // the frame: the automatic variables and the memory the body allocated (temporaries,
        // hidden and automatic variables), but not static variables first made here
        let hi = self.prog.mem.len() as u32;
        let mut cut: Vec<(u32, u32)> = self.subr.static_mem[holes..].iter().copied().filter(|r| r.0 >= lo && r.1 <= hi).collect();
        cut.sort_unstable();
        let mut frame = Vec::new();
        let mut at = lo;
        for (a, b) in cut {
            if a > at {
                frame.push((at, a - at));
            }
            at = at.max(b);
        }
        if hi > at {
            frame.push((at, hi - at));
        }
        let fm = self.subr.func_inst[key].mem;
        if f.automatic && fm.1 > fm.0 {
            frame.push((fm.0, fm.1 - fm.0));
        }
        let frame = if frame.is_empty() { frame } else { merge_ranges(frame) };
        self.subr.static_mem.truncate(holes);
        self.subr.static_mem.push((start, hi));
        self.prog.units[id as usize] = rt::UnitDef { name, code: st.code, frame };
        Ok(Some(id))
    }

    /// The body of a shared subroutine, into the open code buffer. Returns false if it
    /// cannot be shared.
    /// `lo`: set to where the memory the body allocates begins.
    fn unit_body(&mut self, f: &'a SubroutineDecl, key: &(usize, String), lo: &mut u32) -> R<bool> {
        self.func_instance(f, key.0, &key.1)?;
        *lo = self.prog.mem.len() as u32;
        let (fscope, ret, ports, this_sig) = {
            let fi = &self.subr.func_inst[key];
            (fi.scope, fi.ret, fi.ports.clone(), fi.this)
        };
        if ret.is_some_and(|r| self.sig(r).depth > 0) || ports.iter().any(|&(d, s)| d != Dir::Input && self.sig(s).depth > 0) {
            return Ok(false);
        }
        self.subr.inline_stack.push((key.0, f.name.clone(), key.clone()));
        self.code.pb.last_mut().unwrap().rets.push((ret, Vec::new()));
        let r = self.unit_stmts(f, key.0, fscope, ret, this_sig);
        let (_, patches) = self.code.pb.last_mut().unwrap().rets.pop().unwrap();
        self.subr.inline_stack.pop();
        r?;
        let end = self.here();
        for p in patches {
            self.patch(p, end);
        }
        let mut srcs: Vec<(u32, u32)> = Vec::new();
        if let Some(r) = ret {
            let s = self.sig(r);
            srcs.push((s.slot, 2 * rt::nw(s.width) as u32));
        }
        for &(dir, ps) in &ports {
            if dir == Dir::Output || dir == Dir::Inout {
                let s = self.sig(ps);
                srcs.push((s.slot, 2 * rt::nw(s.width) as u32));
            }
        }
        self.emit(Instr::RetUnit { srcs: srcs.into_boxed_slice() });
        // (subroutines of the unit itself: calls that are not shared)
        self.finish_subs()?;
        Ok(true)
    }

    fn unit_stmts(&mut self, f: &'a SubroutineDecl, fs_def: usize, fscope: usize, ret: Option<u32>, this_sig: Option<u32>) -> R<()> {
        // an automatic function's queue result starts empty on each call
        if let (true, Some(r)) = (f.automatic, ret) {
            if let Type::Dyn { .. } = self.sig_type(r) {
                let z = self.const_slot(&Const::zero(64));
                self.emit(Instr::Store { t: rt::Target::Whole { sig: r }, s: z, mode: rt::StoreMode::Blocking });
            }
        }
        // a constructor: the base class's constructor and the property initializers first,
        // unless the body starts with `super.new(..)`
        if let (true, Some(t)) = (f.name == "new" && !crate::Compiler::calls_super_new(f), this_sig) {
            let ts = self.sig(t).slot;
            let c = self.classes.of_scope[&fs_def];
            self.ctor_prologue(c, ts, None, fscope)?;
        }
        self.auto_inits(f, fscope, f.automatic)?;
        self.stmt(&f.body, fscope)
    }
}

/// A statement that a shared subroutine can hold: no timing, fork, `disable`, procedural
/// continuous assignment, deferred assertion, or system task that starts a process or stops.
fn unit_stmt(st: &Stmt) -> bool {
    match st {
        Stmt::Block { fork: Some(_), .. }
        | Stmt::Timed { .. }
        | Stmt::Wait { .. }
        | Stmt::WaitFork { .. }
        | Stmt::Disable { .. }
        | Stmt::Force { .. }
        | Stmt::Deferred { .. }
        | Stmt::Assign { timing: Some(_), .. } => false,
        Stmt::SysTask { name, .. } => !matches!(
            name.as_str(),
            "$strobe" | "$strobeb" | "$strobeo" | "$strobeh" | "$monitor" | "$monitorb" | "$monitoro" | "$monitorh" | "$fstrobe" | "$fstrobeb" | "$fstrobeo" | "$fstrobeh" | "$fmonitor" | "$fmonitorb" | "$fmonitoro" | "$fmonitorh" | "$stop" | "$assign" | "$deassign"
        ),
        Stmt::Block { stmts, .. } => stmts.iter().all(unit_stmt),
        Stmt::If { then_s, else_s, .. } => unit_stmt(then_s) && else_s.as_deref().is_none_or(unit_stmt),
        Stmt::Case { arms, default, .. } => arms.iter().all(|a| unit_stmt(&a.1)) && default.as_deref().is_none_or(unit_stmt),
        Stmt::For { init, step, body, .. } => unit_stmt(init) && unit_stmt(step) && unit_stmt(body),
        Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::Repeat { body, .. } | Stmt::Forever { body, .. } | Stmt::Foreach { body, .. } => unit_stmt(body),
        _ => true,
    }
}
