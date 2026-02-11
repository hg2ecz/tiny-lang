use crate::bytecode::{BytecodeProgram, ChunkId, Instr, LoadMode, LocalId, Mutability, StrId};
use crate::error::LangError;

#[derive(Debug, Clone)]
pub enum Value {
    Num(f64),
    Bool(bool),
    Str(String),
    Vec(Vec<Value>),
    Unit,
    Ref(RefTarget, Mutability),
}

impl Value {
    fn is_copy(&self) -> bool {
        matches!(self, Value::Num(_) | Value::Bool(_))
    }
}

#[derive(Debug, Clone)]
pub enum RefTarget {
    Local(LocalId),
    VecElem { local: LocalId, index: usize },
}

#[derive(Debug, Default, Clone, Copy)]
struct BorrowState {
    immut: usize,
    mut_: bool,
}

#[derive(Debug, Default, Clone)]
struct Slot {
    value: Option<Value>,
    borrows: BorrowState,
}

#[derive(Debug, Default)]
struct Env {
    locals: Vec<Slot>,
    scope_log: Vec<Vec<(RefTarget, Mutability)>>,
    temp_log: Vec<Vec<(RefTarget, Mutability)>>,
}

impl Env {
    fn with_local_count(n: usize) -> Self {
        let mut e = Self::default();
        e.locals = vec![Slot::default(); n];
        e.enter_scope();
        e
    }

    fn enter_scope(&mut self) {
        self.scope_log.push(Vec::new());
    }

    fn exit_scope(&mut self) -> Result<(), LangError> {
        let log = self
            .scope_log
            .pop()
            .ok_or_else(|| LangError::Runtime("Internal: scope underflow".into()))?;
        for (tgt, m) in log.into_iter().rev() {
            self.release_borrow(&tgt, m)?;
        }
        Ok(())
    }

    fn temp_enter(&mut self) {
        self.temp_log.push(Vec::new());
    }

    fn temp_exit_release(&mut self) -> Result<(), LangError> {
        let log = self
            .temp_log
            .pop()
            .ok_or_else(|| LangError::Runtime("Internal: temp scope underflow".into()))?;
        for (tgt, m) in log.into_iter().rev() {
            self.release_borrow(&tgt, m)?;
        }
        Ok(())
    }

    fn record_borrow(&mut self, tgt: RefTarget, m: Mutability) {
        if let Some(frame) = self.temp_log.last_mut() {
            frame.push((tgt, m));
        } else {
            self.scope_log.last_mut().unwrap().push((tgt, m));
        }
    }

    fn get_slot_mut(&mut self, local: LocalId) -> Result<&mut Slot, LangError> {
        self.locals
            .get_mut(local as usize)
            .ok_or_else(|| LangError::Runtime("Internal: bad local id".into()))
    }

    fn get_slot_ref(&self, local: LocalId) -> Result<&Slot, LangError> {
        self.locals
            .get(local as usize)
            .ok_or_else(|| LangError::Runtime("Internal: bad local id".into()))
    }

    fn define(&mut self, local: LocalId, v: Value) -> Result<(), LangError> {
        let slot = self.get_slot_mut(local)?;
        slot.value = Some(v);
        slot.borrows = BorrowState::default();
        Ok(())
    }

    fn store(&mut self, local: LocalId, v: Value) -> Result<(), LangError> {
        let slot = self.get_slot_mut(local)?;
        if slot.borrows.immut > 0 || slot.borrows.mut_ {
            return Err(LangError::Runtime("Cannot assign: it is borrowed".into()));
        }
        slot.value = Some(v);
        Ok(())
    }

    fn load(&mut self, local: LocalId, mode: LoadMode) -> Result<Value, LangError> {
        let slot = self.get_slot_mut(local)?;
        let Some(v) = slot.value.take() else {
            return Err(LangError::Runtime("Use after move".into()));
        };

        match mode {
            LoadMode::Move => {
                if v.is_copy() {
                    slot.value = Some(v.clone());
                    Ok(v)
                } else {
                    Ok(v)
                }
            }
            LoadMode::Peek => {
                slot.value = Some(v.clone());
                Ok(v)
            }
        }
    }

