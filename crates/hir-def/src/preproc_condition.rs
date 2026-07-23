//! Parsed `#Если …` preprocessor conditions and their evaluation against a
//! single execution environment.
//!
//! The platform's conditional-compilation symbols select execution
//! environments (`Сервер`, `ТонкийКлиент`, …), so an availability check can
//! narrow the body's environment set per branch instead of skipping whole
//! `#Если` blocks. Symbols that do not map onto [`EnvFlags`] environments
//! evaluate to "unknown": mobile-application runtimes are `false` for every
//! modelled environment (they are genuinely different runtimes), while
//! anything unrecognized keeps the tri-state `None` so consumers can stay
//! conservative.
//!
//! [`EnvFlags`]: crate::execution_env::EnvFlags

use crate::execution_env::EnvFlags;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreprocCondition {
    Symbol(PreprocSymbol),
    Not(Box<PreprocCondition>),
    And(Box<PreprocCondition>, Box<PreprocCondition>),
    Or(Box<PreprocCondition>, Box<PreprocCondition>),
    /// Unparseable condition — evaluates to unknown everywhere.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreprocSymbol {
    /// `Клиент` / `НаКлиенте` — any interactive client.
    Client,
    /// `Сервер` / `НаСервере`.
    Server,
    ThinClient,
    WebClient,
    MobileClient,
    ThickClientOrdinaryApplication,
    ThickClientManagedApplication,
    ExternalConnection,
    /// `МобильноеПриложениеКлиент` / `МобильноеПриложениеСервер` /
    /// `МобильныйАвтономныйСервер` — separate runtimes, none of the
    /// modelled environments.
    MobileAppRuntime,
    /// Anything else (OS symbols, future platform symbols) — orthogonal to
    /// or unknown for the environment model.
    Unrecognized,
}

impl PreprocSymbol {
    fn from_ident(ident: &str) -> PreprocSymbol {
        use PreprocSymbol::*;
        if eq(ident, "Клиент", "Client") || eq(ident, "НаКлиенте", "AtClient") {
            Client
        } else if eq(ident, "Сервер", "Server") || eq(ident, "НаСервере", "AtServer")
        {
            Server
        } else if eq(ident, "ТонкийКлиент", "ThinClient") {
            ThinClient
        } else if eq(ident, "ВебКлиент", "WebClient") {
            WebClient
        } else if eq(ident, "МобильныйКлиент", "MobileClient") {
            MobileClient
        } else if eq(ident, "ТолстыйКлиентОбычноеПриложение", "ThickClientOrdinaryApplication")
        {
            ThickClientOrdinaryApplication
        } else if eq(ident, "ТолстыйКлиентУправляемоеПриложение", "ThickClientManagedApplication")
        {
            ThickClientManagedApplication
        } else if eq(ident, "ВнешнееСоединение", "ExternalConnection") {
            ExternalConnection
        } else if eq(ident, "МобильноеПриложениеКлиент", "MobileAppClient")
            || eq(ident, "МобильноеПриложениеСервер", "MobileAppServer")
            || eq(ident, "МобильныйАвтономныйСервер", "MobileStandaloneServer")
        {
            MobileAppRuntime
        } else {
            Unrecognized
        }
    }

    /// Whether the symbol holds in environment `env` (a single flag).
    /// `None` — cannot be decided from the environment alone.
    fn eval(self, env: EnvFlags) -> Option<bool> {
        use PreprocSymbol::*;
        match self {
            Client => {
                Some(env.intersects(EnvFlags::MANAGED_CLIENTS | EnvFlags::THICK_CLIENT_ORDINARY))
            }
            // In the file-mode thick client and in the external connection the
            // platform compiles "server" code locally, so `Сервер` there
            // depends on the infobase mode — undecidable from the
            // environment alone.
            Server => match env {
                EnvFlags::SERVER => Some(true),
                EnvFlags::THICK_CLIENT_MANAGED
                | EnvFlags::THICK_CLIENT_ORDINARY
                | EnvFlags::EXTERNAL_CONNECTION => None,
                _ => Some(false),
            },
            ThinClient => Some(env == EnvFlags::THIN_CLIENT),
            WebClient => Some(env == EnvFlags::WEB_CLIENT),
            MobileClient => Some(env == EnvFlags::MOBILE_CLIENT),
            ThickClientOrdinaryApplication => Some(env == EnvFlags::THICK_CLIENT_ORDINARY),
            ThickClientManagedApplication => Some(env == EnvFlags::THICK_CLIENT_MANAGED),
            ExternalConnection => Some(env == EnvFlags::EXTERNAL_CONNECTION),
            MobileAppRuntime => Some(false),
            Unrecognized => None,
        }
    }
}

