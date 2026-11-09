//! Edit drafts of the exception rule lists.

use fvm_contract::app_internal::problem::AddressProblem;
use fvm_contract::app_internal::section::RowProblem;
use fvm_core::bypass_rules::{Ipv4Network, ListDraft};

fn net(text: &str) -> Ipv4Network {
    text.parse().unwrap()
}
fn host() -> Ipv4Network {
    net("10.30.161.16")
}
fn network() -> Ipv4Network {
    net("10.30.170.0/24")
}
fn other() -> Ipv4Network {
    net("10.50.100.10")
}

fn rows(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| text.to_string()).collect()
}

#[test]
fn starts_from_the_saved_list_without_changes() {
    let draft = ListDraft::new(vec![host(), network()]);
    assert_eq!(draft.rows(), rows(&["10.30.161.16", "10.30.170.0/24"]));
    assert!(!draft.has_changes());
    assert!(draft.problems().is_empty());
    assert_eq!(draft.entries(), Some(vec![host(), network()]));
}

#[test]
fn edits_are_changes_until_discarded() {
    let mut draft = ListDraft::new(vec![host()]);
    draft.set_rows(rows(&["10.30.161.16", "  10.30.170.0/24 ", ""]));
    assert_eq!(draft.rows(), rows(&["10.30.161.16", "10.30.170.0/24"]));
    assert!(draft.has_changes());
    assert_eq!(draft.entries(), Some(vec![host(), network()]));
    draft.discard_changes();
    assert_eq!(draft.rows(), rows(&["10.30.161.16"]));
    assert!(!draft.has_changes());
}

#[test]
fn respelling_is_no_change() {
    let mut draft = ListDraft::new(vec![host()]);
    draft.set_rows(rows(&["10.30.161.16/32"]));
    assert!(!draft.has_changes());
}

#[test]
fn problem_rows_block_apply() {
    let mut draft = ListDraft::new(vec![]);
    draft.set_rows(rows(&[
        "10.30.161",
        "10.30.161.16",
        "10.30.161.16/32",
        "10.30.170.5/24",
    ]));
    assert_eq!(
        draft.problems(),
        vec![
            (
                0,
                RowProblem::Invalid {
                    problem: AddressProblem::Malformed {}
                }
            ),
            (
                2,
                RowProblem::Duplicate {
                    entry: "10.30.161.16".into()
                }
            ),
            (
                3,
                RowProblem::Invalid {
                    problem: AddressProblem::HostBitsSet {
                        network: "10.30.170.0/24".into()
                    }
                }
            ),
        ]
    );
    assert_eq!(draft.entries(), None);
    assert!(draft.has_changes());
}

#[test]
fn overlapping_rows_block_apply() {
    let mut draft = ListDraft::new(vec![]);
    draft.set_rows(rows(&["10.30.170.0/24", "10.30.170.9", "10.30.0.0/16"]));
    let overlap = RowProblem::Overlap {
        other: "10.30.170.0/24".into(),
    };
    assert_eq!(draft.problems(), vec![(1, overlap.clone()), (2, overlap)]);
    assert_eq!(draft.entries(), None);
}

#[test]
fn without_edits_the_rows_follow_the_saved_list() {
    let mut draft = ListDraft::new(vec![host()]);
    draft.merge_saved(vec![network(), host()]);
    assert_eq!(draft.rows(), rows(&["10.30.170.0/24", "10.30.161.16"]));
    assert!(!draft.has_changes());
}

#[test]
fn outside_changes_merge_into_edits() {
    let mut draft = ListDraft::new(vec![host(), network()]);
    draft.set_rows(rows(&["10.30.161.16", "10.30.170.0/24", "10.30.99"]));
    // Elsewhere: `network` removed, `other` added.
    draft.merge_saved(vec![host(), other()]);
    assert_eq!(
        draft.rows(),
        rows(&["10.30.161.16", "10.30.99", "10.50.100.10"])
    );
    assert_eq!(draft.saved(), [host(), other()]);
    assert!(draft.has_changes());
}

#[test]
fn add_skips_an_entry_already_listed() {
    let mut draft = ListDraft::new(vec![network()]);
    draft.add(network());
    draft.add(host());
    assert_eq!(draft.rows(), rows(&["10.30.170.0/24", "10.30.161.16"]));
}

#[test]
fn rows_overlapping_an_entry() {
    let mut draft = ListDraft::new(vec![network()]);
    draft.set_rows(rows(&["10.30.170.0/24", "not an address"]));
    assert!(draft.has_row_overlapping(&network()));
    assert!(draft.has_row_overlapping(&net("10.30.170.5")));
    assert!(draft.has_row_overlapping(&net("10.30.0.0/16")));
    assert!(!draft.has_row_overlapping(&host()));
}

#[test]
fn merge_does_not_repeat_a_row_the_user_already_typed() {
    let mut draft = ListDraft::new(vec![host()]);
    draft.set_rows(rows(&["10.30.161.16", "10.50.100.10/32"]));
    draft.merge_saved(vec![host(), other()]);
    assert_eq!(draft.rows(), rows(&["10.30.161.16", "10.50.100.10/32"]));
    assert!(!draft.has_changes());
}