    fn acquire_borrow(&mut self, local: LocalId, m: Mutability) -> Result<(), LangError> {
        let slot = self.get_slot_mut(local)?;
        if slot.value.is_none() {
            return Err(LangError::Runtime("Cannot borrow: value was moved".into()));
        }
        match m {
            Mutability::Imm => {
                if slot.borrows.mut_ {
                    return Err(LangError::Runtime("Cannot immut-borrow: mut-borrowed".into()));
                }
                slot.borrows.immut += 1;
            }
            Mutability::Mut => {
                if slot.borrows.mut_ || slot.borrows.immut > 0 {
                    return Err(LangError::Runtime(
                        "Cannot mut-borrow: already borrowed".into(),
                    ));
                }
                slot.borrows.mut_ = true;
            }
        }
        self.record_borrow(RefTarget::Local(local), m);
        Ok(())
    }

    fn release_borrow(&mut self, tgt: &RefTarget, m: Mutability) -> Result<(), LangError> {
        let local = match tgt {
            RefTarget::Local(l) => *l,
            RefTarget::VecElem { local, .. } => *local,
        };
        let slot = self.get_slot_mut(local)?;
        match m {
            Mutability::Imm => {
                if slot.borrows.immut == 0 {
                    return Err(LangError::Runtime("Internal: immut borrow underflow".into()));
                }
                slot.borrows.immut -= 1;
            }
            Mutability::Mut => {
                if !slot.borrows.mut_ {
                    return Err(LangError::Runtime("Internal: mut borrow underflow".into()));
                }
                slot.borrows.mut_ = false;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct CallFrame {
    chunk: ChunkId,
    ip: usize,
    stack_base: usize,
    env: Env,
}

#[derive(Debug)]
struct LoopFrame {
    var: LocalId,
    elems: Vec<Value>,
    idx: usize,
    body_start: usize,
    end: usize,
}

pub struct Vm {
    bc: BytecodeProgram,
    env: Env,
    stack: Vec<Value>,
    callstack: Vec<CallFrame>,
    loops: Vec<LoopFrame>,
}

impl Vm {
    pub fn new(bc: BytecodeProgram) -> Self {
        let main_locals = bc.chunks[bc.main as usize].local_count;
        Self {
            bc,
            env: Env::with_local_count(main_locals),
            stack: Vec::new(),
            callstack: Vec::new(),
            loops: Vec::new(),
        }
    }

    pub fn run_main(&mut self) -> Result<(), LangError> {
        self.run_chunk(self.bc.main)
    }

    fn run_chunk(&mut self, start_chunk: ChunkId) -> Result<(), LangError> {
        let mut chunk = start_chunk;
        let mut ip: usize = 0;

        loop {
            let code = &self.bc.chunks[chunk as usize].code;

            if ip >= code.len() {
                // fell off end: implicit return ()
                let ret = Value::Unit;
                if let Some(frame) = self.callstack.pop() {
                    self.env = frame.env;
                    self.stack.truncate(frame.stack_base);
                    chunk = frame.chunk;
                    ip = frame.ip;
                    self.stack.push(ret);
                    continue;
                }
                return Ok(());
            }

            let ins = code[ip].clone();
            ip += 1;

            match ins {
                Instr::PushNum(n) => self.stack.push(Value::Num(n)),
                Instr::PushBool(b) => self.stack.push(Value::Bool(b)),
                Instr::PushStr(id) => self.stack.push(Value::Str(self.bc.strings[id as usize].clone())),
                Instr::PushUnit => self.stack.push(Value::Unit),

                Instr::VecMake(n) => {
                    let n = n as usize;
                    if self.stack.len() < n {
                        return Err(LangError::Runtime("Stack underflow in VecMake".into()));
                    }
                    let start = self.stack.len() - n;
                    let xs = self.stack.split_off(start);
                    self.stack.push(Value::Vec(xs));
                }

                Instr::LoadLocal { local, mode } => {
                    let v = self.env.load(local, mode)?;
                    self.stack.push(v);
                }
                Instr::DefineLocal(local) => {
                    let v = self.pop1()?;
                    self.env.define(local, v)?;
                }
                Instr::StoreLocal(local) => {
                    let v = self.pop1()?;
                    self.env.store(local, v)?;
                }

                Instr::Neg => {
                    let v = self.pop1()?;
                    match v {
                        Value::Num(x) => self.stack.push(Value::Num(-x)),
                        _ => return Err(LangError::Runtime("Unary '-' expects number".into())),
                    }
                }

                Instr::Add => self.bin_num(|a, b| a + b)?,
                Instr::Sub => self.bin_num(|a, b| a - b)?,
                Instr::Mul => self.bin_num(|a, b| a * b)?,
                Instr::Div => {
                    let (a, b) = self.pop2()?;
                    let (Value::Num(x), Value::Num(y)) = (a, b) else {
                        return Err(LangError::Runtime("Division expects numbers".into()));
                    };
                    if y == 0.0 {
                        return Err(LangError::Runtime("Division by zero".into()));
                    }
                    self.stack.push(Value::Num(x / y));
                }
                Instr::Mod => {
                    let (a, b) = self.pop2()?;
                    let (Value::Num(x), Value::Num(y)) = (a, b) else {
                        return Err(LangError::Runtime("Modulo expects numbers".into()));
                    };
                    if y == 0.0 {
                        return Err(LangError::Runtime("Modulo by zero".into()));
                    }
                    self.stack.push(Value::Num(x % y));
                }

                Instr::Eq => self.bin_cmp(|a, b| a == b)?,
                Instr::Ne => self.bin_cmp(|a, b| a != b)?,
                Instr::Lt => self.bin_ord(|a, b| a < b)?,
                Instr::Le => self.bin_ord(|a, b| a <= b)?,
                Instr::Gt => self.bin_ord(|a, b| a > b)?,
                Instr::Ge => self.bin_ord(|a, b| a >= b)?,

                Instr::BorrowLocal { local, mut_ } => {
                    let m = if mut_ { Mutability::Mut } else { Mutability::Imm };
                    self.env.acquire_borrow(local, m)?;
                    self.stack.push(Value::Ref(RefTarget::Local(local), m));
                }

                Instr::IndexLocal { base, base_mut } => {
                    // pops index
                    let idx = self.pop_index_usize()?;
                    let slot = self.env.get_slot_ref(base)?;
                    let Some(v) = &slot.value else {
                        return Err(LangError::Runtime("Use after move".into()));
                    };
                    match v {
                        Value::Vec(xs) => {
                            if idx >= xs.len() {
                                return Err(LangError::Runtime(format!(
                                    "Index out of bounds: {} (len {})",
                                    idx,
                                    xs.len()
                                )));
                            }
                            // indexing reads clone (like peek): no move out of vector
                            self.stack.push(xs[idx].clone());
                        }
                        _ => return Err(LangError::Runtime("Indexing expects a vector".into())),
                    }

                    // NOTE: base_mut is validated by compiler; runtime doesn't need it here.
                    let _ = base_mut;
                }

                Instr::BorrowIndexLocal {
                    base,
                    base_mut,
                    mut_,
                } => {
                    // pops index
                    let idx = self.pop_index_usize()?;
                    // borrow the base (as imm or mut, depending on base form)
                    self.env.acquire_borrow(base, base_mut)?;
                    let m = if mut_ { Mutability::Mut } else { Mutability::Imm };
                    // record element ref (so set() knows where to write)
                    self.stack
                        .push(Value::Ref(RefTarget::VecElem { local: base, index: idx }, m));
                }

                Instr::Call { callee, argc } => {
                    let argc = argc as usize;
                    if self.stack.len() < argc {
                        return Err(LangError::Runtime("Stack underflow in Call".into()));
                    }
                    let args = self.stack.split_off(self.stack.len() - argc);
                    let name = self.bc.strings[callee as usize].clone();

                    match name.as_str() {
                        "len" => {
                            if args.len() != 1 {
                                return Err(LangError::Runtime("len(x) expects 1 argument".into()));
                            }
                            let v = self.resolve_value_for_read(&args[0])?;
                            match v {
                                Value::Str(s) => self.stack.push(Value::Num(s.len() as f64)),
                                Value::Vec(xs) => self.stack.push(Value::Num(xs.len() as f64)),
                                _ => {
                                    return Err(LangError::Runtime(
                                        "len() expects string or vector".into(),
                                    ))
                                }
                            }
                        }
                        "push" => {
                            if args.len() != 2 {
                                return Err(LangError::Runtime(
                                    "push(&mut vec, value) expects 2 arguments".into(),
                                ));
                            }
                            let r = &args[0];
                            let v = args[1].clone();

                            let (local, m) = match r {
                                Value::Ref(RefTarget::Local(local), m) => (*local, *m),
                                _ => {
                                    return Err(LangError::Runtime(
                                        "push first argument must be &mut vec variable".into(),
                                    ))
                                }
                            };
                            if m != Mutability::Mut {
                                return Err(LangError::Runtime("push requires &mut vec".into()));
                            }

                            let slot = self.env.get_slot_mut(local)?;
                            let cur = slot
                                .value
                                .as_mut()
                                .ok_or_else(|| LangError::Runtime("Use after move".into()))?;
                            match cur {
                                Value::Vec(xs) => {
                                    xs.push(v);
                                    self.stack.push(Value::Unit);
                                }
                                _ => return Err(LangError::Runtime("push target must be a vector".into())),
                            }
                        }
                        "set" => {
                            if args.len() != 2 {
                                return Err(LangError::Runtime(
                                    "set(&mut elem_ref, value) expects 2 arguments".into(),
                                ));
                            }
                            let r = &args[0];
                            let v = args[1].clone();

                            let (local, idx, m) = match r {
                                Value::Ref(RefTarget::VecElem { local, index }, m) => {
                                    (*local, *index, *m)
                                }
                                _ => {
                                    return Err(LangError::Runtime(
                                        "set first argument must be &mut (xs[i])".into(),
                                    ))
                                }
                            };
                            if m != Mutability::Mut {
                                return Err(LangError::Runtime(
                                    "set requires a mutable element reference".into(),
                                ));
                            }

                            let slot = self.env.get_slot_mut(local)?;
                            let cur = slot
                                .value
                                .as_mut()
                                .ok_or_else(|| LangError::Runtime("Use after move".into()))?;
                            match cur {
                                Value::Vec(xs) => {
                                    if idx >= xs.len() {
                                        return Err(LangError::Runtime(format!(
                                            "Index out of bounds: {} (len {})",
                                            idx,
                                            xs.len()
                                        )));
                                    }
                                    xs[idx] = v;
                                    self.stack.push(Value::Unit);
                                }
                                _ => {
                                    return Err(LangError::Runtime(
                                        "set target base must be a vector".into(),
                                    ))
                                }
                            }
                        }
                        "range" => {
                            if !(args.len() == 1 || args.len() == 2) {
                                return Err(LangError::Runtime(
                                    "range(end) or range(start, end) expects 1 or 2 arguments"
                                        .into(),
                                ));
                            }
                            let (start, end) = if args.len() == 1 {
                                (Value::Num(0.0), args[0].clone())
                            } else {
                                (args[0].clone(), args[1].clone())
                            };
                            let start = self.resolve_value_for_read(&start)?;
                            let end = self.resolve_value_for_read(&end)?;

                            let (Value::Num(s), Value::Num(e)) = (start, end) else {
                                return Err(LangError::Runtime("range() expects number arguments".into()));
                            };

                            let s = self.to_usize_int(s, "range start")? as i64;
                            let e = self.to_usize_int(e, "range end")? as i64;

                            let mut out = Vec::new();
                            let mut i = s;
                            while i < e {
                                out.push(Value::Num(i as f64));
                                i += 1;
                            }
                            self.stack.push(Value::Vec(out));
                        }
                        _ => {
                            // user function
                            let Some(&fn_chunk) = self.bc.fns.get(&callee) else {
                                return Err(LangError::Runtime(format!("Unknown function: {}", name)));
                            };
                            let params = self.bc.chunks[fn_chunk as usize].params.clone();
                            if params.len() != args.len() {
                                return Err(LangError::Runtime(format!(
                                    "Function {} expects {} args, got {}",
                                    name,
                                    params.len(),
                                    args.len()
                                )));
                            }

                            // save current frame
                            let saved = CallFrame {
                                chunk,
                                ip,
                                stack_base: self.stack.len(),
                                env: std::mem::replace(
                                    &mut self.env,
                                    Env::with_local_count(self.bc.chunks[fn_chunk as usize].local_count),
                                ),
                            };
                            self.callstack.push(saved);

                            // bind params
                            for (p, a) in params.into_iter().zip(args.into_iter()) {
                                self.env.define(p, a)?;
                            }

                            chunk = fn_chunk;
                            ip = 0;
                        }
                    }
                }

                Instr::Return => {
                    let ret = self.pop1()?;
                    // unwind scopes to base
                    while self.env.scope_log.len() > 1 {
                        self.env.exit_scope()?;
                    }
                    if let Some(frame) = self.callstack.pop() {
                        self.env = frame.env;
                        self.stack.truncate(frame.stack_base);
                        chunk = frame.chunk;
                        ip = frame.ip;
                        self.stack.push(ret);
                    } else {
                        // return from main
                        return Ok(());
                    }
                }

                Instr::Print => {
                    let v = self.pop1()?;
                    let s = self.value_to_string(&v)?;
                    println!("{}", s);
                }

                Instr::Pop => {
                    let _ = self.pop1()?;
                }

                Instr::EnterScope => self.env.enter_scope(),
                Instr::ExitScope => self.env.exit_scope()?,
                Instr::TempEnter => self.env.temp_enter(),
                Instr::TempExit => self.env.temp_exit_release()?,

                Instr::Jump(to) => ip = to,
                Instr::JumpIfFalse(to) => {
                    let v = self.pop1()?;
                    let b = self.value_truthy(&v)?;
                    if !b {
                        ip = to;
                    }
                }

                Instr::ForInit { var, body_start, end } => {
                    let it = self.pop1()?;
                    let it = self.resolve_value_for_read(&it)?;
                    let Value::Vec(elems) = it else {
                        return Err(LangError::Runtime("for-in expects a vector".into()));
                    };

                    if elems.is_empty() {
                        ip = end;
                    } else {
                        // define first iteration var
                        let first = elems[0].clone();
                        self.env.define(var, first)?;
                        self.loops.push(LoopFrame { var, elems, idx: 0, body_start, end });
                        ip = body_start;
                    }
                }

                Instr::ForNext { body_start, end } => {
                    let Some(top) = self.loops.last_mut() else {
                        return Err(LangError::Runtime("Internal: ForNext without loop".into()));
                    };
                    top.idx += 1;
                    if top.idx >= top.elems.len() {
                        self.loops.pop();
                        ip = end;
                    } else {
                        let v = top.elems[top.idx].clone();
                        self.env.define(top.var, v)?;
                        ip = body_start;
                    }
                }
            }
        }
    }

    // --- helpers ---

    fn pop1(&mut self) -> Result<Value, LangError> {
        self.stack.pop().ok_or_else(|| LangError::Runtime("Stack underflow".into()))
    }

    fn pop2(&mut self) -> Result<(Value, Value), LangError> {
        let b = self.pop1()?;
        let a = self.pop1()?;
        Ok((a, b))
    }

    fn bin_num<F: FnOnce(f64, f64) -> f64>(&mut self, f: F) -> Result<(), LangError> {
        let (a, b) = self.pop2()?;
        let (Value::Num(x), Value::Num(y)) = (a, b) else {
            return Err(LangError::Runtime("Arithmetic expects numbers".into()));
        };
        self.stack.push(Value::Num(f(x, y)));
        Ok(())
    }

    fn bin_cmp<F: FnOnce(Value, Value) -> bool>(&mut self, f: F) -> Result<(), LangError> {
        let (a, b) = self.pop2()?;
        self.stack.push(Value::Bool(f(a, b)));
        Ok(())
    }

    fn bin_ord<F: FnOnce(f64, f64) -> bool>(&mut self, f: F) -> Result<(), LangError> {
        let (a, b) = self.pop2()?;
        let (Value::Num(x), Value::Num(y)) = (a, b) else {
            return Err(LangError::Runtime("Ordering expects numbers".into()));
        };
        self.stack.push(Value::Bool(f(x, y)));
        Ok(())
    }

    fn value_truthy(&self, v: &Value) -> Result<bool, LangError> {
        match v {
            Value::Bool(b) => Ok(*b),
            _ => Err(LangError::Runtime("Condition must be bool".into())),
        }
    }

    fn resolve_value_for_read(&self, v: &Value) -> Result<Value, LangError> {
        match v {
            Value::Ref(RefTarget::Local(local), _) => {
                let slot = self.env.get_slot_ref(*local)?;
                slot.value.clone().ok_or_else(|| LangError::Runtime("Use after move".into()))
            }
            Value::Ref(RefTarget::VecElem { local, index }, _) => {
                let slot = self.env.get_slot_ref(*local)?;
                let Some(base) = &slot.value else {
                    return Err(LangError::Runtime("Use after move".into()));
                };
                match base {
                    Value::Vec(xs) => xs.get(*index).cloned().ok_or_else(|| {
                        LangError::Runtime(format!("Index out of bounds: {}", index))
                    }),
                    _ => Err(LangError::Runtime("Internal: VecElem base not vec".into())),
                }
            }
            other => Ok(other.clone()),
        }
    }

    fn pop_index_usize(&mut self) -> Result<usize, LangError> {
        let v = self.pop1()?;
        let v = self.resolve_value_for_read(&v)?;
        match v {
            Value::Num(x) => Ok(self.to_usize_int(x, "index")?),
            _ => Err(LangError::Runtime("Index must be a number".into())),
        }
    }

    fn to_usize_int(&self, x: f64, what: &str) -> Result<usize, LangError> {
        if !x.is_finite() {
            return Err(LangError::Runtime(format!("{} must be finite", what)));
        }
        if x < 0.0 {
            return Err(LangError::Runtime(format!("{} must be >= 0", what)));
        }
        if x.fract() != 0.0 {
            return Err(LangError::Runtime(format!("{} must be an integer number", what)));
        }
        let ux = x as u64;
        usize::try_from(ux).map_err(|_| LangError::Runtime(format!("{} too large", what)))
    }

    fn value_to_string(&self, v: &Value) -> Result<String, LangError> {
        match v {
            Value::Num(n) => Ok({
                // rövid, de stabil: 3.0 -> "3", 3.25 -> "3.25"
                if n.fract() == 0.0 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }),
            Value::Bool(b) => Ok(b.to_string()),
            Value::Str(s) => Ok(s.clone()),
            Value::Vec(xs) => {
                let mut out = String::from("[");
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&self.value_to_string(x)?);
                }
                out.push(']');
                Ok(out)
            }
            Value::Unit => Ok("()".into()),
            Value::Ref(_, _) => {
                let r = self.resolve_value_for_read(v)?;
                self.value_to_string(&r)
            }
        }
    }
}