fn eq(ident: &str, ru: &str, en: &str) -> bool {
    stdx::case::eq_ignore_case(ident, ru) || stdx::case::eq_ignore_case(ident, en)
}

impl PreprocCondition {
    /// Parse the text between `#Если`/`#ИначеЕсли` and `Тогда`. Grammar:
    /// `или_выражение`, with `НЕ` binding tighter than `И`, and `И` tighter
    /// than `ИЛИ`; parentheses allowed. A malformed condition yields
    /// [`PreprocCondition::Unknown`].
    pub fn parse(text: &str) -> PreprocCondition {
        let tokens = tokenize(text);
        // Real conditions are a handful of tokens; a pathological chain would
        // otherwise build a tree whose recursive evaluation (and drop) can
        // overflow the stack.
        if tokens.len() > MAX_CONDITION_TOKENS {
            return PreprocCondition::Unknown;
        }
        let mut parser = Parser { tokens: &tokens, pos: 0 };
        match parser.parse_or() {
            Some(cond) if parser.pos == parser.tokens.len() => cond,
            _ => PreprocCondition::Unknown,
        }
    }

    /// Parse a whole `#Если …`/`#ИначеЕсли …` header line. The condition is
    /// whatever sits between the directive keyword and the trailing
    /// `Тогда`/`Then`; a header that does not match this shape (multi-line,
    /// error recovery, trailing tokens) yields [`PreprocCondition::Unknown`].
    /// Parsing the raw header — rather than the parser's `PRE_EXPR` node —
    /// keeps a partially-recovered condition from masquerading as a valid
    /// one.
    pub fn parse_directive_header(header: &str) -> PreprocCondition {
        let Some(condition_text) = extract_condition_text(header) else {
            return PreprocCondition::Unknown;
        };
        PreprocCondition::parse(condition_text)
    }

