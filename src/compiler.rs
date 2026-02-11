use std::collections::HashMap;

use crate::ast::{Expr, FnDef, Item, Program, Stmt};
use crate::bytecode::{
    BytecodeProgram, Chunk, ChunkId, Instr, LoadMode, LocalId, Mutability, StrId,
};
use crate::error::LangError;

#[derive(Debug, Default)]
pub struct Compiler {
    // global string interner (also used for function names)
    interner: HashMap<String, StrId>,
    strings: Vec<String>,

    // output
    chunks: Vec<Chunk>,
    fns: HashMap<StrId, ChunkId>,

    // per-compilation-unit state
    scopes: Vec<HashMap<StrId, LocalId>>,
    next_local: LocalId,
}

impl Compiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn compile_program(&mut self, p: Program) -> Result<BytecodeProgram, LangError> {
        // 1) allocate chunk ids for functions first (allows forward calls)
        for it in &p.items {
            if let Item::Fn(f) = it {
                let name = self.intern(&f.name);
                if self.fns.contains_key(&name) {
                    return Err(LangError::Runtime(format!(
                        "Duplicate function: {}",
                        f.name
                    )));
                }
                let cid = self.alloc_chunk();
                self.fns.insert(name, cid);
            }
        }

        // 2) compile each function
        for it in &p.items {
            if let Item::Fn(f) = it {
                self.compile_fn(f.clone())?;
            }
        }

        // 3) compile top-level statements as main
        let main = self.alloc_chunk();
        self.begin_chunk();
        for it in p.items {
            if let Item::Stmt(s) = it {
                if matches!(s, Stmt::Return(_)) {
                    return Err(LangError::Runtime(
                        "return is only allowed inside functions".into(),
                    ));
                }
                self.compile_stmt(main, s)?;
            }
        }
        self.end_chunk(main);

