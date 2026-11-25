//! TMTR: build expression text, then evaluate it as signed integers. See
//! tools/tmtr-evidence.md for the native dispatch and checked fixtures.
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::Write;

type Variables = BTreeMap<u32, i32>;

#[derive(Debug, Default, Deserialize)]
pub struct ConditionInputs {
    /// Keys are packed selectors, not class/job IDs or EXD row numbers.
    #[serde(default)]
    pub variables: Variables,
    /// Optional resolved provider values by logical TMB path then actor ID.
    #[serde(default)]
    pub actors: BTreeMap<String, BTreeMap<u16, Variables>>,
}

impl ConditionInputs {
    pub(super) fn evaluate(&self, path: &str, actor: u16, script: &[[u32; 3]]) -> Result<bool> {
        let local = self.actors.get(path).and_then(|actors| actors.get(&actor));
        let mut text = String::new();
        ensure!(script.len() <= 1024, "TMTR condition token budget");
        for &[op, value, bits] in script {
            match op {
                1 | 2 => {
                    let n = (value as i32).max(0) as usize;
                    ensure!(
                        n < 384 - text.len(),
                        "TMTR expression exceeds native buffer"
                    );
                    text.extend(std::iter::repeat_n(if op == 1 { '(' } else { ')' }, n));
                }
                17 => write!(text, "{}", value as i32)?,
                18 => {
                    let f = f32::from_bits(bits);
                    ensure!(f.is_finite(), "nonfinite TMTR float");
                    write!(text, "{f:.6}")?;
                }
                19 => {
                    let x = local
                        .and_then(|vars| vars.get(&value))
                        .or_else(|| self.variables.get(&value))
                        .with_context(|| {
                            format!("missing TMTR variable {value:#010x} for actor {actor}")
                        })?;
                    write!(text, "{x}")?;
                }
                3..=16 => text.push_str(
                    [
                        "+", "-", "*", "/", "%", ">", ">=", "<", "<=", "~=", "==", "and", "or",
                        "not",
                    ][(op - 3) as usize],
                ),
                20 => text.push_str("true"),
                21 => text.push_str("false"),
                _ => bail!("unsupported TMTR operation {op}"),
            }
            ensure!(text.len() < 384, "TMTR expression exceeds native buffer");
        }
        let mut parser = Parser(&text);
        let result = parser.expression(1, 0)?;
        ensure!(parser.0.is_empty(), "unused TMTR expression suffix");
        Ok(result != 0)
    }
}

#[derive(Clone, Copy)]
struct Parser<'a>(&'a str);

impl<'a> Parser<'a> {
    // The native token reader advances by the *full* token length but copies
    // at most nine characters into the numeric conversion buffer.
    fn token(&mut self) -> &'a str {
        let bytes = self.0.as_bytes();
        let Some(&first) = bytes.first() else {
            return "";
        };
        let n = if first.is_ascii_digit() {
            bytes
                .iter()
                .take_while(|b| b.is_ascii_digit() || **b == b'.')
                .count()
        } else if first.is_ascii_lowercase() {
            bytes.iter().take_while(|b| b.is_ascii_lowercase()).count()
        } else if matches!(first, b'<' | b'>' | b'~' | b'=') && bytes.get(1) == Some(&b'=') {
            2
        } else {
            1
        };
        let (s, rest) = self.0.split_at(n);
        self.0 = rest;
        s
    }

    fn expression(&mut self, min: u8, depth: u16) -> Result<i32> {
        ensure!(depth < 64, "TMTR nesting budget");
        let mut x = match self.token() {
            "(" => {
                let x = self.expression(1, depth + 1)?;
                ensure!(self.token() == ")", "unclosed TMTR group");
                x
            }
            "not" => i32::from(self.expression(7, depth + 1)? == 0),
            "-" => self.expression(7, depth + 1)?.wrapping_neg(),
            "+" => self.expression(7, depth + 1)?,
            "true" => 1,
            "false" => 0,
            s if s.starts_with(|c: char| c.is_ascii_digit()) => {
                s[..s.len().min(9)].split('.').next().unwrap().parse()?
            }
            s => bail!("invalid TMTR operand {s:?}"),
        };
        loop {
            let mut next = *self;
            let op = next.token();
            let precedence = match op {
                "or" => 1,
                "and" => 2,
                "==" | "~=" => 3,
                "<" | ">" | "<=" | ">=" => 4,
                "+" | "-" => 5,
                "*" | "/" | "%" => 6,
                _ => 0,
            };
            if precedence < min {
                break;
            }
            *self = next;
            let y = self.expression(precedence + 1, depth + 1)?;
            x = match op {
                "or" => x | y,
                "and" => x & y,
                "==" => i32::from(x == y),
                "~=" => i32::from(x != y),
                "<" => i32::from(x < y),
                ">" => i32::from(x > y),
                "<=" => i32::from(x <= y),
                ">=" => i32::from(x >= y),
                "+" => x.wrapping_add(y),
                "-" => x.wrapping_sub(y),
                "*" => x.wrapping_mul(y),
                "/" => x.checked_div(y).context("invalid TMTR division")?,
                "%" => x.checked_rem(y).context("invalid TMTR remainder")?,
                _ => unreachable!(),
            };
        }
        Ok(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_integer_edges() {
        let env = ConditionInputs::default();
        let val = |v| [17, v, 0];
        let op = |v| [v, 0, 0];
        let eval = |s: &[[u32; 3]]| env.evaluate("t", 0, s);
        assert!(!eval(&[val(2), op(14), val(1)]).unwrap()); // bitwise AND, not Lua truth
        assert!(eval(&[val(1), op(3), val(2), op(5), val(3), op(13), val(7)]).unwrap());
        assert!(eval(&[[18, 0, (-3.75f32).to_bits()], op(13), val((-3i32) as u32)]).unwrap());
        assert!(eval(&[val(1234567890), op(13), val(123456789)]).unwrap());
        assert!(eval(&[[1, 0x7fff_ffff, 0]]).is_err());
        assert!(eval(&[val(1), op(6), val(0)]).is_err());
        assert!(eval(&[op(43)]).is_err());
        assert!(
            eval(&[[19, 0x10000029, 0]])
                .unwrap_err()
                .to_string()
                .contains("0x10000029")
        );
    }

    #[test]
    #[ignore = "set TMTR_NATIVE_ROWS to verify-tmtr-native.py JSONL output"]
    fn native_condition_rows() -> Result<()> {
        #[derive(Deserialize)]
        struct Row {
            script: Vec<[u32; 3]>,
            variables: Variables,
            admitted: bool,
            expression: String,
        }
        let rows = std::fs::read_to_string(std::env::var("TMTR_NATIVE_ROWS")?)?;
        ensure!(!rows.is_empty(), "empty oracle");
        for line in rows.lines() {
            let row: Row = serde_json::from_str(line)?;
            let env = ConditionInputs {
                variables: row.variables,
                ..Default::default()
            };
            assert_eq!(
                env.evaluate("track", 0, &row.script)?,
                row.admitted,
                "{}",
                row.expression
            );
        }
        Ok(())
    }
}
