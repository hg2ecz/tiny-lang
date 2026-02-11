#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Stmt(Stmt),
    Fn(FnDef),
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        expr: Expr,
    },
    Assign {
        name: String,
        expr: Expr,
    },

    Print {
        expr: Expr,
    },

    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },

    For {
        var: String,
        iter: Expr,
        body: Vec<Stmt>,
    },

    Return(Expr),
    ExprStmt {
        expr: Expr,
    },
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Bool(bool),
    Str(String),
    VecLit(Vec<Expr>),
    Ident(String),

    // Borrowing / indexing
    Borrow { mut_: bool, expr: Box<Expr> }, // &x, &mut x, &xs[i], &mut xs[i]
    Index { base: Box<Expr>, index: Box<Expr> }, // xs[i], (&mut xs)[i]

    // Arithmetic
    Add { left: Box<Expr>, right: Box<Expr> },
    Sub { left: Box<Expr>, right: Box<Expr> },
    Mul { left: Box<Expr>, right: Box<Expr> },
    Div { left: Box<Expr>, right: Box<Expr> },
    Mod { left: Box<Expr>, right: Box<Expr> },
    Neg { expr: Box<Expr> },

    // Comparisons
    Eq { left: Box<Expr>, right: Box<Expr> },
    Ne { left: Box<Expr>, right: Box<Expr> },
    Lt { left: Box<Expr>, right: Box<Expr> },
    Le { left: Box<Expr>, right: Box<Expr> },
    Gt { left: Box<Expr>, right: Box<Expr> },
    Ge { left: Box<Expr>, right: Box<Expr> },

    // Calls
    Call { callee: String, args: Vec<Expr> },
}
