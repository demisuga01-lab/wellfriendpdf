//! Unexecuted regression source for the non-painting logical alphabet.
use super::*;

#[test]
fn stable_codes_keep_the_original_separators_and_supplementary_scalars() {
    assert_eq!(
        encode("\r\n\u{000b}\u{000c}\u{0085}\u{2028}\u{2029}").unwrap(),
        "0001000200030004000500060007"
    );
    assert_eq!(
        encode("\u{200d}\u{061c}\u{fe0f}\u{e0100}").unwrap(),
        "0013000A003A014D"
    );
    let tab_code = code('\t').unwrap();
    assert!(tab_code > code('\u{e0100}').unwrap());
    let mut codes = BTreeSet::new();
    for ch in SEPARATORS.into_iter().chain(std::iter::once('\t')).chain(
        REMOVED_RANGES
            .iter()
            .flat_map(|&(start, end)| start..=end)
            .map(|ch| char::from_u32(ch).unwrap()),
    ) {
        let cid = code(ch).unwrap();
        assert_ne!(cid, 0);
        assert!(codes.insert(cid), "duplicate carrier code for {ch:?}");
    }
}

#[test]
fn unsupported_controls_and_painted_whitespace_are_not_silently_suppressed() {
    for text in [
        "",
        "A",
        " ",
        "\u{00a0}",
        "\u{115f}",
        "\u{180f}",
        "\u{1bca0}",
        "\0",
        "A\u{200d}",
    ] {
        assert!(!is_text(text));
        if !text.is_empty() {
            assert!(encode(text).is_err());
        }
    }
    for text in [
        "\t",
        "\u{200d}",
        "\u{2067}\u{2069}",
        "\u{e0100}\r\n",
        "\u{00ad}",
    ] {
        assert!(is_text(text));
        assert!(encode(text).is_ok());
    }
}

#[test]
fn only_used_controls_enlarge_the_mapping_alphabet() {
    let chars = alphabet(["\u{200d}\u{200d}", "\n"]).unwrap();
    assert_eq!(chars.len(), 8);
    assert!(chars.contains(&'\u{200d}'));
    assert!(!chars.contains(&'\u{e0100}'));
    assert!(alphabet(["paint"]).is_err());
    assert!(encode(&"\n".repeat(4_000_001)).is_err());
}

#[test]
fn logical_encoding_and_alphabet_observe_cancellation() {
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel.scope(|| encode("\u{200d}")).is_err());
    assert!(cancel.scope(|| alphabet(["\u{200d}"])).is_err());
}
