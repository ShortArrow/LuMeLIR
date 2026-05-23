//! Phase 2.devinfra-adr-cleanup: sanity check that the ADR docs
//! filesystem stays in sync with the rest of the repo.
//!
//! Catches the "AGENTS row landed but `docs/design/NNNN-*.md`
//! never written" failure mode that motivated the cleanup
//! commit. The dense-numbering test ensures every ADR slot has
//! a doc file; the AGENTS-reference test ensures every
//! `(ADR NNNN)` mention in the progress table points at an
//! existing file.
//!
//! Both tests are filesystem-only, pure validation — no
//! codegen / parser / HIR dependency. They run on every
//! `cargo test` so a future feature commit that omits the
//! corresponding ADR doc fails fast.

use std::collections::BTreeSet;
use std::fs;

/// Collect the 4-digit ID prefix of every `docs/design/NNNN-*.md`
/// file. Non-ADR docs (`README.md`, `tagged-semantics.md`) skip
/// because they do not start with a digit.
fn list_adr_files() -> BTreeSet<u32> {
    fs::read_dir("docs/design")
        .expect("read docs/design")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".md") {
                return None;
            }
            let prefix: String = name.chars().take(4).collect();
            prefix.parse::<u32>().ok()
        })
        .collect()
}

#[test]
fn adr_doc_numbering_is_dense() {
    // No gaps from 0001 to max(found). A missing ID means the
    // last feature commit forgot to land the matching ADR doc.
    let files = list_adr_files();
    let max = *files.iter().max().expect("at least one ADR file");
    let mut missing: Vec<u32> = Vec::new();
    for n in 1..=max {
        if !files.contains(&n) {
            missing.push(n);
        }
    }
    assert!(
        missing.is_empty(),
        "ADR doc numbering has gaps; missing files for IDs: {missing:?} \
         (each gap means a feature shipped without its docs/design/\
         {{id:04}}-*.md — add the ADR doc to close the gap)"
    );
}

#[test]
fn agents_md_adr_references_resolve() {
    // Every `(ADR NNNN)` mention in `AGENTS.md` must point at an
    // existing ADR doc. Loose token scan so AGENTS formatting
    // changes (e.g. extra punctuation) stay robust.
    let agents = fs::read_to_string("AGENTS.md").expect("read AGENTS.md");
    let files = list_adr_files();
    let mut missing: BTreeSet<u32> = BTreeSet::new();
    // Find "ADR NNNN" pairs — `ADR` token followed by a 4+ digit
    // token. AGENTS uses `(ADR 0118)` style consistently, so the
    // scan is precise enough without a regex dependency.
    let mut prev_was_adr = false;
    for raw in agents.split_whitespace() {
        if prev_was_adr {
            // Strip leading non-digit chars (e.g. `0118)` → 0118).
            let digits: String = raw
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if digits.len() == 4
                && let Ok(n) = digits.parse::<u32>()
                && !files.contains(&n)
            {
                missing.insert(n);
            }
            prev_was_adr = false;
        } else if raw == "ADR" || raw.ends_with("ADR") {
            prev_was_adr = true;
        }
    }
    assert!(
        missing.is_empty(),
        "AGENTS.md references missing ADR docs: {missing:?} \
         (each missing ID needs a docs/design/{{id:04}}-*.md file)"
    );
}
