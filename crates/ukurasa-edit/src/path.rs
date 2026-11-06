//! A restricted SVG path model: absolute `M`, `L`, `Q`, `C`, and `Z` commands in a y-down box.

use std::fmt::Write;

use crate::EditError;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

impl Segment {
    fn arity(command: char) -> Option<usize> {
        match command {
            'M' | 'L' => Some(2),
            'Q' => Some(4),
            'C' => Some(6),
            'Z' => Some(0),
            _ => None,
        }
    }

    fn translate(self, dx: f64, dy: f64) -> Self {
        match self {
            Self::Move(x, y) => Self::Move(x + dx, y + dy),
            Self::Line(x, y) => Self::Line(x + dx, y + dy),
            Self::Quad(x1, y1, x, y) => Self::Quad(x1 + dx, y1 + dy, x + dx, y + dy),
            Self::Cubic(x1, y1, x2, y2, x, y) => {
                Self::Cubic(x1 + dx, y1 + dy, x2 + dx, y2 + dy, x + dx, y + dy)
            }
            Self::Close => Self::Close,
        }
    }
}

pub fn parse(d: &str) -> Result<Vec<Segment>, EditError> {
    let invalid = |why: &str| EditError::InvalidPath(why.to_string());
    let mut segments = Vec::new();
    let mut command = None;
    let mut args: Vec<f64> = Vec::new();
    let mut tokens = tokenize(d)?.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            Token::Command(c) => {
                command = Some(c);
                if c == 'Z' {
                    segments.push(Segment::Close);
                }
            }
            Token::Number(n) => {
                let c = command.ok_or_else(|| invalid("path data must start with a command"))?;
                let arity = Segment::arity(c).unwrap_or(0);
                if arity == 0 {
                    return Err(invalid("Z takes no coordinates"));
                }
                args.push(n);
                while args.len() < arity {
                    match tokens.next() {
                        Some(Token::Number(n)) => args.push(n),
                        _ => return Err(invalid("a command is missing coordinates")),
                    }
                }
                segments.push(match (c, args.as_slice()) {
                    ('M', &[x, y]) => Segment::Move(x, y),
                    ('L', &[x, y]) => Segment::Line(x, y),
                    ('Q', &[x1, y1, x, y]) => Segment::Quad(x1, y1, x, y),
                    ('C', &[x1, y1, x2, y2, x, y]) => Segment::Cubic(x1, y1, x2, y2, x, y),
                    _ => unreachable!(),
                });
                args.clear();
                // Extra coordinate pairs after a moveto are linetos, as in SVG.
                if c == 'M' {
                    command = Some('L');
                }
            }
        }
    }
    if !matches!(segments.first(), None | Some(Segment::Move(..))) {
        return Err(invalid("path data must start with M"));
    }
    Ok(segments)
}

enum Token {
    Command(char),
    Number(f64),
}

fn tokenize(d: &str) -> Result<Vec<Token>, EditError> {
    let mut tokens = Vec::new();
    let mut number = String::new();
    let flush = |number: &mut String, tokens: &mut Vec<Token>| -> Result<(), EditError> {
        if !number.is_empty() {
            let n: f64 = number
                .parse()
                .ok()
                .filter(|n: &f64| n.is_finite())
                .ok_or_else(|| EditError::InvalidPath(format!("{number} is not a number")))?;
            tokens.push(Token::Number(n));
            number.clear();
        }
        Ok(())
    };
    for c in d.chars() {
        match c {
            '0'..='9' | '.' | 'e' => number.push(c),
            '-' | '+' => {
                if !number.ends_with('e') {
                    flush(&mut number, &mut tokens)?;
                }
                number.push(c);
            }
            c if c.is_whitespace() || c == ',' => flush(&mut number, &mut tokens)?,
            c if Segment::arity(c).is_some() => {
                flush(&mut number, &mut tokens)?;
                tokens.push(Token::Command(c));
            }
            c => return Err(EditError::InvalidPath(format!("unsupported command {c}"))),
        }
    }
    flush(&mut number, &mut tokens)?;
    Ok(tokens)
}

pub fn translate(segments: &[Segment], dx: f64, dy: f64) -> Vec<Segment> {
    segments.iter().map(|s| s.translate(dx, dy)).collect()
}

