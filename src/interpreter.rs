use std::collections::HashMap;

use crate::ast::{Expr, FnDef, Item, Program, Stmt};
use crate::error::LangError;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Str(String),
    Vec(Vec<Value>),
    Unit,

    Ref(RefTarget, Mutability),
}

impl Value {
    fn is_copy(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Bool(_))
    }
}

#[derive(Debug, Clone)]
pub enum RefTarget {
    Var(String),
    VecElem { var: String, index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mutability {
    Imm,
    Mut,
}

#[derive(Debug, Default)]
struct BorrowState {
    immut: usize,
    mut_: bool,
}

#[derive(Debug, Default)]
struct Slot {
    value: Option<Value>,
    borrows: BorrowState,
}

#[derive(Debug, Default)]
struct Env {
    scopes: Vec<HashMap<String, Slot>>,
    borrow_log: Vec<Vec<(RefTarget, Mutability)>>, // released on pop_scope

    // NEW: temporary borrows, released at end of statement (print/exprstmt)
    temp_borrow_log: Vec<Vec<(RefTarget, Mutability)>>,
}

impl Env {
    fn new() -> Self {
        let mut e = Self::default();
        e.push_scope();
        e
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.borrow_log.push(Vec::new());
    }

    fn pop_scope(&mut self) -> Result<(), LangError> {
        let log = self.borrow_log.pop().unwrap();
        for (tgt, mutability) in log.into_iter().rev() {
            self.release_borrow(&tgt, mutability)?;
        }
        self.scopes.pop();
        Ok(())
    }

    // ---- temporary borrow frames ----

    fn push_temp_frame(&mut self) {
        self.temp_borrow_log.push(Vec::new());
    }

    fn pop_temp_frame_release(&mut self) -> Result<(), LangError> {
        let log = self
            .temp_borrow_log
            .pop()
            .expect("temp borrow frame underflow");
        for (tgt, mutability) in log.into_iter().rev() {
            self.release_borrow(&tgt, mutability)?;
        }
        Ok(())
    }

    fn record_borrow(&mut self, target: RefTarget, mutability: Mutability) {
        if let Some(frame) = self.temp_borrow_log.last_mut() {
            frame.push((target, mutability));
        } else {
            self.borrow_log.last_mut().unwrap().push((target, mutability));
        }
    }

    fn replace_last_record_with(&mut self, target: RefTarget, mutability: Mutability) {
        if let Some(frame) = self.temp_borrow_log.last_mut() {
            frame.pop();
            frame.push((target, mutability));
        } else {
            self.borrow_log.last_mut().unwrap().pop();
            self.borrow_log.last_mut().unwrap().push((target, mutability));
        }
    }

    // ---- vars ----

    fn define(&mut self, name: String, v: Value) {
        self.scopes.last_mut().unwrap().insert(
            name,
            Slot {
                value: Some(v),
                borrows: BorrowState::default(),
            },
        );
    }

    fn lookup_slot_mut(&mut self, name: &str) -> Option<&mut Slot> {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                return scope.get_mut(name);
            }
        }
        None
    }

    fn lookup_slot_ref(&self, name: &str) -> Option<&Slot> {
        for scope in self.scopes.iter().rev() {
            if scope.contains_key(name) {
                return scope.get(name);
            }
        }
        None
    }

    fn assign(&mut self, name: &str, v: Value) -> Result<(), LangError> {
        let slot = self
            .lookup_slot_mut(name)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;

        if slot.borrows.immut > 0 || slot.borrows.mut_ {
            return Err(LangError::Runtime(format!(
                "Cannot assign '{}': it is borrowed",
                name
            )));
        }

        slot.value = Some(v);
        Ok(())
    }

    // ---- borrowing ----

    fn acquire_borrow(&mut self, var: &str, mutability: Mutability) -> Result<(), LangError> {
        let slot = self
            .lookup_slot_mut(var)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", var)))?;

        if slot.value.is_none() {
            return Err(LangError::Runtime(format!(
                "Cannot borrow '{}': value was moved",
                var
            )));
        }

        match mutability {
            Mutability::Imm => {
                if slot.borrows.mut_ {
                    return Err(LangError::Runtime(format!(
                        "Cannot immut-borrow '{}': it is mut-borrowed",
                        var
                    )));
                }
                slot.borrows.immut += 1;
            }
            Mutability::Mut => {
                if slot.borrows.mut_ || slot.borrows.immut > 0 {
                    return Err(LangError::Runtime(format!(
                        "Cannot mut-borrow '{}': it is already borrowed",
                        var
                    )));
                }
                slot.borrows.mut_ = true;
            }
        }

        // record in temp frame if active, otherwise scope log
        self.record_borrow(RefTarget::Var(var.to_string()), mutability);
        Ok(())
    }

