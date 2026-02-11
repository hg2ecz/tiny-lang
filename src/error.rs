#[derive(Debug)]
pub enum LangError {
    Lex(String),
    Parse(String),
    Runtime(String),
}

impl std::fmt::Display for LangError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LangError::Lex(s) => write!(f, "Lex error: {}", s),
            LangError::Parse(s) => write!(f, "Parse error: {}", s),
            LangError::Runtime(s) => write!(f, "Runtime error: {}", s),
        }
    }
}
impl std::error::Error for LangError {}
