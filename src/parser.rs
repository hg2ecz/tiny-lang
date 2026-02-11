use crate::ast::{Expr, FnDef, Item, Program, Stmt};
use crate::error::LangError;
use crate::lexer::Lexer;
use crate::token::Token;

pub struct Parser<'a> {
    lex: Lexer<'a>,
    cur: Token,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Result<Self, LangError> {
        let mut lex = Lexer::new(input);
        let cur = lex.next_token()?;
        Ok(Self { lex, cur })
    }

    fn bump(&mut self) -> Result<(), LangError> {
        self.cur = self.lex.next_token()?;
        Ok(())
    }

    fn expect(&mut self, t: Token) -> Result<(), LangError> {
        if self.cur == t {
            self.bump()
        } else {
            Err(LangError::Parse(format!(
                "Expected {:?}, got {:?}",
                t, self.cur
            )))
        }
    }

    fn expect_ident(&mut self) -> Result<String, LangError> {
        match &self.cur {
            Token::Ident(s) => {
                let v = s.clone();
                self.bump()?;
                Ok(v)
            }
            _ => Err(LangError::Parse(format!(
                "Expected identifier, got {:?}",
                self.cur
            ))),
        }
    }

    pub fn parse_program(&mut self) -> Result<Program, LangError> {
        let mut items = Vec::new();
        while self.cur != Token::Eof {
            if self.cur == Token::Fn {
                items.push(Item::Fn(self.parse_fn_def()?));
            } else {
                items.push(Item::Stmt(self.parse_stmt()?));
            }
        }
        Ok(Program { items })
    }

    fn parse_fn_def(&mut self) -> Result<FnDef, LangError> {
        self.expect(Token::Fn)?;
        let name = self.expect_ident()?;
        self.expect(Token::LParen)?;
        let mut params = Vec::new();
        if self.cur != Token::RParen {
            params.push(self.expect_ident()?);
            while self.cur == Token::Comma {
                self.bump()?;
                params.push(self.expect_ident()?);
            }
        }
        self.expect(Token::RParen)?;
        let body = self.parse_block()?;
        Ok(FnDef { name, params, body })
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, LangError> {
        self.expect(Token::LBrace)?;
        let mut stmts = Vec::new();
        while self.cur != Token::RBrace {
            if self.cur == Token::Eof {
                return Err(LangError::Parse("Unclosed block".into()));
            }
            stmts.push(self.parse_stmt()?);
        }
        self.expect(Token::RBrace)?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, LangError> {
        match &self.cur {
            Token::Let => self.parse_let(),
            Token::Print => self.parse_print(),
            Token::For => self.parse_for(),
            Token::Return => self.parse_return(),
            Token::If => self.parse_if(),
            Token::Ident(_) => {
                // assign or exprstmt
                let saved = self.cur.clone();
                self.bump()?; // consume ident
                if self.cur == Token::Eq {
                    let name = match saved {
                        Token::Ident(s) => s,
                        _ => unreachable!(),
                    };
                    self.bump()?; // '='
                    let expr = self.parse_expr()?;
                    self.expect(Token::Semi)?;
                    Ok(Stmt::Assign { name, expr })
                } else {
                    // expression statement starting with already-consumed ident
                    let name = match saved {
                        Token::Ident(s) => s,
                        _ => unreachable!(),
                    };
                    let left = Expr::Ident(name);
                    let expr = self.parse_expr_with_left(left)?;
                    self.expect(Token::Semi)?;
                    Ok(Stmt::ExprStmt { expr })
                }
            }
            _ => {
                let expr = self.parse_expr()?;
                self.expect(Token::Semi)?;
                Ok(Stmt::ExprStmt { expr })
            }
        }
    }

    fn parse_let(&mut self) -> Result<Stmt, LangError> {
        self.expect(Token::Let)?;
        let name = self.expect_ident()?;
        self.expect(Token::Eq)?;
        let expr = self.parse_expr()?;
        self.expect(Token::Semi)?;
        Ok(Stmt::Let { name, expr })
    }

    fn parse_print(&mut self) -> Result<Stmt, LangError> {
        self.expect(Token::Print)?;
        let expr = self.parse_expr()?;
        self.expect(Token::Semi)?;
        Ok(Stmt::Print { expr })
    }

    fn parse_for(&mut self) -> Result<Stmt, LangError> {
        self.expect(Token::For)?;
        let var = self.expect_ident()?;
        self.expect(Token::In)?;
        let iter = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt::For { var, iter, body })
    }

    fn parse_return(&mut self) -> Result<Stmt, LangError> {
        self.expect(Token::Return)?;
        let e = self.parse_expr()?;
        self.expect(Token::Semi)?;
        Ok(Stmt::Return(e))
    }

    fn parse_if(&mut self) -> Result<Stmt, LangError> {
        self.expect(Token::If)?;
        let cond = self.parse_expr()?;
        let then_body = self.parse_block()?;

        let else_body = if self.cur == Token::Else {
            self.bump()?;
            self.parse_block()?
        } else {
            Vec::new()
        };

        Ok(Stmt::If {
            cond,
            then_body,
            else_body,
        })
    }

    // Expression grammar (precedence):
    // expr      := equality
    // equality  := compare ( (==|!=) compare )*
    // compare   := add ( (<|<=|>|>=) add )*
    // add       := mul ( (+|-) mul )*
    // mul       := unary ( (*|/|%) unary )*
    // unary     := (& (mut) unary) | (- unary) | postfix
    // postfix   := primary ( (args) | [expr] )*
    // primary   := int | str | true | false | ident | vec | (expr)

    fn parse_expr(&mut self) -> Result<Expr, LangError> {
        self.parse_equality()
    }

    // Used for stmt lookahead: we already consumed an Ident and want to continue parsing as expr.
    fn parse_expr_with_left(&mut self, left_primary: Expr) -> Result<Expr, LangError> {
        let mut e = self.parse_postfix_with_left(left_primary)?;
        e = self.parse_mul_tail(e)?;
        e = self.parse_add_tail(e)?;
        e = self.parse_compare_tail(e)?;
        e = self.parse_equality_tail(e)?;
        Ok(e)
    }

    fn parse_equality(&mut self) -> Result<Expr, LangError> {
        let mut e = self.parse_compare()?;
        e = self.parse_equality_tail(e)?;
        Ok(e)
    }

    fn parse_equality_tail(&mut self, mut e: Expr) -> Result<Expr, LangError> {
        loop {
            match self.cur {
                Token::EqEq => {
                    self.bump()?;
                    let rhs = self.parse_compare()?;
                    e = Expr::Eq {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                Token::NotEq => {
                    self.bump()?;
                    let rhs = self.parse_compare()?;
                    e = Expr::Ne {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_compare(&mut self) -> Result<Expr, LangError> {
        let mut e = self.parse_add()?;
        e = self.parse_compare_tail(e)?;
        Ok(e)
    }

    fn parse_compare_tail(&mut self, mut e: Expr) -> Result<Expr, LangError> {
        loop {
            match self.cur {
                Token::Lt => {
                    self.bump()?;
                    let rhs = self.parse_add()?;
                    e = Expr::Lt {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                Token::Le => {
                    self.bump()?;
                    let rhs = self.parse_add()?;
                    e = Expr::Le {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                Token::Gt => {
                    self.bump()?;
                    let rhs = self.parse_add()?;
                    e = Expr::Gt {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                Token::Ge => {
                    self.bump()?;
                    let rhs = self.parse_add()?;
                    e = Expr::Ge {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_add(&mut self) -> Result<Expr, LangError> {
        let mut e = self.parse_mul()?;
        e = self.parse_add_tail(e)?;
        Ok(e)
    }

    fn parse_add_tail(&mut self, mut e: Expr) -> Result<Expr, LangError> {
        loop {
            match self.cur {
                Token::Plus => {
                    self.bump()?;
                    let rhs = self.parse_mul()?;
                    e = Expr::Add {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                Token::Minus => {
                    self.bump()?;
                    let rhs = self.parse_mul()?;
                    e = Expr::Sub {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_mul(&mut self) -> Result<Expr, LangError> {
        let mut e = self.parse_unary()?;
        e = self.parse_mul_tail(e)?;
        Ok(e)
    }

    fn parse_mul_tail(&mut self, mut e: Expr) -> Result<Expr, LangError> {
        loop {
            match self.cur {
                Token::Star => {
                    self.bump()?;
                    let rhs = self.parse_unary()?;
                    e = Expr::Mul {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                Token::Slash => {
                    self.bump()?;
                    let rhs = self.parse_unary()?;
                    e = Expr::Div {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                Token::Percent => {
                    self.bump()?;
                    let rhs = self.parse_unary()?;
                    e = Expr::Mod {
                        left: Box::new(e),
                        right: Box::new(rhs),
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_unary(&mut self) -> Result<Expr, LangError> {
        if self.cur == Token::Amp {
            self.bump()?;
            let mut_ = if self.cur == Token::Mut {
                self.bump()?;
                true
            } else {
                false
            };
            let inner = self.parse_unary()?;
            return Ok(Expr::Borrow {
                mut_,
                expr: Box::new(inner),
            });
        }

        if self.cur == Token::Minus {
            self.bump()?;
            let inner = self.parse_unary()?;
            return Ok(Expr::Neg {
                expr: Box::new(inner),
            });
        }

        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, LangError> {
        let left = self.parse_primary()?;
        self.parse_postfix_with_left(left)
    }

    fn parse_postfix_with_left(&mut self, mut e: Expr) -> Result<Expr, LangError> {
        loop {
            match &self.cur {
                Token::LParen => {
                    // call: only allowed if callee is Ident
                    let callee = match &e {
                        Expr::Ident(name) => name.clone(),
                        _ => return Err(LangError::Parse("Call target must be identifier".into())),
                    };
                    self.bump()?; // '('
                    let mut args = Vec::new();
                    if self.cur != Token::RParen {
                        args.push(self.parse_expr()?);
                        while self.cur == Token::Comma {
                            self.bump()?;
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect(Token::RParen)?;
                    e = Expr::Call { callee, args };
                }
                Token::LBracket => {
                    self.bump()?; // '['
                    let idx = self.parse_expr()?;
                    self.expect(Token::RBracket)?;
                    e = Expr::Index {
                        base: Box::new(e),
                        index: Box::new(idx),
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr, LangError> {
        match &self.cur {
            Token::Int(n) => {
                let v = *n;
                self.bump()?;
                Ok(Expr::Int(v))
            }
            Token::Str(s) => {
                let v = s.clone();
                self.bump()?;
                Ok(Expr::Str(v))
            }
            Token::True => {
                self.bump()?;
                Ok(Expr::Bool(true))
            }
            Token::False => {
                self.bump()?;
                Ok(Expr::Bool(false))
            }
            Token::Ident(s) => {
                let v = s.clone();
                self.bump()?;
                Ok(Expr::Ident(v))
            }
            Token::LBracket => self.parse_vec_lit(),
            Token::LParen => {
                self.bump()?;
                let e = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(e)
            }
            _ => Err(LangError::Parse(format!(
                "Unexpected token in expr: {:?}",
                self.cur
            ))),
        }
    }

    fn parse_vec_lit(&mut self) -> Result<Expr, LangError> {
        self.expect(Token::LBracket)?;
        let mut items = Vec::new();
        if self.cur != Token::RBracket {
            items.push(self.parse_expr()?);
            while self.cur == Token::Comma {
                self.bump()?;
                items.push(self.parse_expr()?);
            }
        }
        self.expect(Token::RBracket)?;
        Ok(Expr::VecLit(items))
    }
}