    /// Tri-state truth of the condition in environment `env` (single flag):
    /// `Some(true)` — the branch is compiled in `env`; `Some(false)` — it is
    /// not; `None` — undecidable (unrecognized symbol involved).
    pub fn eval(&self, env: EnvFlags) -> Option<bool> {
        match self {
            PreprocCondition::Symbol(sym) => sym.eval(env),
            PreprocCondition::Not(inner) => inner.eval(env).map(|v| !v),
            PreprocCondition::And(lhs, rhs) => match (lhs.eval(env), rhs.eval(env)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            PreprocCondition::Or(lhs, rhs) => match (lhs.eval(env), rhs.eval(env)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            PreprocCondition::Unknown => None,
        }
    }

    /// Environment mask of one `#Если`/`#ИначеЕсли` branch in a chain.
    ///
    /// `remaining` holds the environments not yet claimed by earlier
    /// branches; the returned mask is the subset where this branch is
    /// definitely compiled. Environments the condition cannot decide
    /// (`None`) are removed from `remaining` without entering any branch —
    /// availability checks conservatively skip them everywhere in the
    /// chain, including `#Иначе` (which receives whatever is left in
    /// `remaining`).
    pub fn narrow_branch(&self, remaining: &mut EnvFlags) -> EnvFlags {
        let mut mask = EnvFlags::EMPTY;
        for env in remaining.iter() {
            match self.eval(env) {
                Some(true) => {
                    mask = mask | env;
                    *remaining = remaining.without(env);
                }
                Some(false) => {}
                None => {
                    *remaining = remaining.without(env);
                }
            }
        }
        mask
    }

    /// Rough heap bytes owned by this node's boxed children. The node
    /// itself is inline in its owner (a `PreprocIfStmt` field, a slice
    /// element, or a parent's `Box`) and is counted there.
    pub fn memory_usage(&self) -> usize {
        fn boxed(child: &PreprocCondition) -> usize {
            std::mem::size_of::<PreprocCondition>() + child.memory_usage()
        }
        match self {
            PreprocCondition::Not(inner) => boxed(inner),
            PreprocCondition::And(lhs, rhs) | PreprocCondition::Or(lhs, rhs) => {
                boxed(lhs) + boxed(rhs)
            }
            PreprocCondition::Symbol(_) | PreprocCondition::Unknown => 0,
        }
    }
}

const MAX_CONDITION_TOKENS: usize = 64;

/// Slice the condition out of a directive header: `#Если X Тогда` → `X`.
pub(crate) fn extract_condition_text(header: &str) -> Option<&str> {
    // Real configs routinely append a line comment after the directive
    // (`#Если … Тогда // +бит добавлено …`); drop it so the `Тогда`/`Then`
    // terminator lands on the actual directive text. Preprocessor
    // conditions never contain string literals, so the first `//` always
    // begins the comment.
    let header = match header.find("//") {
        Some(at) => &header[..at],
        None => header,
    };
    let rest = header.trim_start();
    let rest = rest.strip_prefix('#')?.trim_start();
    let keyword_len = ["ИначеЕсли", "ElsIf", "Если", "If"]
        .iter()
        .find(|kw| starts_with_ignore_case(rest, kw))?
        .len();
    let rest = rest[keyword_len..].trim();
    let then_at = ["Тогда", "Then"].iter().find_map(|kw| {
        rest.len()
            .checked_sub(kw.len())
            // `checked_sub` yields a byte offset; a non-boundary index (a
            // trailing multibyte char that is shorter than the keyword)
            // would panic the slice, so reject it before comparing.
            .filter(|&at| rest.is_char_boundary(at) && stdx::case::eq_ignore_case(&rest[at..], kw))
    })?;
    Some(rest[..then_at].trim_end())
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len()).is_some_and(|head| stdx::case::eq_ignore_case(head, prefix))
}

#[derive(Debug, PartialEq)]
enum Token<'a> {
    Ident(&'a str),
    Not,
    And,
    Or,
    LParen,
    RParen,
    Error,
}

fn tokenize(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '(' {
            chars.next();
            tokens.push(Token::LParen);
        } else if c == ')' {
            chars.next();
            tokens.push(Token::RParen);
        } else if c.is_alphabetic() || c == '_' {
            let mut end = start + c.len_utf8();
            chars.next();
            while let Some(&(idx, c)) = chars.peek() {
                if c.is_alphanumeric() || c == '_' {
                    end = idx + c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let word = &text[start..end];
            if eq(word, "НЕ", "Not") {
                tokens.push(Token::Not);
            } else if eq(word, "И", "And") {
                tokens.push(Token::And);
            } else if eq(word, "ИЛИ", "Or") {
                tokens.push(Token::Or);
            } else {
                tokens.push(Token::Ident(word));
            }
        } else {
            chars.next();
            tokens.push(Token::Error);
        }
    }
    tokens
}

struct Parser<'a, 't> {
    tokens: &'a [Token<'t>],
    pos: usize,
}

impl Parser<'_, '_> {
    fn parse_or(&mut self) -> Option<PreprocCondition> {
        let mut lhs = self.parse_and()?;
        while matches!(self.tokens.get(self.pos), Some(Token::Or)) {
            self.pos += 1;
            let rhs = self.parse_and()?;
            lhs = PreprocCondition::Or(Box::new(lhs), Box::new(rhs));
        }
        Some(lhs)
    }

    fn parse_and(&mut self) -> Option<PreprocCondition> {
        let mut lhs = self.parse_unary()?;
        while matches!(self.tokens.get(self.pos), Some(Token::And)) {
            self.pos += 1;
            let rhs = self.parse_unary()?;
            lhs = PreprocCondition::And(Box::new(lhs), Box::new(rhs));
        }
        Some(lhs)
    }

