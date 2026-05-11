//! TEST_0209 (WKC matches → Up-eligible) and TEST_0210 (WKC mismatch
//! → Degraded with reason naming the offending cycle). Pure logic;
//! the actual `ConnectorHealth` transition happens in the gateway's
//! cycle loop in C5c.

#![allow(clippy::doc_markdown)]

use sonic_connector_ethercat::{WkcVerdict, evaluate_wkc};

#[test]
fn observed_equal_to_expected_matches() {
    assert_eq!(evaluate_wkc(3, 3), WkcVerdict::Match);
}

#[test]
fn observed_above_expected_still_matches() {
    // Spec only forbids `observed < expected`; tolerance over
    // expectations is acceptable (a SubDevice may respond on more
    // datagrams than strictly required by the mapping).
    assert_eq!(evaluate_wkc(2, 5), WkcVerdict::Match);
}

#[test]
fn observed_below_expected_is_mismatch() {
    assert_eq!(
        evaluate_wkc(3, 2),
        WkcVerdict::Mismatch {
            observed: 2,
            expected: 3,
        }
    );
}

#[test]
fn match_has_no_degraded_reason() {
    let v = evaluate_wkc(1, 1);
    assert!(v.degraded_reason(42).is_none());
}

#[test]
fn mismatch_reason_names_observed_expected_and_cycle() {
    let v = evaluate_wkc(7, 3);
    let reason = v
        .degraded_reason(101)
        .expect("mismatch must produce a reason");
    assert!(
        reason.contains("101"),
        "reason should name cycle index 101: {reason}"
    );
    assert!(
        reason.contains('3'),
        "reason should name observed=3: {reason}"
    );
    assert!(
        reason.contains('7'),
        "reason should name expected=7: {reason}"
    );
}

#[test]
fn zero_observed_is_mismatch_when_anything_expected() {
    assert_eq!(
        evaluate_wkc(1, 0),
        WkcVerdict::Mismatch {
            observed: 0,
            expected: 1,
        }
    );
}

#[test]
fn zero_expected_always_matches() {
    // A SubDevice with no mapping contributes zero to the expected
    // count; any observed value satisfies that.
    assert_eq!(evaluate_wkc(0, 0), WkcVerdict::Match);
    assert_eq!(evaluate_wkc(0, 5), WkcVerdict::Match);
}