        Ok(BytecodeProgram {
            strings: std::mem::take(&mut self.strings),
            main,
            chunks: std::mem::take(&mut self.chunks),
            fns: std::mem::take(&mut self.fns),
        })
    }

    fn compile_fn(&mut self, f: FnDef) -> Result<(), LangError> {
        let name_id = self.intern(&f.name);
        let cid = *self.fns.get(&name_id).expect("allocated earlier");

        // reset per-function locals/scopes
        self.begin_chunk();

        // params are locals 0..n
        {
            let chunk = &mut self.chunks[cid as usize];
            chunk.code.clear();
            chunk.params.clear();
        }
        for p in &f.params {
            let pid = self.intern(p);
            let l = self.define_local(pid);
            self.chunks[cid as usize].params.push(l);
        }

        for st in f.body {
            self.compile_stmt(cid, st)?;
        }

        // implicit return ()
        self.chunks[cid as usize].code.push(Instr::PushUnit);
        self.chunks[cid as usize].code.push(Instr::Return);

        self.end_chunk(cid);
        Ok(())
    }

    fn compile_stmt(&mut self, cid: ChunkId, s: Stmt) -> Result<(), LangError> {
        let code = &mut self.chunks[cid as usize].code;
        match s {
            Stmt::Let { name, expr } => {
                self.compile_expr(cid, expr)?;
                let id = self.intern(&name);
                let local = self.define_local(id);
                code.push(Instr::DefineLocal(local));
            }
            Stmt::Assign { name, expr } => {
                let id = self.intern(&name);
                let local = self.resolve_local(id).ok_or_else(|| {
                    LangError::Runtime(format!("Unknown variable: {}", name))
                })?;
                self.compile_expr(cid, expr)?;
                code.push(Instr::StoreLocal(local));
            }
            Stmt::Print { expr } => {
                code.push(Instr::TempEnter);
                self.compile_expr(cid, expr)?;
                code.push(Instr::Print);
                code.push(Instr::TempExit);
            }
            Stmt::ExprStmt { expr } => {
                code.push(Instr::TempEnter);
                self.compile_expr(cid, expr)?;
                code.push(Instr::Pop);
                code.push(Instr::TempExit);
            }
            Stmt::Return(expr) => {
                self.compile_expr(cid, expr)?;
                code.push(Instr::Return);
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                self.compile_expr(cid, cond)?;
                let jf_pos = code.len();
                code.push(Instr::JumpIfFalse(usize::MAX));

                code.push(Instr::EnterScope);
                self.push_scope();
                for st in then_body {
                    self.compile_stmt(cid, st)?;
                }
                self.pop_scope();
                code.push(Instr::ExitScope);

                let j_pos = code.len();
                code.push(Instr::Jump(usize::MAX));

                let else_start = code.len();
                code[jf_pos] = Instr::JumpIfFalse(else_start);

                code.push(Instr::EnterScope);
                self.push_scope();
                for st in else_body {
                    self.compile_stmt(cid, st)?;
                }
                self.pop_scope();
                code.push(Instr::ExitScope);

                let end = code.len();
                code[j_pos] = Instr::Jump(end);
            }
            Stmt::For { var, iter, body } => {
                // compile iterable expression (must yield Vec)
                self.compile_expr(cid, iter)?;

                // loop variable is a local; each iteration defines (assigns) into it
                let var_id = self.intern(&var);
                let var_local = self.define_local(var_id);

                let init_pos = code.len();
                code.push(Instr::ForInit {
                    var: var_local,
                    body_start: usize::MAX,
                    end: usize::MAX,
                });

                let body_start = code.len();
                code[init_pos] = Instr::ForInit {
                    var: var_local,
                    body_start,
                    end: usize::MAX,
                };

                code.push(Instr::EnterScope);
                self.push_scope();
                // the loop variable is in scope for the body
                self.current_scope_mut().insert(var_id, var_local);

                for st in body {
                    self.compile_stmt(cid, st)?;
                }

                self.pop_scope();
                code.push(Instr::ExitScope);

                let next_pos = code.len();
                code.push(Instr::ForNext {
                    body_start,
                    end: usize::MAX,
                });

                let end = code.len();
                if let Instr::ForInit {
                    var: _,
                    body_start: _,
                    end: ref mut e,
                } = &mut code[init_pos]
                {
                    *e = end;
                }
                code[next_pos] = Instr::ForNext { body_start, end };
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, cid: ChunkId, e: Expr) -> Result<(), LangError> {
        let code = &mut self.chunks[cid as usize].code;
        match e {
            Expr::Num(n) => code.push(Instr::PushNum(n)),
            Expr::Bool(b) => code.push(Instr::PushBool(b)),
            Expr::Str(s) => code.push(Instr::PushStr(self.intern(&s))),
            Expr::VecLit(xs) => {
                for x in xs.iter().cloned() {
                    self.compile_expr(cid, x)?;
                }
                code.push(Instr::VecMake(xs.len() as u32));
            }
            Expr::Ident(name) => {
                let id = self.intern(&name);
                let local = self
                    .resolve_local(id)
                    .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;
                code.push(Instr::LoadLocal {
                    local,
                    mode: LoadMode::Move,
                });
            }
            Expr::Borrow { mut_, expr } => {
                self.compile_borrow(cid, mut_, *expr)?;
            }
            Expr::Index { base, index } => {
                // base must be: ident | &ident | &mut ident
                let (base_local, base_mut) = self.parse_base_for_index(*base)?;
                self.compile_expr(cid, *index)?;
                code.push(Instr::IndexLocal {
                    base: base_local,
                    base_mut,
                });
            }

            Expr::Neg { expr } => {
                self.compile_expr(cid, *expr)?;
                code.push(Instr::Neg);
            }
            Expr::Add { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Add);
            }
            Expr::Sub { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Sub);
            }
            Expr::Mul { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Mul);
            }
            Expr::Div { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Div);
            }
            Expr::Mod { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Mod);
            }

            Expr::Eq { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Eq);
            }
            Expr::Ne { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Ne);
            }
            Expr::Lt { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Lt);
            }
            Expr::Le { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Le);
            }
            Expr::Gt { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Gt);
            }
            Expr::Ge { left, right } => {
                self.compile_expr(cid, *left)?;
                self.compile_expr(cid, *right)?;
                code.push(Instr::Ge);
            }

            Expr::Call { callee, args } => {
                let callee_id = self.intern(&callee);

                // lightweight compile-time check: known function or builtin
                if !self.fns.contains_key(&callee_id)
                    && !matches!(callee.as_str(), "len" | "push" | "set" | "range")
                {
                    return Err(LangError::Runtime(format!("Unknown function: {}", callee)));
                }

                for a in args.iter().cloned() {
                    self.compile_expr(cid, a)?;
                }
                code.push(Instr::Call {
                    callee: callee_id,
                    argc: args.len() as u32,
                });
            }
        }
        Ok(())
    }

    fn compile_borrow(&mut self, cid: ChunkId, mut_: bool, expr: Expr) -> Result<(), LangError> {
        let code = &mut self.chunks[cid as usize].code;
        match expr {
            Expr::Ident(name) => {
                let id = self.intern(&name);
                let local = self
                    .resolve_local(id)
                    .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;
                code.push(Instr::BorrowLocal { local, mut_ });
                Ok(())
            }
            Expr::Index { base, index } => {
                let (base_local, base_mut) = self.parse_base_for_index(*base)?;
                self.compile_expr(cid, *index)?;
                code.push(Instr::BorrowIndexLocal {
                    base: base_local,
                    base_mut,
                    mut_,
                });
                Ok(())
            }
            other => Err(LangError::Runtime(format!(
                "Unsupported borrow target (only var or index): {:?}",
                other
            ))),
        }
    }

    fn parse_base_for_index(&mut self, base: Expr) -> Result<(LocalId, Mutability), LangError> {
        match base {
            Expr::Ident(name) => {
                let id = self.intern(&name);
                let local = self
                    .resolve_local(id)
                    .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;
                Ok((local, Mutability::Imm))
            }
            Expr::Borrow { mut_, expr } => match *expr {
                Expr::Ident(name) => {
                    let id = self.intern(&name);
                    let local = self
                        .resolve_local(id)
                        .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;
                    Ok((
                        local,
                        if mut_ { Mutability::Mut } else { Mutability::Imm },
                    ))
                }
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

    // ---- locals / scopes ----

    fn begin_chunk(&mut self) {
        self.scopes.clear();
        self.scopes.push(HashMap::new());
        self.next_local = 0;
    }

    fn end_chunk(&mut self, cid: ChunkId) {
        let lc = self.next_local as usize;
        self.chunks[cid as usize].local_count = lc;
        self.scopes.clear();
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn current_scope_mut(&mut self) -> &mut HashMap<StrId, LocalId> {
        self.scopes.last_mut().unwrap()
    }

    fn define_local(&mut self, name: StrId) -> LocalId {
        let local = self.next_local;
        self.next_local += 1;
        self.current_scope_mut().insert(name, local);
        local
    }

    fn resolve_local(&self, name: StrId) -> Option<LocalId> {
        for scope in self.scopes.iter().rev() {
            if let Some(l) = scope.get(&name) {
                return Some(*l);
            }
        }
        None
    }

    // ---- program / chunks / strings ----

    fn alloc_chunk(&mut self) -> ChunkId {
        let id = self.chunks.len() as ChunkId;
        self.chunks.push(Chunk::new());
        id
    }

    fn intern(&mut self, s: &str) -> StrId {
        if let Some(id) = self.interner.get(s) {
            return *id;
        }
        let id = self.strings.len() as StrId;
        self.strings.push(s.to_string());
        self.interner.insert(s.to_string(), id);
        id
    }
}