    fn parse_unary(&mut self) -> Option<PreprocCondition> {
        match self.tokens.get(self.pos)? {
            Token::Not => {
                self.pos += 1;
                Some(PreprocCondition::Not(Box::new(self.parse_unary()?)))
            }
            Token::LParen => {
                self.pos += 1;
                let inner = self.parse_or()?;
                if !matches!(self.tokens.get(self.pos), Some(Token::RParen)) {
                    return None;
                }
                self.pos += 1;
                Some(inner)
            }
            Token::Ident(word) => {
                let sym = PreprocSymbol::from_ident(word);
                self.pos += 1;
                Some(PreprocCondition::Symbol(sym))
            }
            Token::And | Token::Or | Token::RParen | Token::Error => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(cond: &str, env: EnvFlags) -> Option<bool> {
        PreprocCondition::parse(cond).eval(env)
    }

    #[test]
    fn single_symbols() {
        assert_eq!(t("Сервер", EnvFlags::SERVER), Some(true));
        assert_eq!(t("Сервер", EnvFlags::THIN_CLIENT), Some(false));
        assert_eq!(t("Server", EnvFlags::SERVER), Some(true));
        assert_eq!(t("ВебКлиент", EnvFlags::WEB_CLIENT), Some(true));
        assert_eq!(t("вебклиент", EnvFlags::WEB_CLIENT), Some(true));
        assert_eq!(t("Клиент", EnvFlags::THIN_CLIENT), Some(true));
        assert_eq!(t("Клиент", EnvFlags::THICK_CLIENT_ORDINARY), Some(true));
        assert_eq!(t("Клиент", EnvFlags::SERVER), Some(false));
        assert_eq!(t("НаСервере", EnvFlags::SERVER), Some(true));
        assert_eq!(t("ВнешнееСоединение", EnvFlags::EXTERNAL_CONNECTION), Some(true));
        assert_eq!(
            t("ТолстыйКлиентУправляемоеПриложение", EnvFlags::THICK_CLIENT_MANAGED),
            Some(true)
        );
    }

    #[test]
    fn operators_and_precedence() {
        assert_eq!(t("НЕ ВебКлиент", EnvFlags::THIN_CLIENT), Some(true));
        assert_eq!(t("НЕ ВебКлиент", EnvFlags::WEB_CLIENT), Some(false));
        // И binds tighter than ИЛИ: Сервер ИЛИ (ТонкийКлиент И НЕ ВебКлиент)
        let cond = "Сервер ИЛИ ТонкийКлиент И НЕ ВебКлиент";
        assert_eq!(t(cond, EnvFlags::SERVER), Some(true));
        assert_eq!(t(cond, EnvFlags::THIN_CLIENT), Some(true));
        assert_eq!(t(cond, EnvFlags::WEB_CLIENT), Some(false));
        // Parens override.
        let cond = "(Сервер ИЛИ ТонкийКлиент) И НЕ Сервер";
        assert_eq!(t(cond, EnvFlags::SERVER), Some(false));
        assert_eq!(t(cond, EnvFlags::THIN_CLIENT), Some(true));
        assert_eq!(t("Not WebClient Or Server", EnvFlags::SERVER), Some(true));
    }

    #[test]
    fn unrecognized_symbols_are_tri_state() {
        assert_eq!(t("Линукс", EnvFlags::THIN_CLIENT), None);
        // Absorbing operands still decide.
        assert_eq!(t("Линукс И НЕ ТонкийКлиент", EnvFlags::THIN_CLIENT), Some(false));
        assert_eq!(t("Линукс ИЛИ ТонкийКлиент", EnvFlags::THIN_CLIENT), Some(true));
        assert_eq!(t("Линукс И Сервер", EnvFlags::SERVER), None);
        assert_eq!(t("НЕ Линукс", EnvFlags::SERVER), None);
    }

    #[test]
    fn mobile_app_runtimes_are_false_for_modelled_envs() {
        assert_eq!(t("МобильноеПриложениеКлиент", EnvFlags::THIN_CLIENT), Some(false));
        assert_eq!(t("МобильноеПриложениеСервер", EnvFlags::SERVER), Some(false));
        assert_eq!(t("НЕ МобильноеПриложениеКлиент", EnvFlags::WEB_CLIENT), Some(true));
    }

    #[test]
    fn narrow_branch_chain_partitions_environments() {
        let clients = EnvFlags::THIN_CLIENT | EnvFlags::WEB_CLIENT | EnvFlags::THICK_CLIENT_MANAGED;
        let mut remaining = clients | EnvFlags::SERVER;

        let then_mask = PreprocCondition::parse("Сервер").narrow_branch(&mut remaining);
        assert_eq!(then_mask, EnvFlags::SERVER);
        // The thick client can compile "server" code in file mode, so it is
        // undecidable under `Сервер` and leaves the chain entirely.
        assert_eq!(remaining, EnvFlags::THIN_CLIENT | EnvFlags::WEB_CLIENT);
        let _ = clients;

        let elsif_mask = PreprocCondition::parse("ВебКлиент").narrow_branch(&mut remaining);
        assert_eq!(elsif_mask, EnvFlags::WEB_CLIENT);
        // #Иначе gets the rest.
        assert_eq!(remaining, EnvFlags::THIN_CLIENT);
    }

    #[test]
    fn narrow_branch_drops_undecidable_envs_from_whole_chain() {
        let mut remaining = EnvFlags::THIN_CLIENT | EnvFlags::SERVER;
        let mask = PreprocCondition::parse("Линукс").narrow_branch(&mut remaining);
        assert!(mask.is_empty(), "undecidable branch checks nothing");
        assert!(remaining.is_empty(), "undecidable envs leave the chain entirely");
    }

    #[test]
    fn server_is_undecidable_for_file_mode_capable_envs() {
        assert_eq!(t("Сервер", EnvFlags::THICK_CLIENT_MANAGED), None);
        assert_eq!(t("Сервер", EnvFlags::THICK_CLIENT_ORDINARY), None);
        assert_eq!(t("Сервер", EnvFlags::EXTERNAL_CONNECTION), None);
        assert_eq!(t("НЕ Сервер", EnvFlags::THICK_CLIENT_MANAGED), None);
    }

    #[test]
    fn directive_header_parsing() {
        let h = PreprocCondition::parse_directive_header;
        assert_eq!(h("#Если Сервер Тогда"), PreprocCondition::Symbol(PreprocSymbol::Server));
        assert_eq!(h("# Если НЕ ВебКлиент Тогда").eval(EnvFlags::THIN_CLIENT), Some(true));
        assert_eq!(h("#If Server Then"), PreprocCondition::Symbol(PreprocSymbol::Server));
        assert_eq!(
            h("#ИначеЕсли ТонкийКлиент Тогда"),
            PreprocCondition::Symbol(PreprocSymbol::ThinClient)
        );
        // Error-recovery leftovers must not sneak in as a valid prefix.
        assert_eq!(h("#Если ВебКлиент = 1 Тогда"), PreprocCondition::Unknown);
        assert_eq!(h("#Если ВебКлиент"), PreprocCondition::Unknown);
        assert_eq!(h("#КонецЕсли"), PreprocCondition::Unknown);
        assert_eq!(h(""), PreprocCondition::Unknown);
    }

    #[test]
    fn trailing_cyrillic_comment_does_not_panic() {
        let h = PreprocCondition::parse_directive_header;
        // Exact line from the reported БИТ config: the Cyrillic comment used
        // to slice the header at a non-char-boundary byte and panic.
        let real = "#Если Сервер Или ТолстыйКлиентОбычноеПриложение Или \
                    ВнешнееСоединение Или ТолстыйКлиентУправляемоеПриложение Тогда \
                    // +бит добавлено ТолстыйКлиентУправляемоеПриложение.";
        assert_eq!(h(real).eval(EnvFlags::SERVER), Some(true));
        // The comment must not leak into the extracted condition.
        assert_eq!(
            h("#Если Сервер Тогда // комментарий"),
            PreprocCondition::Symbol(PreprocSymbol::Server)
        );
        assert_eq!(
            h("#If Server Then // comment"),
            PreprocCondition::Symbol(PreprocSymbol::Server)
        );
        // A comment without a valid terminator still yields Unknown, not a panic.
        assert_eq!(h("#Если Сервер // забыли Тогда"), PreprocCondition::Unknown);
        // No comment at all, but a trailing multibyte char shorter than the
        // terminator lands the suffix offset off a char boundary — this is
        // what the `is_char_boundary` guard alone protects.
        assert_eq!(h("#Если Сервер Ж"), PreprocCondition::Unknown);
        // Comment glued to the terminator, and a commented `#ИначеЕсли`.
        assert_eq!(
            h("#Если Сервер Тогда//комментарий"),
            PreprocCondition::Symbol(PreprocSymbol::Server)
        );
        assert_eq!(
            h("#ИначеЕсли Сервер Тогда // +бит"),
            PreprocCondition::Symbol(PreprocSymbol::Server)
        );
    }

    #[test]
    fn pathological_chain_is_capped() {
        let huge = vec!["ТонкийКлиент"; 20_000].join(" ИЛИ ");
        // Must neither overflow the stack nor produce a verdict.
        assert_eq!(PreprocCondition::parse(&huge), PreprocCondition::Unknown);
    }

    #[test]
    fn malformed_conditions_parse_to_unknown() {
        assert_eq!(PreprocCondition::parse(""), PreprocCondition::Unknown);
        assert_eq!(PreprocCondition::parse("И Сервер"), PreprocCondition::Unknown);
        assert_eq!(PreprocCondition::parse("Сервер И"), PreprocCondition::Unknown);
        assert_eq!(PreprocCondition::parse("(Сервер"), PreprocCondition::Unknown);
        assert_eq!(PreprocCondition::parse("Сервер = 1"), PreprocCondition::Unknown);
        assert_eq!(PreprocCondition::parse("Сервер ТонкийКлиент"), PreprocCondition::Unknown);
    }
}
