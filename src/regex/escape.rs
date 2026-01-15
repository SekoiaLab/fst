use regex_syntax::hir::{Hir, HirKind, Look};

pub(super) fn escape_start_and_end_anchors(hir: Hir) -> Hir {
    match hir.into_kind() {
        HirKind::Alternation(alternations) => {
            let new_alternations: Vec<Hir> = alternations
                .into_iter()
                .map(escape_start_and_end_anchors)
                .collect();
            Hir::alternation(new_alternations)
        }
        HirKind::Concat(concats) => {
            let new_concats: Vec<Hir> = concats
                .into_iter()
                .map(escape_start_and_end_anchors)
                .collect();
            Hir::concat(new_concats)
        }
        HirKind::Capture(mut capture) => {
            capture.sub = Box::new(escape_start_and_end_anchors(*capture.sub));
            Hir::capture(capture)
        }
        HirKind::Class(class) => Hir::class(class),
        HirKind::Empty => Hir::empty(),
        HirKind::Literal(literal) => Hir::literal(literal.0),
        HirKind::Repetition(mut repetition) => {
            repetition.sub = Box::new(escape_start_and_end_anchors(*repetition.sub));
            Hir::repetition(repetition)
        }
        HirKind::Look(Look::Start) => Hir::literal([b'^']),
        HirKind::Look(Look::End) => Hir::literal([b'$']),
        HirKind::Look(look) => Hir::look(look),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex_syntax::hir::HirKind;

    #[test]
    fn test_escape_start_and_end_anchors() {
        let hir = Hir::look(Look::Start);
        let escaped_hir = escape_start_and_end_anchors(hir);
        let HirKind::Literal(literal) = escaped_hir.kind() else {
            panic!("Expected Literal")
        };
        assert_eq!(literal.0, vec![b'^'].into_boxed_slice());

        let hir = Hir::look(Look::End);
        let escaped_hir = escape_start_and_end_anchors(hir);
        let HirKind::Literal(literal) = escaped_hir.kind() else {
            panic!("Expected Literal")
        };
        assert_eq!(literal.0, vec![b'$'].into_boxed_slice());
    }

    #[test]
    fn test_inside_alternation() {
        let hir = regex_syntax::Parser::new().parse(r"\^|^").unwrap();
        let escaped_hir = escape_start_and_end_anchors(hir);
        let HirKind::Literal(literal) = escaped_hir.kind() else {
            panic!("Expected Literal")
        };
        assert_eq!(literal.0, vec![b'^'].into_boxed_slice());

        let hir = regex_syntax::Parser::new()
            .parse(r"(\W|^)hello(\W|$)")
            .unwrap();
        let escaped_hir = escape_start_and_end_anchors(hir);
        let HirKind::Concat(concats) = escaped_hir.kind() else {
            panic!("Expected Concat")
        };
        assert_eq!(concats.len(), 3);
        {
            let HirKind::Capture(capture) = concats[0].kind() else {
                panic!("Expected Capture")
            };
            let HirKind::Alternation(alternations) = capture.sub.kind() else {
                panic!("Expected Alternation")
            };
            assert!(matches!(alternations[0].kind(), HirKind::Class(_)));
            assert!(matches!(alternations[1].kind(), HirKind::Literal(_)));
        }
        {
            let HirKind::Literal(literal) = concats[1].kind() else {
                panic!("Expected Literal")
            };
            assert_eq!(
                literal.0,
                vec![b'h', b'e', b'l', b'l', b'o'].into_boxed_slice()
            );
        }
        {
            let HirKind::Capture(capture) = concats[2].kind() else {
                panic!("Expected Capture")
            };
            let HirKind::Alternation(alternations) = capture.sub.kind() else {
                panic!("Expected Alternation")
            };
            assert!(matches!(alternations[0].kind(), HirKind::Class(_)));
            assert!(matches!(alternations[1].kind(), HirKind::Literal(_)));
        }
    }

    #[test]
    fn test_inside_adjacent_literal() {
        let hir = regex_syntax::Parser::new().parse("ab^cd").unwrap();
        let escaped_hir = escape_start_and_end_anchors(hir);
        let HirKind::Literal(literal) = escaped_hir.kind() else {
            panic!("Expected Literal")
        };
        assert_eq!(
            literal.0,
            vec![b'a', b'b', b'^', b'c', b'd'].into_boxed_slice()
        );

        let hir = regex_syntax::Parser::new().parse("abcd$").unwrap();
        let escaped_hir = escape_start_and_end_anchors(hir);
        let HirKind::Literal(literal) = escaped_hir.kind() else {
            panic!("Expected Literal")
        };
        assert_eq!(
            literal.0,
            vec![b'a', b'b', b'c', b'd', b'$'].into_boxed_slice()
        );
    }

    #[test]
    fn test_escape_side_effects() {
        let regexes = vec![
            "hello",
            r"\^",
            r"hello\$",
            r"(\W|a)hello(b|\W)",
            r"ab\^cd",
            r"[^abc]",
        ];
        for regex_str in regexes {
            let hir = regex_syntax::Parser::new().parse(regex_str).unwrap();
            let escaped_hir = escape_start_and_end_anchors(hir.clone());
            assert_eq!(hir, escaped_hir, "failed for regex: {}", regex_str);
        }
    }
}