    fn release_borrow(
        &mut self,
        target: &RefTarget,
        mutability: Mutability,
    ) -> Result<(), LangError> {
        let var_name = match target {
            RefTarget::Var(v) => v.as_str(),
            RefTarget::VecElem { var, .. } => var.as_str(),
        };

        let slot = self.lookup_slot_mut(var_name).ok_or_else(|| {
            LangError::Runtime(format!(
                "Internal: releasing borrow of unknown var '{}'",
                var_name
            ))
        })?;

        match mutability {
            Mutability::Imm => {
                if slot.borrows.immut == 0 {
                    return Err(LangError::Runtime(format!(
                        "Internal: immut borrow underflow for '{}'",
                        var_name
                    )));
                }
                slot.borrows.immut -= 1;
            }
            Mutability::Mut => {
                if !slot.borrows.mut_ {
                    return Err(LangError::Runtime(format!(
                        "Internal: mut borrow underflow for '{}'",
                        var_name
                    )));
                }
                slot.borrows.mut_ = false;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
enum Control {
    None,
    Return(Value),
}

#[derive(Debug, Default)]
pub struct Interpreter {
    env: Env,
    fns: HashMap<String, FnDef>,
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            env: Env::new(),
            fns: HashMap::new(),
        }
    }

    pub fn run(&mut self, p: Program) -> Result<(), LangError> {
        // collect functions
        for it in &p.items {
            if let Item::Fn(f) = it {
                if self.fns.contains_key(&f.name) {
                    return Err(LangError::Runtime(format!(
                        "Duplicate function: {}",
                        f.name
                    )));
                }
                self.fns.insert(f.name.clone(), f.clone());
            }
        }

        // execute top-level statements
        for it in p.items {
            if let Item::Stmt(s) = it {
                let c = self.exec_stmt(s)?;
                if let Control::Return(_) = c {
                    return Err(LangError::Runtime(
                        "return is only allowed inside functions".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn exec_block(&mut self, body: &[Stmt]) -> Result<Control, LangError> {
        self.env.push_scope();
        for s in body.iter().cloned() {
            let c = self.exec_stmt(s)?;
            if let Control::Return(_) = c {
                self.env.pop_scope()?;
                return Ok(c);
            }
        }
        self.env.pop_scope()?;
        Ok(Control::None)
    }

    fn exec_stmt(&mut self, s: Stmt) -> Result<Control, LangError> {
        match s {
            Stmt::Let { name, expr } => {
                // DO NOT use temp frame here: if expr is a reference, it must stay alive.
                let v = self.eval_expr(expr)?;
                self.env.define(name, v);
                Ok(Control::None)
            }
            Stmt::Assign { name, expr } => {
                // DO NOT use temp frame here for the same reason.
                let v = self.eval_expr(expr)?;
                self.env.assign(&name, v)?;
                Ok(Control::None)
            }
            Stmt::Print { expr } => {
                // Temporary borrows created inside this statement are released at statement end.
                self.env.push_temp_frame();
                let res: Result<(), LangError> = (|| {
                    let v = self.eval_expr(expr)?;
                    println!("{}", self.format_value(&v)?);
                    Ok(())
                })();
                let pop_res = self.env.pop_temp_frame_release();
                res?;
                pop_res?;
                Ok(Control::None)
            }
            Stmt::ExprStmt { expr } => {
                // Same statement-level temp-borrow behavior.
                self.env.push_temp_frame();
                let res: Result<(), LangError> = (|| {
                    let _ = self.eval_expr(expr)?;
                    Ok(())
                })();
                let pop_res = self.env.pop_temp_frame_release();
                res?;
                pop_res?;
                Ok(Control::None)
            }
            Stmt::Return(e) => {
                let v = self.eval_expr(e)?;
                Ok(Control::Return(v))
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                let cond_v = self.eval_expr(cond)?;
                let cond_r = self.resolve_value_for_read(&cond_v)?;
                let b = match cond_r {
                    Value::Bool(x) => x,
                    _ => {
                        return Err(LangError::Runtime(
                            "if condition must be Bool (true/false)".into(),
                        ))
                    }
                };

                if b {
                    self.exec_block(&then_body)
                } else {
                    self.exec_block(&else_body)
                }
            }
            Stmt::For { var, iter, body } => {
                // iter must evaluate to Vec (owned, moved)
                let v = self.eval_expr(iter)?;
                let elems = match v {
                    Value::Vec(xs) => xs,
                    _ => {
                        return Err(LangError::Runtime(
                            "for expects a vector value (moved)".into(),
                        ))
                    }
                };

                for elem in elems {
                    self.env.push_scope();
                    self.env.define(var.clone(), elem);

                    for st in body.clone() {
                        let c = self.exec_stmt(st)?;
                        if let Control::Return(_) = c {
                            self.env.pop_scope()?;
                            return Ok(c);
                        }
                    }

                    self.env.pop_scope()?;
                }
                Ok(Control::None)
            }
        }
    }

    fn eval_expr(&mut self, e: Expr) -> Result<Value, LangError> {
        match e {
            Expr::Int(n) => Ok(Value::Int(n)),
            Expr::Bool(b) => Ok(Value::Bool(b)),
            Expr::Str(s) => Ok(Value::Str(s)),

            Expr::VecLit(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(self.eval_expr(it)?);
                }
                Ok(Value::Vec(out))
            }

            Expr::Ident(name) => self.read_var_move(&name),

            Expr::Neg { expr } => {
                let tmp = self.eval_expr(*expr)?;
                let v = self.resolve_value_for_read(&tmp)?;
                match v {
                    Value::Int(n) => Ok(Value::Int(-n)),
                    _ => Err(LangError::Runtime("Unary '-' expects Int".into())),
                }
            }

            Expr::Add { left, right } => self.eval_int_binop(*left, *right, |x, y| x + y, "+"),
            Expr::Sub { left, right } => self.eval_int_binop(*left, *right, |x, y| x - y, "-"),
            Expr::Mul { left, right } => self.eval_int_binop(*left, *right, |x, y| x * y, "*"),

            Expr::Div { left, right } => {
                let a_tmp = self.eval_expr(*left)?;
                let b_tmp = self.eval_expr(*right)?;
                let a = self.resolve_value_for_read(&a_tmp)?;
                let b = self.resolve_value_for_read(&b_tmp)?;
                match (a, b) {
                    (Value::Int(_), Value::Int(0)) => {
                        Err(LangError::Runtime("Division by zero".into()))
                    }
                    (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x / y)),
                    _ => Err(LangError::Runtime("Only Int / Int is supported".into())),
                }
            }

            Expr::Mod { left, right } => {
                let a_tmp = self.eval_expr(*left)?;
                let b_tmp = self.eval_expr(*right)?;
                let a = self.resolve_value_for_read(&a_tmp)?;
                let b = self.resolve_value_for_read(&b_tmp)?;
                match (a, b) {
                    (Value::Int(_), Value::Int(0)) => {
                        Err(LangError::Runtime("Modulo by zero".into()))
                    }
                    (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x % y)),
                    _ => Err(LangError::Runtime("Only Int % Int is supported".into())),
                }
            }

            Expr::Eq { left, right } => {
                let a_tmp = self.eval_expr(*left)?;
                let b_tmp = self.eval_expr(*right)?;
                let a = self.resolve_value_for_read(&a_tmp)?;
                let b = self.resolve_value_for_read(&b_tmp)?;
                Ok(Value::Bool(self.value_eq(&a, &b)))
            }
            Expr::Ne { left, right } => {
                let a_tmp = self.eval_expr(*left)?;
                let b_tmp = self.eval_expr(*right)?;
                let a = self.resolve_value_for_read(&a_tmp)?;
                let b = self.resolve_value_for_read(&b_tmp)?;
                Ok(Value::Bool(!self.value_eq(&a, &b)))
            }

            Expr::Lt { left, right } => self.eval_int_cmp(*left, *right, |x, y| x < y),
            Expr::Le { left, right } => self.eval_int_cmp(*left, *right, |x, y| x <= y),
            Expr::Gt { left, right } => self.eval_int_cmp(*left, *right, |x, y| x > y),
            Expr::Ge { left, right } => self.eval_int_cmp(*left, *right, |x, y| x >= y),

            Expr::Call { callee, args } => self.call(&callee, args),

            Expr::Index { base, index } => self.eval_index(*base, *index),

            Expr::Borrow { mut_, expr } => self.eval_borrow(mut_, *expr),
        }
    }

    fn value_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Str(x), Value::Str(y)) => x == y,
            _ => false,
        }
    }

    fn eval_int_binop<F: FnOnce(i64, i64) -> i64>(
        &mut self,
        left: Expr,
        right: Expr,
        f: F,
        op: &'static str,
    ) -> Result<Value, LangError> {
        let a_tmp = self.eval_expr(left)?;
        let b_tmp = self.eval_expr(right)?;
        let a = self.resolve_value_for_read(&a_tmp)?;
        let b = self.resolve_value_for_read(&b_tmp)?;
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(f(x, y))),
            _ => Err(LangError::Runtime(format!("Only Int {} Int is supported", op))),
        }
    }

