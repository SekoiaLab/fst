use crate::{regex::escape::escape_start_and_end_anchors, Automaton};
use regex_syntax;
use std::fmt;

mod compile;
mod dfa;
mod error;
mod escape;
mod sparse;

pub use self::error::Error;

/// A regular expression for searching FSTs with Unicode support.
///
/// Regular expressions are compiled down to a deterministic finite automaton
/// that can efficiently search any finite state transducer. Notably, most
/// regular expressions only need to explore a small portion of a finite state
/// transducer without loading all of it into memory.
///
/// # Syntax
///
/// `Regex` supports fully featured regular expressions. Namely, it supports
/// all of the same constructs as the standard `regex` crate except for the
/// following things:
///
/// 1. Lazy quantifiers, since a regular expression automaton only reports
///    whether a key matches at all, and not its location. Namely, lazy
///    quantifiers such as `+?` only modify the location of a match, but never
///    change a non-match into a match or a match into a non-match.
/// 2. Word boundaries (i.e., `\b`). Because such things are hard to do in
///    a deterministic finite automaton, but not impossible. As such, these
///    may be allowed some day.
/// 3. Other zero width assertions like `^` and `$`. These are easier to
///    support than word boundaries, but are still tricky and usually aren't
///    as useful when searching dictionaries.
///
/// Otherwise, the [full syntax of the `regex`
/// crate](http://doc.rust-lang.org/regex/regex/index.html#syntax)
/// is supported. This includes all Unicode support and relevant flags.
/// (The `U` and `m` flags are no-ops because of (1) and (3) above,
/// respectively.)
///
/// # Matching semantics
///
/// A regular expression matches a key in a finite state transducer if and only
/// if it matches from the start of a key all the way to end. Stated
/// differently, every regular expression `(re)` is matched as if it were
/// `^(re)$`. This means that if you want to do a substring match, then you
/// must use `.*substring.*`.
///
/// **Caution**: Starting a regular expression with `.*` means that it could
/// potentially match *any* key in a finite state transducer. This implies that
/// all keys could be visited, which could be slow. It is possible that this
/// crate will grow facilities for detecting regular expressions that will
/// scan a large portion of a transducer and optionally disallow them.
///
pub struct Regex {
    original: String,
    dfa: dfa::Dfa,
}

#[derive(Eq, PartialEq)]
pub enum Inst {
    Match,
    Jump(usize),
    Split(usize, usize),
    Range(u8, u8),
}

/// Default maximum size (in bytes) of the compiled regex program.
const DEFAULT_SIZE_LIMIT: usize = 10 * (1 << 20);

/// The result of compiling a disjunction of regexes.
pub enum DisjunctionRegex {
    /// The regexes were compiled into a single DFA.
    Single(Regex),
    /// The regexes had to be compiled into multiple DFAs.
    Multi(Vec<Regex>),
}

impl Regex {
    /// Create a new regular expression query.
    ///
    /// The query finds all terms matching the regular expression.
    ///
    /// If the regular expression is malformed or if it results in an automaton
    /// that is too big, then an error is returned.
    ///
    /// A `Regex` value satisfies the `Automaton` trait, which means it can be
    /// used with the `search` method of any finite state transducer.
    #[inline]
    pub fn new(re: &str) -> Result<Regex, Error> {
        Regex::with_size_limit(DEFAULT_SIZE_LIMIT, re)
    }

    fn with_size_limit(size: usize, re: &str) -> Result<Regex, Error> {
        let hir = regex_syntax::Parser::new().parse(re)?;
        Regex::from_hir_with_size_limit(size, hir, re)
    }

    fn from_hir_with_size_limit(
        size: usize,
        hir: regex_syntax::hir::Hir,
        original: impl ToString,
    ) -> Result<Regex, Error> {
        let escaped_hir = escape_start_and_end_anchors(hir);
        let insts = self::compile::Compiler::new(size).compile(&escaped_hir)?;
        let dfa = self::dfa::DfaBuilder::new(insts).build()?;
        Ok(Regex {
            original: original.to_string(),
            dfa,
        })
    }

