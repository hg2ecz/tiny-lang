#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Let,
    Print,
    Fn,
    For,
    In,
    Return,
    Mut,
    If,
    Else,
    True,
    False,

    // Ident + literals
    Ident(String),
    Num(f64),
    Str(String),

    // Operators / punct
    Amp,     // &
    Plus,    // +
    Minus,   // -
    Star,    // *
    Slash,   // /
    Percent, // %

    Eq,    // =
    EqEq,  // ==
    NotEq, // !=

    Lt, // <
    Le, // <=
    Gt, // >
    Ge, // >=

    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semi,

    Eof,
}
