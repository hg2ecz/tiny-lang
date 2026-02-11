use crate::error::LangError;
use crate::token::Token;

pub struct Lexer<'a> {
    input: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(s: &'a str) -> Self {
        Self {
            input: s.as_bytes(),
            i: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        Some(c)
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                self.i += 1;
            }
            // line comment: //
            if self.peek() == Some(b'/') && self.input.get(self.i + 1).copied() == Some(b'/') {
                while let Some(c) = self.peek() {
                    self.i += 1;
                    if c == b'\n' {
                        break;
                    }
                }
                continue;
            }
            break;
        }
    }

    pub fn next_token(&mut self) -> Result<Token, LangError> {
        self.skip_ws_and_comments();
        let c = match self.peek() {
            Some(c) => c,
            None => return Ok(Token::Eof),
        };

        match c {
            b'(' => {
                self.bump();
                Ok(Token::LParen)
            }
            b')' => {
                self.bump();
                Ok(Token::RParen)
            }
            b'[' => {
                self.bump();
                Ok(Token::LBracket)
            }
            b']' => {
                self.bump();
                Ok(Token::RBracket)
            }
            b'{' => {
                self.bump();
                Ok(Token::LBrace)
            }
            b'}' => {
                self.bump();
                Ok(Token::RBrace)
            }
            b',' => {
                self.bump();
                Ok(Token::Comma)
            }
            b';' => {
                self.bump();
                Ok(Token::Semi)
            }
            b'&' => {
                self.bump();
                Ok(Token::Amp)
            }
            b'+' => {
                self.bump();
                Ok(Token::Plus)
            }
            b'-' => {
                self.bump();
                Ok(Token::Minus)
            }
            b'*' => {
                self.bump();
                Ok(Token::Star)
            }
            b'/' => {
                self.bump();
                Ok(Token::Slash)
            }
            b'%' => {
                self.bump();
                Ok(Token::Percent)
            }

            b'=' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(Token::EqEq)
                } else {
                    Ok(Token::Eq)
                }
            }
            b'!' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(Token::NotEq)
                } else {
                    Err(LangError::Lex("Unexpected '!': did you mean '!='?".into()))
                }
            }
            b'<' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(Token::Le)
                } else {
                    Ok(Token::Lt)
                }
            }
            b'>' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(Token::Ge)
                } else {
                    Ok(Token::Gt)
                }
            }

            b'"' => self.lex_string(),
            b'0'..=b'9' => self.lex_number(),
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_ident_or_kw(),
            _ => Err(LangError::Lex(format!("Unexpected char: {}", c as char))),
        }
    }

    fn lex_number(&mut self) -> Result<Token, LangError> {
        let start = self.i;

        // integer part
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }

        // fractional part
        if self.peek() == Some(b'.') {
            self.i += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }

        // exponent part
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }

        let s = std::str::from_utf8(&self.input[start..self.i])
            .map_err(|e| LangError::Lex(e.to_string()))?;
        let n = s
            .parse::<f64>()
            .map_err(|e| LangError::Lex(e.to_string()))?;
        Ok(Token::Num(n))
    }

    fn lex_ident_or_kw(&mut self) -> Result<Token, LangError> {
        let start = self.i;
        while matches!(
            self.peek(),
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'0'..=b'9')
        ) {
            self.i += 1;
        }
        let s = std::str::from_utf8(&self.input[start..self.i])
            .map_err(|e| LangError::Lex(e.to_string()))?;

        Ok(match s {
            "let" => Token::Let,
            "print" => Token::Print,
            "fn" => Token::Fn,
            "for" => Token::For,
            "in" => Token::In,
            "return" => Token::Return,
            "mut" => Token::Mut,
            "if" => Token::If,
            "else" => Token::Else,
            "true" => Token::True,
            "false" => Token::False,
            _ => Token::Ident(s.to_string()),
        })
    }

    fn lex_string(&mut self) -> Result<Token, LangError> {
        self.bump(); // opening "
        let mut out = String::new();
        while let Some(c) = self.bump() {
            match c {
                b'"' => return Ok(Token::Str(out)),
                b'\\' => {
                    let nxt = self
                        .bump()
                        .ok_or_else(|| LangError::Lex("Unclosed escape".into()))?;
                    let ch = match nxt {
                        b'n' => '\n',
                        b't' => '\t',
                        b'"' => '"',
                        b'\\' => '\\',
                        other => {
                            return Err(LangError::Lex(format!(
                                "Unknown escape: \\{}",
                                other as char
                            )));
                        }
                    };
                    out.push(ch);
                }
                other => out.push(other as char),
            }
        }
        Err(LangError::Lex("Unclosed string literal".into()))
    }
}