    /// Build a regex automaton that matches the **union** of the given patterns
    /// (i.e. as if they were joined by alternation `|`).
    ///
    /// Each pattern is parsed independently and combined at the HIR level with
    /// `Hir::alternation`, which avoids string-splicing pitfalls (operator
    /// precedence, inline flags, anchors) of concatenating raw pattern strings.
    ///
    /// For some alternations, e.g regexes starting with a wildcard, the DFA
    /// grows exponentially. If the size limit is exceeded, we fall back to
    /// per-pattern automata. This is slower because you need to perform one
    /// state transition per pattern and state pruning is less efficient.
    ///
    /// Note: an empty slice yields an error. Callers that require at
    /// least one pattern should enforce that themselves.
    #[inline]
    pub fn from_patterns<S>(patterns: &[S]) -> Result<DisjunctionRegex, Error>
    where
        S: AsRef<str> + std::fmt::Debug,
    {
        Regex::from_patterns_with_size_limit(DEFAULT_SIZE_LIMIT, patterns)
    }

    /// Same as [`Regex::from_patterns`], with an explicit compiled-size limit.
    fn from_patterns_with_size_limit<S>(
        size: usize,
        patterns: &[S],
    ) -> Result<DisjunctionRegex, Error>
    where
        S: AsRef<str> + std::fmt::Debug,
    {
        let mut hirs = Vec::with_capacity(patterns.len());

        for pattern in patterns {
            hirs.push(regex_syntax::Parser::new().parse(pattern.as_ref())?);
        }
        // this clone is a bit unfortunate, but we can live with it.
        let combined = regex_syntax::hir::Hir::alternation(hirs.clone());
        match Regex::from_hir_with_size_limit(size, combined, format!("{patterns:?}")) {
            Ok(regex) => Ok(DisjunctionRegex::Single(regex)),
            Err(Error::TooManyStates(_)) => {
                // DFA sizes can grow exponentially with the number of patterns.
                // If its size exceeds the limit, compile each pattern
                // separately.
                let mut regexes = Vec::with_capacity(hirs.len());
                for (hir, pattern) in hirs.into_iter().zip(patterns) {
                    regexes.push(Regex::from_hir_with_size_limit(
                        size,
                        hir,
                        pattern.as_ref(),
                    )?);
                }
                Ok(DisjunctionRegex::Multi(regexes))
            }
            Err(e) => Err(e),
        }
    }
}

impl Automaton for Regex {
    type State = Option<usize>;

    #[inline]
    fn start(&self) -> Option<usize> {
        Some(0)
    }

    #[inline]
    fn is_match(&self, state: &Option<usize>) -> bool {
        state.map(|state| self.dfa.is_match(state)).unwrap_or(false)
    }

    #[inline]
    fn can_match(&self, state: &Option<usize>) -> bool {
        state.is_some()
    }

    #[inline]
    fn accept(&self, state: &Option<usize>, byte: u8) -> Option<usize> {
        state.and_then(|state| self.dfa.accept(state, byte))
    }
}

impl Automaton for Vec<Regex> {
    type State = Vec<<Regex as Automaton>::State>;

    fn start(&self) -> Self::State {
        self.iter().map(|r| r.start()).collect()
    }

    fn is_match(&self, state: &Self::State) -> bool {
        self.iter().zip(state).any(|(r, s)| r.is_match(s))
    }

    fn can_match(&self, state: &Self::State) -> bool {
        self.iter().zip(state).any(|(r, s)| r.can_match(s))
    }

    fn accept(&self, state: &Self::State, byte: u8) -> Self::State {
        self.iter()
            .zip(state)
            .map(|(r, s)| r.accept(s, byte))
            .collect()
    }
}

impl fmt::Debug for Regex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Regex({:?})", self.original)?;
        self.dfa.fmt(f)
    }
}

