use crate::ast::{Expr, FnDef, Item, Program, Stmt};
use crate::env::Env;
use crate::error::LangError;
use crate::value::{Mutability, RefTarget, Value};

use std::collections::HashMap;

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

        // run top-level statements
        for it in p.items {
            if let Item::Stmt(s) = it {
                self.exec_stmt(s)?;
            }
        }
        Ok(())
    }

    fn exec_stmt(&mut self, s: Stmt) -> Result<Option<Value>, LangError> {
        match s {
            Stmt::Let { name, expr } => {
                let v = self.eval_expr(expr)?;
                self.env.define(name, v);
                Ok(None)
            }
            Stmt::Assign { name, expr } => {
                let v = self.eval_expr(expr)?;
                self.env.assign(&name, v)?;
                Ok(None)
            }
            Stmt::Print { expr } => {
                self.env.temp_enter();
                let v = self.eval_expr(expr)?;
                let s = self.value_to_string(&v)?;
                println!("{}", s);
                self.env.temp_exit_release()?;
                Ok(None)
            }
            Stmt::ExprStmt { expr } => {
                self.env.temp_enter();
                let _ = self.eval_expr(expr)?;
                self.env.temp_exit_release()?;
                Ok(None)
            }
            Stmt::Return(expr) => {
                let v = self.eval_expr(expr)?;
                Ok(Some(v))
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                let c = self.eval_expr(cond)?;
                if c.truthy()? {
                    self.env.enter_scope();
                    for st in then_body {
                        if let Some(ret) = self.exec_stmt(st)? {
                            // unwind scope before returning
                            self.env.exit_scope()?;
                            return Ok(Some(ret));
                        }
                    }
                    self.env.exit_scope()?;
                } else {
                    self.env.enter_scope();
                    for st in else_body {
                        if let Some(ret) = self.exec_stmt(st)? {
                            self.env.exit_scope()?;
                            return Ok(Some(ret));
                        }
                    }
                    self.env.exit_scope()?;
                }
                Ok(None)
            }
            Stmt::For { var, iter, body } => {
                let it = self.eval_expr(iter)?;
                let it = self.env.resolve_ref_for_read(&it)?;
                let Value::Vec(elems) = it else {
                    return Err(LangError::Runtime("for-in expects a vector".into()));
                };

                for elem in elems {
                    self.env.enter_scope();
                    self.env.define(var.clone(), elem);

                    for st in body.iter().cloned() {
                        if let Some(ret) = self.exec_stmt(st)? {
                            self.env.exit_scope()?;
                            return Ok(Some(ret));
                        }
                    }

                    self.env.exit_scope()?;
                }
                Ok(None)
            }
        }
    }

    fn eval_expr(&mut self, e: Expr) -> Result<Value, LangError> {
        match e {
            Expr::Num(n) => Ok(Value::Num(n)),
            Expr::Bool(b) => Ok(Value::Bool(b)),
            Expr::Str(s) => Ok(Value::Str(s)),
            Expr::VecLit(xs) => {
                let mut out = Vec::with_capacity(xs.len());
                for x in xs {
                    out.push(self.eval_expr(x)?);
                }
                Ok(Value::Vec(out))
            }
            Expr::Ident(name) => self.env.load_move(&name),

            Expr::Borrow { mut_, expr } => self.eval_borrow(mut_, *expr),

            Expr::Index { base, index } => {
                let (base_name, _base_mut) = self.parse_base_for_index(*base)?;
                let idxv = self.eval_expr(*index)?;
                let idxv = self.env.resolve_ref_for_read(&idxv)?;
                let idx = idxv.to_index_usize("index")?;

                let basev = self.env.load_peek(&base_name)?;
                match basev {
                    Value::Vec(xs) => xs
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| LangError::Runtime(format!("Index out of bounds: {}", idx))),
                    _ => Err(LangError::Runtime("Indexing expects a vector".into())),
                }
            }

            Expr::Neg { expr } => {
                let v = self.eval_expr(*expr)?;
                let v = self.env.resolve_ref_for_read(&v)?;
                match v {
                    Value::Num(x) => Ok(Value::Num(-x)),
                    _ => Err(LangError::Runtime("Unary '-' expects number".into())),
                }
            }

            Expr::Add { left, right } => self.bin_num(*left, *right, |a, b| a + b),
            Expr::Sub { left, right } => self.bin_num(*left, *right, |a, b| a - b),
            Expr::Mul { left, right } => self.bin_num(*left, *right, |a, b| a * b),
            Expr::Div { left, right } => {
                let (a, b) = self.bin_num2(*left, *right)?;
                if b == 0.0 {
                    return Err(LangError::Runtime("Division by zero".into()));
                }
                Ok(Value::Num(a / b))
            }
            Expr::Mod { left, right } => {
                let (a, b) = self.bin_num2(*left, *right)?;
                if b == 0.0 {
                    return Err(LangError::Runtime("Modulo by zero".into()));
                }
                Ok(Value::Num(a % b))
            }

            Expr::Eq { left, right } => self.bin_cmp(*left, *right, |a, b| a == b),
            Expr::Ne { left, right } => self.bin_cmp(*left, *right, |a, b| a != b),
            Expr::Lt { left, right } => self.bin_ord(*left, *right, |a, b| a < b),
            Expr::Le { left, right } => self.bin_ord(*left, *right, |a, b| a <= b),
            Expr::Gt { left, right } => self.bin_ord(*left, *right, |a, b| a > b),
            Expr::Ge { left, right } => self.bin_ord(*left, *right, |a, b| a >= b),

            Expr::Call { callee, args } => self.eval_call(&callee, args),
        }
    }

    fn eval_borrow(&mut self, mut_: bool, expr: Expr) -> Result<Value, LangError> {
        let m = if mut_ {
            Mutability::Mut
        } else {
            Mutability::Imm
        };

        match expr {
            Expr::Ident(name) => self.env.borrow_local(&name, m),
            Expr::Index { base, index } => {
                let (base_name, base_mut) = self.parse_base_for_index(*base)?;
                let idxv = self.eval_expr(*index)?;
                let idxv = self.env.resolve_ref_for_read(&idxv)?;
                let idx = idxv.to_index_usize("index")?;
                self.env.borrow_index(&base_name, base_mut, idx, m)
            }
            other => Err(LangError::Runtime(format!(
                "Unsupported borrow target (only var or index): {:?}",
                other
            ))),
        }
    }

    fn parse_base_for_index(&mut self, base: Expr) -> Result<(String, Mutability), LangError> {
        match base {
            Expr::Ident(name) => Ok((name, Mutability::Imm)),
            Expr::Borrow { mut_, expr } => match *expr {
                Expr::Ident(name) => Ok((
                    name,
                    if mut_ {
                        Mutability::Mut
                    } else {
                        Mutability::Imm
                    },
                )),
                other => Err(LangError::Runtime(format!(
                    "Index base must be ident or &ident / &mut ident, got {:?}",
                    other
                ))),
            },
            other => Err(LangError::Runtime(format!(
                "Index base must be ident or &ident / &mut ident, got {:?}",
                other
            ))),
        }
    }

    fn eval_call(&mut self, callee: &str, args: Vec<Expr>) -> Result<Value, LangError> {
        // eager-eval arguments
        let mut av = Vec::with_capacity(args.len());
        for a in args {
            av.push(self.eval_expr(a)?);
        }

        match callee {
            "len" => {
                if av.len() != 1 {
                    return Err(LangError::Runtime("len(x) expects 1 argument".into()));
                }
                let v = self.env.resolve_ref_for_read(&av[0])?;
                match v {
                    Value::Str(s) => Ok(Value::Num(s.len() as f64)),
                    Value::Vec(xs) => Ok(Value::Num(xs.len() as f64)),
                    _ => Err(LangError::Runtime("len expects string or vector".into())),
                }
            }
            "push" => {
                if av.len() != 2 {
                    return Err(LangError::Runtime(
                        "push(&mut vec, value) expects 2 arguments".into(),
                    ));
                }
                let r = &av[0];
                let v = av[1].clone();

                let (name, m) = match r {
                    Value::Ref(RefTarget::Local(n), m) => (n.clone(), m.clone()),
                    _ => {
                        return Err(LangError::Runtime(
                            "push first argument must be &mut vec variable".into(),
                        ));
                    }
                };
                if !matches!(m, Mutability::Mut) {
                    return Err(LangError::Runtime("push requires &mut vec".into()));
                }
                self.env.push_vec_elem(&name, v)?;
                Ok(Value::Unit)
            }
            "set" => {
                if av.len() != 2 {
                    return Err(LangError::Runtime(
                        "set(&mut elem_ref, value) expects 2 arguments".into(),
                    ));
                }
                let r = &av[0];
                let v = av[1].clone();

                let (base, idx, m) = match r {
                    Value::Ref(RefTarget::VecElem { base, index }, m) => {
                        (base.clone(), *index, m.clone())
                    }
                    _ => {
                        return Err(LangError::Runtime(
                            "set first argument must be &mut (xs[i])".into(),
                        ));
                    }
                };
                if !matches!(m, Mutability::Mut) {
                    return Err(LangError::Runtime("set requires &mut element ref".into()));
                }
                self.env.set_vec_elem(&base, idx, v)?;
                Ok(Value::Unit)
            }
            "range" => {
                if !(av.len() == 1 || av.len() == 2) {
                    return Err(LangError::Runtime(
                        "range(end) or range(start, end) expects 1 or 2 arguments".into(),
                    ));
                }
                let (start, end) = if av.len() == 1 {
                    (Value::Num(0.0), av[0].clone())
                } else {
                    (av[0].clone(), av[1].clone())
                };

                let s = self.env.resolve_ref_for_read(&start)?.as_num()?;
                let e = self.env.resolve_ref_for_read(&end)?.as_num()?;
                let s = Value::Num(s).to_index_usize("range start")? as i64;
                let e = Value::Num(e).to_index_usize("range end")? as i64;

                let mut out = Vec::new();
                let mut i = s;
                while i < e {
                    out.push(Value::Num(i as f64));
                    i += 1;
                }
                Ok(Value::Vec(out))
            }
            _ => {
                // user function
                let f =
                    self.fns.get(callee).cloned().ok_or_else(|| {
                        LangError::Runtime(format!("Unknown function: {}", callee))
                    })?;

                if f.params.len() != av.len() {
                    return Err(LangError::Runtime(format!(
                        "Function {} expects {} args, got {}",
                        callee,
                        f.params.len(),
                        av.len()
                    )));
                }

                self.env.enter_scope();
                for (p, a) in f.params.iter().zip(av.into_iter()) {
                    self.env.define(p.clone(), a);
                }

                let mut ret = Value::Unit;
                for st in f.body {
                    if let Some(v) = self.exec_stmt(st)? {
                        ret = v;
                        break;
                    }
                }
                self.env.exit_scope()?;
                Ok(ret)
            }
        }
    }

    fn bin_num<F: FnOnce(f64, f64) -> f64>(
        &mut self,
        l: Expr,
        r: Expr,
        f: F,
    ) -> Result<Value, LangError> {
        let (a, b) = self.bin_num2(l, r)?;
        Ok(Value::Num(f(a, b)))
    }

    fn bin_num2(&mut self, l: Expr, r: Expr) -> Result<(f64, f64), LangError> {
        let a = self.eval_expr(l)?;
        let b = self.eval_expr(r)?;
        let a = self.env.resolve_ref_for_read(&a)?.as_num()?;
        let b = self.env.resolve_ref_for_read(&b)?.as_num()?;
        Ok((a, b))
    }

    fn bin_ord<F: FnOnce(f64, f64) -> bool>(
        &mut self,
        l: Expr,
        r: Expr,
        f: F,
    ) -> Result<Value, LangError> {
        let (a, b) = self.bin_num2(l, r)?;
        Ok(Value::Bool(f(a, b)))
    }

    fn bin_cmp<F: FnOnce(Value, Value) -> bool>(
        &mut self,
        l: Expr,
        r: Expr,
        f: F,
    ) -> Result<Value, LangError> {
        let a = self.eval_expr(l)?;
        let b = self.eval_expr(r)?;
        let a = self.env.resolve_ref_for_read(&a)?;
        let b = self.env.resolve_ref_for_read(&b)?;
        Ok(Value::Bool(f(a, b)))
    }

    fn value_to_string(&self, v: &Value) -> Result<String, LangError> {
        // IMPORTANT: deref refs for display
        let v = self.env.resolve_ref_for_read(v)?;

        match v {
            Value::Num(n) => {
                if n.fract() == 0.0 {
                    Ok(format!("{}", n as i64))
                } else {
                    Ok(format!("{}", n))
                }
            }
            Value::Bool(b) => Ok(b.to_string()),
            Value::Str(s) => Ok(s),
            Value::Vec(xs) => {
                let mut out = String::from("[");
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&self.value_to_string(x)?); // recursive (deref inside)
                }
                out.push(']');
                Ok(out)
            }
            Value::Unit => Ok("()".into()),
            Value::Ref(_, _) => unreachable!("resolved above"),
        }
    }
}