pub fn to_svg(segments: &[Segment]) -> String {
    let mut d = String::new();
    for segment in segments {
        let (command, coords): (char, &[f64]) = match segment {
            Segment::Move(x, y) => ('M', &[*x, *y]),
            Segment::Line(x, y) => ('L', &[*x, *y]),
            Segment::Quad(x1, y1, x, y) => ('Q', &[*x1, *y1, *x, *y]),
            Segment::Cubic(x1, y1, x2, y2, x, y) => ('C', &[*x1, *y1, *x2, *y2, *x, *y]),
            Segment::Close => ('Z', &[]),
        };
        if !d.is_empty() {
            d.push(' ');
        }
        d.push(command);
        for n in coords {
            write!(d, " {}", number(*n)).unwrap();
        }
    }
    d
}

/// Appends PDF path construction operators, converting quadratic curves to cubic ones.
pub fn write_pdf(segments: &[Segment], out: &mut String) {
    let mut current = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    for segment in segments {
        match *segment {
            Segment::Move(x, y) => {
                writeln!(out, "{} {} m", number(x), number(y)).unwrap();
                current = (x, y);
                start = current;
            }
            Segment::Line(x, y) => {
                writeln!(out, "{} {} l", number(x), number(y)).unwrap();
                current = (x, y);
            }
            Segment::Quad(qx, qy, x, y) => {
                let (x0, y0) = current;
                let c1 = (x0 + 2.0 / 3.0 * (qx - x0), y0 + 2.0 / 3.0 * (qy - y0));
                let c2 = (x + 2.0 / 3.0 * (qx - x), y + 2.0 / 3.0 * (qy - y));
                write_cubic(out, c1, c2, (x, y));
                current = (x, y);
            }
            Segment::Cubic(x1, y1, x2, y2, x, y) => {
                write_cubic(out, (x1, y1), (x2, y2), (x, y));
                current = (x, y);
            }
            Segment::Close => {
                out.push_str("h\n");
                current = start;
            }
        }
    }
}

fn write_cubic(out: &mut String, c1: (f64, f64), c2: (f64, f64), end: (f64, f64)) {
    writeln!(
        out,
        "{} {} {} {} {} {} c",
        number(c1.0),
        number(c1.1),
        number(c2.0),
        number(c2.1),
        number(end.0),
        number(end.1)
    )
    .unwrap();
}

/// Formats a coordinate with at most three decimals and no exponent.
pub fn number(n: f64) -> String {
    let s = format!("{:.3}", n);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    match s {
        "-0" | "" => "0".to_string(),
        s => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_round_trips() {
        let d = "M 1 2 L 3.5 -4 Q 5 6 7 8 C 1 2 3 4 5 6 Z";
        let segments = parse(d).unwrap();
        assert_eq!(
            segments,
            vec![
                Segment::Move(1.0, 2.0),
                Segment::Line(3.5, -4.0),
                Segment::Quad(5.0, 6.0, 7.0, 8.0),
                Segment::Cubic(1.0, 2.0, 3.0, 4.0, 5.0, 6.0),
                Segment::Close,
            ]
        );
        assert_eq!(to_svg(&segments), d);
    }

    #[test]
    fn accepts_compact_syntax_and_implicit_lines() {
        let segments = parse("M1,2 3,4-5-6").unwrap();
        assert_eq!(
            segments,
            vec![
                Segment::Move(1.0, 2.0),
                Segment::Line(3.0, 4.0),
                Segment::Line(-5.0, -6.0),
            ]
        );
    }

    #[test]
    fn rejects_unsupported_input() {
        assert!(parse("m 1 2").is_err());
        assert!(parse("M 1").is_err());
        assert!(parse("L 1 2").is_err());
        assert!(parse("M 1 2 A 1 1 0 0 0 3 4").is_err());
        assert!(parse("M NaN 2").is_err());
    }

    #[test]
    fn converts_quadratics_to_cubics() {
        let mut out = String::new();
        write_pdf(&parse("M 0 0 Q 3 3 6 0").unwrap(), &mut out);
        assert_eq!(out, "0 0 m\n2 2 4 2 6 0 c\n");
    }

    #[test]
    fn formats_numbers_compactly() {
        assert_eq!(number(1.0), "1");
        assert_eq!(number(-0.0001), "0");
        assert_eq!(number(2.5), "2.5");
        assert_eq!(number(1.23456), "1.235");
    }
}