impl fmt::Debug for Inst {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::Inst::*;
        match *self {
            Match => write!(f, "Match"),
            Jump(ip) => write!(f, "JUMP {}", ip),
            Split(ip1, ip2) => write!(f, "SPLIT {}, {}", ip1, ip2),
            Range(s, e) => write!(f, "RANGE {:X}-{:X}", s, e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Automaton;

    fn matches(re: impl Automaton, s: &str) -> bool {
        let mut state = re.start();
        for &b in s.as_bytes() {
            state = re.accept(&state, b);
        }
        re.is_match(&state)
    }

    #[test]
    fn test_from_patterns_union_equivalence() {
        let combined = Regex::from_patterns(&["abc.*", "xyz"]).unwrap();
        let DisjunctionRegex::Single(combined) = combined else {
            panic!("Expected DisjunctionRegex::Single")
        };
        let re_abc = Regex::new("abc.*").unwrap();
        let re_xyz = Regex::new("xyz").unwrap();
        let re_alt = Regex::new("(?:abc.*)|(?:xyz)").unwrap();

        for key in &["abcdef", "abc", "xyz", "nope", "xyzz", "ab"] {
            let expected = matches(&re_abc, key) || matches(&re_xyz, key);
            assert_eq!(
                matches(&combined, key),
                expected,
                "from_patterns mismatch on {:?}",
                key
            );
            assert_eq!(
                matches(&re_alt, key),
                expected,
                "alternation string mismatch on {:?}",
                key
            );
        }
    }

    #[test]
    fn test_anchors_parity() {
        // Patterns with ^ and $ are treated as literals by this crate.
        // Verify from_patterns([pat]) behaves identically to new(pat).
        let pat = "^hello$";
        let from_new = Regex::new(pat).unwrap();
        let from_patterns = Regex::from_patterns(&[pat]).unwrap();
        let DisjunctionRegex::Single(from_patterns) = from_patterns else {
            panic!("Expected DisjunctionRegex::Single")
        };

        for key in &["hello", "^hello$", "", "hello\n"] {
            assert_eq!(
                matches(&from_new, key),
                matches(&from_patterns, key),
                "anchor parity mismatch on {:?}",
                key
            );
        }
    }

    #[test]
    fn test_size_limit_error() {
        // A tiny size limit should produce a compilation error, not a panic.
        let result = Regex::from_patterns_with_size_limit(1, &["abc.*", "xyz.*"]);
        assert!(
            result.is_err(),
            "expected error with tiny size limit, got Ok"
        );
    }

    #[test]
    fn test_from_patterns_substring_builds_multi() {
        // 10 substring patterns (.*x.*) cause the combined DFA to grow
        // exponentially and exceed the default size limit
        let patterns: Vec<String> = (0..10).map(|i| format!(".*{}word.*", i)).collect();
        let result = Regex::from_patterns(&patterns).unwrap();
        let DisjunctionRegex::Multi(ref regexes) = result else {
            panic!("expected DisjunctionRegex::Multi for 10 wildcard-prefixed patterns");
        };

        let test_keys = [
            ("hello0wordbye", true),
            ("foo5wordbar", true),
            ("9word", true),
            ("0wordx", true),
            ("word", false),
            ("nope", false),
            ("", false),
            ("3wordmiddle", true),
            ("prefix1wordsuffix", true),
        ];

        for (key, expected) in &test_keys {
            assert_eq!(
                matches(regexes, key),
                *expected,
                "multi mismatch on {:?}",
                key
            );
        }
    }

    #[test]
    fn test_empty_patterns_matches_nothing() {
        // `Hir::alternation(vec![])` produces `Hir::fail()`, which this
        // crate's compiler cannot represent as a DFA (it returns `NoBytes`).
        // Callers must therefore ensure at least one pattern is provided.
        let result = Regex::from_patterns(&[] as &[&str]);
        assert!(matches!(
            result,
            Err(Error::NoBytes)
        ));
    }
}
