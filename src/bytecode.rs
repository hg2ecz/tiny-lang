use std::collections::HashMap;

pub type StrId = u32;
pub type ChunkId = u32;
pub type LocalId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutability {
    Imm,
    Mut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    /// Rust-like: move unless the value is Copy.
    Move,
    /// Clone (never moves).
    Peek,
}

#[derive(Debug, Clone)]
pub enum Instr {
    // --- stack constants ---
    PushNum(f64),
    PushBool(bool),
    PushStr(StrId),
    PushUnit,

    // --- aggregate construction ---
    VecMake(u32), // pops N values

    // --- locals (fast: indexed, no HashMap at runtime) ---
    LoadLocal { local: LocalId, mode: LoadMode },
    DefineLocal(LocalId), // pop value, bind
    StoreLocal(LocalId),  // pop value, assign

    // --- arithmetic / comparisons ---
    Neg,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,

    // --- borrow / index ---
    BorrowLocal { local: LocalId, mut_: bool },
    // base = xs | &xs | &mut xs (compiler validates base form)
    BorrowIndexLocal {
        base: LocalId,
        base_mut: Mutability,
        mut_: bool,
    },
    IndexLocal { base: LocalId, base_mut: Mutability },

    // --- calls ---
    Call { callee: StrId, argc: u32 },
    Return,

    // --- statements ---
    Print,
    Pop,

    // --- scoping / borrow lifetimes ---
    EnterScope,
    ExitScope,
    TempEnter,
    TempExit,

    // --- control flow ---
    Jump(usize),
    JumpIfFalse(usize),

    // --- for-loops ---
    ForInit {
        var: LocalId,
        body_start: usize,
        end: usize,
    },
    ForNext {
        body_start: usize,
        end: usize,
    },
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub code: Vec<Instr>,
    pub params: Vec<LocalId>,
    pub local_count: usize,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            params: Vec::new(),
            local_count: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BytecodeProgram {
    pub strings: Vec<String>,
    pub main: ChunkId,
    pub chunks: Vec<Chunk>,
    pub fns: HashMap<StrId, ChunkId>,
}