    fn eval_int_cmp<F: FnOnce(i64, i64) -> bool>(
        &mut self,
        left: Expr,
        right: Expr,
        f: F,
    ) -> Result<Value, LangError> {
        let a_tmp = self.eval_expr(left)?;
        let b_tmp = self.eval_expr(right)?;
        let a = self.resolve_value_for_read(&a_tmp)?;
        let b = self.resolve_value_for_read(&b_tmp)?;
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(Value::Bool(f(x, y))),
            _ => Err(LangError::Runtime("Comparison expects Int values".into())),
        }
    }

    fn read_var_move(&mut self, name: &str) -> Result<Value, LangError> {
        let slot = self
            .env
            .lookup_slot_mut(name)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;

        if slot.borrows.immut > 0 || slot.borrows.mut_ {
            return Err(LangError::Runtime(format!(
                "Cannot move '{}': it is borrowed",
                name
            )));
        }

        let vref = slot
            .value
            .as_ref()
            .ok_or_else(|| LangError::Runtime(format!("Use after move: {}", name)))?;

        if vref.is_copy() {
            Ok(vref.clone())
        } else {
            Ok(slot.value.take().unwrap())
        }
    }

    fn peek_var(&self, name: &str) -> Result<Value, LangError> {
        let slot = self
            .env
            .lookup_slot_ref(name)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;
        let v = slot
            .value
            .as_ref()
            .ok_or_else(|| LangError::Runtime(format!("Use after move: {}", name)))?;
        Ok(v.clone())
    }

    fn eval_borrow(&mut self, mut_: bool, inner: Expr) -> Result<Value, LangError> {
        let want = if mut_ { Mutability::Mut } else { Mutability::Imm };
        let target = self.lvalue_target(inner, want)?;

        let owning_var = match &target {
            RefTarget::Var(v) => v.clone(),
            RefTarget::VecElem { var, .. } => var.clone(),
        };

        self.env.acquire_borrow(&owning_var, want)?;

        if let RefTarget::VecElem { .. } = &target {
            // replace Var(...) record with VecElem(...) in active log (temp or scope)
            self.env.replace_last_record_with(target.clone(), want);
        }

        Ok(Value::Ref(target, want))
    }

    fn lvalue_target(&mut self, e: Expr, want: Mutability) -> Result<RefTarget, LangError> {
        match e {
            Expr::Ident(name) => Ok(RefTarget::Var(name)),

            Expr::Index { base, index } => {
                let (var, base_mut) = self.base_as_var_or_ref(*base)?;
                if want == Mutability::Mut && base_mut != Mutability::Mut {
                    return Err(LangError::Runtime(
                        "Need &mut base to take mutable element reference".into(),
                    ));
                }

                let idx_v = self.eval_expr(*index)?;
                let idx = match idx_v {
                    Value::Int(n) if n >= 0 => n as usize,
                    _ => {
                        return Err(LangError::Runtime(
                            "Index must be a non-negative Int".into(),
                        ))
                    }
                };

                let vec_val = self.peek_var(&var)?;
                match vec_val {
                    Value::Vec(xs) => {
                        if idx >= xs.len() {
                            return Err(LangError::Runtime(format!(
                                "Index out of bounds: {} (len {})",
                                idx,
                                xs.len()
                            )));
                        }
                    }
                    _ => {
                        return Err(LangError::Runtime(
                            "Index base must be a vector variable".into(),
                        ))
                    }
                }

                Ok(RefTarget::VecElem { var, index: idx })
            }

            _ => Err(LangError::Runtime(
                "Borrow target must be a variable or index (xs[i])".into(),
            )),
        }
    }

    fn base_as_var_or_ref(&mut self, base: Expr) -> Result<(String, Mutability), LangError> {
        match base {
            Expr::Ident(name) => Ok((name, Mutability::Imm)),
            Expr::Borrow { mut_, expr } => {
                let v = self.eval_borrow(mut_, *expr)?;
                match v {
                    Value::Ref(RefTarget::Var(var), m) => Ok((var, m)),
                    _ => Err(LangError::Runtime(
                        "Index base borrow must be &var or &mut var".into(),
                    )),
                }
            }
            _ => Err(LangError::Runtime(
                "Index base must be a variable (xs) or a borrow (&xs / &mut xs)".into(),
            )),
        }
    }

    fn eval_index(&mut self, base: Expr, index: Expr) -> Result<Value, LangError> {
        let (var, base_mut) = self.base_as_var_or_ref(base)?;

        let idx_v = self.eval_expr(index)?;
        let idx = match idx_v {
            Value::Int(n) if n >= 0 => n as usize,
            _ => {
                return Err(LangError::Runtime(
                    "Index must be a non-negative Int".into(),
                ))
            }
        };

        let vec_val = self.peek_var(&var)?;
        match vec_val {
            Value::Vec(xs) => {
                if idx >= xs.len() {
                    return Err(LangError::Runtime(format!(
                        "Index out of bounds: {} (len {})",
                        idx,
                        xs.len()
                    )));
                }
            }
            _ => {
                return Err(LangError::Runtime(
                    "Index base must be a vector variable".into(),
                ))
            }
        }

        if base_mut == Mutability::Mut {
            Ok(Value::Ref(
                RefTarget::VecElem { var, index: idx },
                Mutability::Mut,
            ))
        } else {
            // immut element ref borrows the base vector immutably
            self.env.acquire_borrow(&var, Mutability::Imm)?;
            self.env.replace_last_record_with(
                RefTarget::VecElem {
                    var: var.clone(),
                    index: idx,
                },
                Mutability::Imm,
            );

            Ok(Value::Ref(
                RefTarget::VecElem { var, index: idx },
                Mutability::Imm,
            ))
        }
    }

    fn call(&mut self, callee: &str, args: Vec<Expr>) -> Result<Value, LangError> {
        match callee {
            "len" => self.builtin_len(args),
            "push" => self.builtin_push(args),
            "set" => self.builtin_set(args),
            "range" => self.builtin_range(args),
            _ => self.call_user_fn(callee, args),
        }
    }

    fn builtin_len(&mut self, args: Vec<Expr>) -> Result<Value, LangError> {
        if args.len() != 1 {
            return Err(LangError::Runtime("len(x) expects 1 argument".into()));
        }
        let tmp = self.eval_expr(args[0].clone())?;
        let v = self.resolve_value_for_read(&tmp)?;
        match v {
            Value::Str(s) => Ok(Value::Int(s.len() as i64)),
            Value::Vec(xs) => Ok(Value::Int(xs.len() as i64)),
            _ => Err(LangError::Runtime("len() expects string or vector".into())),
        }
    }

    fn builtin_push(&mut self, args: Vec<Expr>) -> Result<Value, LangError> {
        if args.len() != 2 {
            return Err(LangError::Runtime(
                "push(&mut vec, value) expects 2 arguments".into(),
            ));
        }
        let r = self.eval_expr(args[0].clone())?;
        let v = self.eval_expr(args[1].clone())?;

        let (var, m) = match r {
            Value::Ref(RefTarget::Var(var), m) => (var, m),
            _ => {
                return Err(LangError::Runtime(
                    "push first argument must be &mut vec variable".into(),
                ))
            }
        };
        if m != Mutability::Mut {
            return Err(LangError::Runtime("push requires &mut vec".into()));
        }

        let slot = self
            .env
            .lookup_slot_mut(&var)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", var)))?;
        let cur = slot
            .value
            .as_mut()
            .ok_or_else(|| LangError::Runtime(format!("Use after move: {}", var)))?;
        match cur {
            Value::Vec(xs) => {
                xs.push(v);
                Ok(Value::Unit)
            }
            _ => Err(LangError::Runtime("push target must be a vector".into())),
        }
    }

    fn builtin_set(&mut self, args: Vec<Expr>) -> Result<Value, LangError> {
        if args.len() != 2 {
            return Err(LangError::Runtime(
                "set(&mut elem_ref, value) expects 2 arguments".into(),
            ));
        }
        let r = self.eval_expr(args[0].clone())?;
        let v = self.eval_expr(args[1].clone())?;

        let (var, idx, m) = match r {
            Value::Ref(RefTarget::VecElem { var, index }, m) => (var, index, m),
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

        let slot = self
            .env
            .lookup_slot_mut(&var)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", var)))?;
        let cur = slot
            .value
            .as_mut()
            .ok_or_else(|| LangError::Runtime(format!("Use after move: {}", var)))?;
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
                Ok(Value::Unit)
            }
            _ => Err(LangError::Runtime("set target base must be a vector".into())),
        }
    }

    fn builtin_range(&mut self, args: Vec<Expr>) -> Result<Value, LangError> {
        if !(args.len() == 1 || args.len() == 2) {
            return Err(LangError::Runtime(
                "range(end) or range(start, end) expects 1 or 2 arguments".into(),
            ));
        }

        let (start_expr, end_expr) = if args.len() == 1 {
            (Expr::Int(0), args[0].clone())
        } else {
            (args[0].clone(), args[1].clone())
        };

        let start_tmp = self.eval_expr(start_expr)?;
        let end_tmp = self.eval_expr(end_expr)?;
        let start_v = self.resolve_value_for_read(&start_tmp)?;
        let end_v = self.resolve_value_for_read(&end_tmp)?;

        let (Value::Int(start), Value::Int(end)) = (start_v, end_v) else {
            return Err(LangError::Runtime("range() expects Int arguments".into()));
        };

        let mut out = Vec::new();
        let mut i = start;
        while i < end {
            out.push(Value::Int(i));
            i += 1;
        }
        Ok(Value::Vec(out))
    }

    fn call_user_fn(&mut self, callee: &str, args: Vec<Expr>) -> Result<Value, LangError> {
        let f = self
            .fns
            .get(callee)
            .cloned()
            .ok_or_else(|| LangError::Runtime(format!("Unknown function: {}", callee)))?;

        if f.params.len() != args.len() {
            return Err(LangError::Runtime(format!(
                "Function {} expects {} args, got {}",
                callee,
                f.params.len(),
                args.len()
            )));
        }

        let mut arg_vals = Vec::with_capacity(args.len());
        for a in args {
            arg_vals.push(self.eval_expr(a)?);
        }

        self.env.push_scope();
        for (p, v) in f.params.iter().zip(arg_vals.into_iter()) {
            self.env.define(p.clone(), v);
        }

        let mut ret = Value::Unit;
        for st in f.body.clone() {
            match self.exec_stmt(st)? {
                Control::None => {}
                Control::Return(v) => {
                    ret = v;
                    break;
                }
            }
        }

        self.env.pop_scope()?;
        Ok(ret)
    }

    fn resolve_value_for_read(&self, v: &Value) -> Result<Value, LangError> {
        match v {
            Value::Ref(tgt, _) => self.deref_read(tgt),
            other => Ok(other.clone()),
        }
    }

    fn deref_read(&self, tgt: &RefTarget) -> Result<Value, LangError> {
        match tgt {
            RefTarget::Var(var) => self.peek_var(var),
            RefTarget::VecElem { var, index } => {
                let base = self.peek_var(var)?;
                match base {
                    Value::Vec(xs) => xs.get(*index).cloned().ok_or_else(|| {
                        LangError::Runtime(format!(
                            "Index out of bounds: {} (len {})",
                            index,
                            xs.len()
                        ))
                    }),
                    _ => Err(LangError::Runtime(
                        "Element reference base is not a vector".into(),
                    )),
                }
            }
        }
    }

    fn format_value(&self, v: &Value) -> Result<String, LangError> {
        let resolved = self.resolve_value_for_read(v)?;
        Ok(match resolved {
            Value::Int(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Str(s) => format!("{:?}", s),
            Value::Vec(xs) => {
                let inner = xs
                    .iter()
                    .map(|x| self.format_value(x))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ");
                format!("[{}]", inner)
            }
            Value::Unit => "()".into(),
            Value::Ref(_, _) => unreachable!("resolved above"),
        })
    }
}
