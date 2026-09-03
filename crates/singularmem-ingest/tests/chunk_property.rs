use proptest::prelude::*;
use singularmem_ingest::chunk_text;

proptest! {
    #[test]
    fn chunks_are_nonempty_bounded_and_reassemble(
        paras in prop::collection::vec("[a-zA-Z0-9 .,!?]{0,300}", 0..12),
        max in 16usize..512,
    ) {
        let text = paras.join("\n\n");
        let chunks = chunk_text(&text, max);
        for c in &chunks {
            prop_assert!(!c.trim().is_empty(), "empty chunk");
            prop_assert!(c.len() <= max, "chunk {} > {max}", c.len());
        }
        // Reassembly: joining chunks and re-normalising equals the normalised input.
        let norm = |s: &str| s.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n");
        let rejoined: String = chunks.join("\n\n");
        // Hard splits inside a paragraph remove no characters, so the
        // concatenation of chunks without separators must contain every
        // non-whitespace char of the input in order.
        let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        prop_assert_eq!(strip(&rejoined), strip(&norm(&text)));
    }
}

#[test]
fn short_text_is_one_chunk() {
    assert_eq!(chunk_text("hello world", 4096), vec!["hello world"]);
}

#[test]
fn empty_text_is_no_chunks() {
    assert!(chunk_text("   \n\n  ", 4096).is_empty());
}

#[test]
fn splits_on_blank_lines_greedily() {
    let text = "aaaa\n\nbbbb\n\ncccc";
    assert_eq!(chunk_text(text, 10), vec!["aaaa\n\nbbbb", "cccc"]);
}

#[test]
fn hard_splits_oversized_paragraph_on_char_boundary() {
    let text = "ééééé"; // 10 bytes, 2 per char
    let chunks = chunk_text(text, 5);
    assert_eq!(chunks, vec!["éé", "éé", "é"]);
}

#[test]
fn tiny_max_is_floored_to_four_bytes() {
    assert_eq!(chunk_text("éé", 1), vec!["éé"]);

    let chunks = chunk_text("abcdefgh", 1);
    for c in &chunks {
        assert!(c.len() <= 4, "chunk {c:?} exceeds the 4-byte floor");
    }
    assert_eq!(chunks.concat(), "abcdefgh");
}

#[test]
fn whitespace_only_hard_split_piece_is_dropped() {
    let text = format!("a{}b", " ".repeat(10));
    assert_eq!(chunk_text(&text, 5), vec!["a", "b"]);
}
