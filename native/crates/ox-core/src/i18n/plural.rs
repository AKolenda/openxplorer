// SPDX-License-Identifier: AGPL-3.0-only
//! A catalogue's plural rule: the C expression of its `Plural-Forms`
//! header (`nplurals=3; plural=(n==1 ? 0 : …);`), which picks the form a
//! count takes. The grammar is GNU gettext's: `n`, numbers, `!`, `* / %`,
//! `+ -`, `< <= > >=`, `== !=`, `&&`, `||`, `?:` and parentheses, with
//! unsigned arithmetic.

/// A parsed plural expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expression {
    /// The count.
    Count,
    /// A number.
    Number(u64),
    /// `!operand`.
    Not(Box<Expression>),
    /// `left operator right`.
    Binary(Box<Expression>, Operator, Box<Expression>),
    /// `condition ? then : otherwise`.
    Choice(Box<Expression>, Box<Expression>, Box<Expression>),
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Multiply,
    Divide,
    Remainder,
    Add,
    Subtract,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
    And,
    Or,
}

impl Operator {
    /// The operators of each precedence level, loosest first, and their
    /// spelling. Longer spellings come first so `<=` is not read as `<`.
    const LEVELS: [&'static [(&'static str, Operator)]; 6] = [
        &[("||", Operator::Or)],
        &[("&&", Operator::And)],
        &[("==", Operator::Equal), ("!=", Operator::NotEqual)],
        &[
            ("<=", Operator::LessOrEqual),
            (">=", Operator::GreaterOrEqual),
            ("<", Operator::Less),
            (">", Operator::Greater),
        ],
        &[("+", Operator::Add), ("-", Operator::Subtract)],
        &[
            ("*", Operator::Multiply),
            ("/", Operator::Divide),
            ("%", Operator::Remainder),
        ],
    ];

    /// `left` with `right` by this operator; division by zero gives 0.
    fn apply(self, left: u64, right: u64) -> u64 {
        match self {
            Operator::Multiply => left.wrapping_mul(right),
            Operator::Divide => left.checked_div(right).unwrap_or(0),
            Operator::Remainder => left.checked_rem(right).unwrap_or(0),
            Operator::Add => left.wrapping_add(right),
            Operator::Subtract => left.wrapping_sub(right),
            Operator::Less => u64::from(left < right),
            Operator::LessOrEqual => u64::from(left <= right),
            Operator::Greater => u64::from(left > right),
            Operator::GreaterOrEqual => u64::from(left >= right),
            Operator::Equal => u64::from(left == right),
            Operator::NotEqual => u64::from(left != right),
            Operator::And => u64::from(left != 0 && right != 0),
            Operator::Or => u64::from(left != 0 || right != 0),
        }
    }
}

impl Expression {
    /// The value for `count`.
    fn evaluate(&self, count: u64) -> u64 {
        match self {
            Expression::Count => count,
            Expression::Number(number) => *number,
            Expression::Not(operand) => u64::from(operand.evaluate(count) == 0),
            Expression::Binary(left, operator, right) => {
                operator.apply(left.evaluate(count), right.evaluate(count))
            }
            Expression::Choice(condition, then, otherwise) => {
                if condition.evaluate(count) == 0 {
                    otherwise.evaluate(count)
                } else {
                    then.evaluate(count)
                }
            }
        }
    }
}

/// A recursive-descent reader of an expression's text.
struct Parser<'a> {
    rest: &'a str,
}

impl Parser<'_> {
    /// Skips white space, then takes `token` if the text starts with it.
    fn eat(&mut self, token: &str) -> bool {
        self.rest = self.rest.trim_start();
        match self.rest.strip_prefix(token) {
            Some(rest) => {
                self.rest = rest;
                true
            }
            None => false,
        }
    }

    /// `condition ? then : otherwise`, or a binary expression.
    fn choice(&mut self) -> Option<Expression> {
        let condition = self.binary(0)?;
        if !self.eat("?") {
            return Some(condition);
        }
        let then = self.choice()?;
        if !self.eat(":") {
            return None;
        }
        let otherwise = self.choice()?;
        Some(Expression::Choice(
            Box::new(condition),
            Box::new(then),
            Box::new(otherwise),
        ))
    }

    /// The operators of precedence `level` and tighter, left to right.
    fn binary(&mut self, level: usize) -> Option<Expression> {
        let Some(operators) = Operator::LEVELS.get(level) else {
            return self.unary();
        };
        let mut left = self.binary(level + 1)?;
        while let Some(operator) = operators
            .iter()
            .find_map(|(spelling, operator)| self.eat(spelling).then_some(*operator))
        {
            let right = self.binary(level + 1)?;
            left = Expression::Binary(Box::new(left), operator, Box::new(right));
        }
        Some(left)
    }

    /// `!operand`, `(expression)`, `n` or a number.
    fn unary(&mut self) -> Option<Expression> {
        if self.eat("!") {
            return Some(Expression::Not(Box::new(self.unary()?)));
        }
        if self.eat("(") {
            let inner = self.choice()?;
            return self.eat(")").then_some(inner);
        }
        if self.eat("n") {
            return Some(Expression::Count);
        }
        let digits = self.rest.len() - self.rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let number = self.rest[..digits].parse().ok()?;
        self.rest = &self.rest[digits..];
        Some(Expression::Number(number))
    }
}

/// The plural rule of a catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plural {
    /// How many forms the language has.
    forms: u64,
    /// Which form a count takes.
    expression: Expression,
}

impl Default for Plural {
    /// English's rule, which catalogues without a header follow.
    fn default() -> Self {
        Self {
            forms: 2,
            expression: Expression::Binary(
                Box::new(Expression::Count),
                Operator::NotEqual,
                Box::new(Expression::Number(1)),
            ),
        }
    }
}

impl Plural {
    /// The rule in a catalogue `header`'s `Plural-Forms` line; `None` when
    /// it has none or the line does not parse.
    pub(super) fn from_header(header: &str) -> Option<Self> {
        let line = header
            .lines()
            .find_map(|line| line.strip_prefix("Plural-Forms:"))?;
        let mut forms = None;
        let mut expression = None;
        for part in line.split(';') {
            let Some((name, value)) = part.split_once('=') else {
                continue;
            };
            match name.trim() {
                "nplurals" => forms = value.trim().parse().ok(),
                "plural" => {
                    let mut parser = Parser { rest: value };
                    expression = parser.choice().filter(|_| parser.rest.trim().is_empty());
                }
                _ => {}
            }
        }
        Some(Self {
            forms: forms.filter(|forms| *forms > 0)?,
            expression: expression?,
        })
    }

    /// The form `count` takes, never past the last one.
    pub(super) fn form(&self, count: u64) -> usize {
        let form = self.expression.evaluate(count).min(self.forms - 1);
        usize::try_from(form).unwrap_or_default()
    }
}
