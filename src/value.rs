use crate::error::LangError;

#[derive(Debug, Clone, PartialEq)]
pub enum Mutability {
    Imm,
    Mut,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RefTarget {
    Local(String),
    VecElem { base: String, index: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Bool(bool),
    Str(String),
    Vec(Vec<Value>),
    Unit,
    Ref(RefTarget, Mutability),
}

impl Value {
    pub fn is_copy(&self) -> bool {
        matches!(self, Value::Num(_) | Value::Bool(_))
    }

    pub fn as_num(&self) -> Result<f64, LangError> {
        match self {
            Value::Num(n) => Ok(*n),
            _ => Err(LangError::Runtime("Expected number".into())),
        }
    }

    pub fn truthy(&self) -> Result<bool, LangError> {
        match self {
            Value::Bool(b) => Ok(*b),
            _ => Err(LangError::Runtime("Condition must be bool".into())),
        }
    }

    pub fn to_index_usize(&self, what: &str) -> Result<usize, LangError> {
        let n = self.as_num()?;
        if !n.is_finite() {
            return Err(LangError::Runtime(format!("{} must be finite", what)));
        }
        if n < 0.0 {
            return Err(LangError::Runtime(format!("{} must be >= 0", what)));
        }
        if n.fract() != 0.0 {
            return Err(LangError::Runtime(format!(
                "{} must be an integer number",
                what
            )));
        }
        let u = n as u64;
        usize::try_from(u).map_err(|_| LangError::Runtime(format!("{} too large", what)))
    }
}
