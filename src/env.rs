use crate::error::LangError;
use crate::value::{Mutability, RefTarget, Value};

#[derive(Debug, Default, Clone, Copy)]
pub struct BorrowState {
    pub immut: usize,
    pub mut_: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Slot {
    pub value: Option<Value>,
    pub borrows: BorrowState,
}

#[derive(Debug, Default)]
pub struct Env {
    // stack-scope: Vec<HashMap<name, Slot>> helyett gyorsabb: egy "frame stack"
    // de ha nálad már HashMap-es volt, tartsuk egyszerűen:
    frames: Vec<std::collections::HashMap<String, Slot>>,

    // borrows release log: scope + temp statement frame
    scope_log: Vec<Vec<(RefTarget, Mutability)>>,
    temp_log: Vec<Vec<(RefTarget, Mutability)>>,
}

impl Env {
    pub fn new() -> Self {
        let mut e = Self::default();
        e.enter_scope();
        e
    }

    pub fn enter_scope(&mut self) {
        self.frames.push(std::collections::HashMap::new());
        self.scope_log.push(Vec::new());
    }

    pub fn exit_scope(&mut self) -> Result<(), LangError> {
        self.release_scope_borrows()?;
        self.frames
            .pop()
            .ok_or_else(|| LangError::Runtime("Scope underflow".into()))?;
        Ok(())
    }

    pub fn temp_enter(&mut self) {
        self.temp_log.push(Vec::new());
    }

    pub fn temp_exit_release(&mut self) -> Result<(), LangError> {
        let log = self
            .temp_log
            .pop()
            .ok_or_else(|| LangError::Runtime("Temp underflow".into()))?;
        for (tgt, m) in log.into_iter().rev() {
            self.release_borrow(&tgt, m)?;
        }
        Ok(())
    }

    fn release_scope_borrows(&mut self) -> Result<(), LangError> {
        let log = self
            .scope_log
            .pop()
            .ok_or_else(|| LangError::Runtime("Internal: scope_log underflow".into()))?;
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

    fn lookup_slot_mut(&mut self, name: &str) -> Option<&mut Slot> {
        for f in self.frames.iter_mut().rev() {
            if f.contains_key(name) {
                return f.get_mut(name);
            }
        }
        None
    }

    fn lookup_slot_ref(&self, name: &str) -> Option<&Slot> {
        for f in self.frames.iter().rev() {
            if f.contains_key(name) {
                return f.get(name);
            }
        }
        None
    }

    pub fn define(&mut self, name: String, v: Value) {
        let top = self.frames.last_mut().unwrap();
        top.insert(
            name,
            Slot {
                value: Some(v),
                borrows: BorrowState::default(),
            },
        );
    }

    pub fn assign(&mut self, name: &str, v: Value) -> Result<(), LangError> {
        let slot = self
            .lookup_slot_mut(name)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;

        if slot.borrows.immut > 0 || slot.borrows.mut_ {
            return Err(LangError::Runtime("Cannot assign: it is borrowed".into()));
        }
        slot.value = Some(v);
        Ok(())
    }

    pub fn load_move(&mut self, name: &str) -> Result<Value, LangError> {
        let slot = self
            .lookup_slot_mut(name)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;

        let Some(v) = slot.value.take() else {
            return Err(LangError::Runtime("Use after move".into()));
        };

        if v.is_copy() {
            slot.value = Some(v.clone());
            Ok(v)
        } else {
            Ok(v)
        }
    }

    pub fn load_peek(&self, name: &str) -> Result<Value, LangError> {
        let slot = self
            .lookup_slot_ref(name)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;
        slot.value
            .clone()
            .ok_or_else(|| LangError::Runtime("Use after move".into()))
    }

    pub fn borrow_local(&mut self, name: &str, m: Mutability) -> Result<Value, LangError> {
        // ensure exists + not moved
        {
            let slot = self
                .lookup_slot_ref(name)
                .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;
            if slot.value.is_none() {
                return Err(LangError::Runtime("Cannot borrow: value was moved".into()));
            }
        }

        let slot = self
            .lookup_slot_mut(name)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", name)))?;

        match m {
            Mutability::Imm => {
                if slot.borrows.mut_ {
                    return Err(LangError::Runtime(
                        "Cannot immut-borrow: mut-borrowed".into(),
                    ));
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

        let tgt = RefTarget::Local(name.to_string());
        self.record_borrow(tgt.clone(), m.clone());
        Ok(Value::Ref(tgt, m))
    }

    pub fn borrow_index(
        &mut self,
        base: &str,
        base_mut: Mutability,
        idx: usize,
        m: Mutability,
    ) -> Result<Value, LangError> {
        // borrow base container first (as dictated by base form)
        let _ = self.borrow_local(base, base_mut)?;

        // element ref
        let tgt = RefTarget::VecElem {
            base: base.to_string(),
            index: idx,
        };
        self.record_borrow(tgt.clone(), m.clone());
        Ok(Value::Ref(tgt, m))
    }

    fn release_borrow(&mut self, tgt: &RefTarget, m: Mutability) -> Result<(), LangError> {
        let local_name = match tgt {
            RefTarget::Local(n) => n.as_str(),
            RefTarget::VecElem { base, .. } => base.as_str(),
        };

        let slot = self
            .lookup_slot_mut(local_name)
            .ok_or_else(|| LangError::Runtime("Internal: release unknown slot".into()))?;

        match m {
            Mutability::Imm => {
                if slot.borrows.immut == 0 {
                    return Err(LangError::Runtime(
                        "Internal: immut borrow underflow".into(),
                    ));
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

    pub fn resolve_ref_for_read(&self, v: &Value) -> Result<Value, LangError> {
        match v {
            Value::Ref(RefTarget::Local(name), _) => self.load_peek(name),
            Value::Ref(RefTarget::VecElem { base, index }, _) => {
                let basev = self.load_peek(base)?;
                match basev {
                    Value::Vec(xs) => xs.get(*index).cloned().ok_or_else(|| {
                        LangError::Runtime(format!("Index out of bounds: {}", index))
                    }),
                    _ => Err(LangError::Runtime("Index base is not a vector".into())),
                }
            }
            other => Ok(other.clone()),
        }
    }

    pub fn push_vec_elem(&mut self, base: &str, v: Value) -> Result<(), LangError> {
        let slot = self
            .lookup_slot_mut(base)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", base)))?;

        if slot.borrows.immut > 0 {
            return Err(LangError::Runtime(
                "Cannot push: vector is immut-borrowed".into(),
            ));
        }
        if slot.value.is_none() {
            return Err(LangError::Runtime("Use after move".into()));
        }

        match slot.value.as_mut().unwrap() {
            Value::Vec(xs) => {
                xs.push(v);
                Ok(())
            }
            _ => Err(LangError::Runtime("push target must be a vector".into())),
        }
    }

    pub fn set_vec_elem(&mut self, base: &str, idx: usize, v: Value) -> Result<(), LangError> {
        let slot = self
            .lookup_slot_mut(base)
            .ok_or_else(|| LangError::Runtime(format!("Unknown variable: {}", base)))?;
        if slot.value.is_none() {
            return Err(LangError::Runtime("Use after move".into()));
        }
        match slot.value.as_mut().unwrap() {
            Value::Vec(xs) => {
                if idx >= xs.len() {
                    return Err(LangError::Runtime(format!(
                        "Index out of bounds: {} (len {})",
                        idx,
                        xs.len()
                    )));
                }
                xs[idx] = v;
                Ok(())
            }
            _ => Err(LangError::Runtime("set target must be a vector".into())),
        }
    }
}
