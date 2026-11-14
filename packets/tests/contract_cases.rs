//! openroad's readers against the original client's reading contract.
//!
//! IDEA. The studio keeps, outside this repository, a contract of what the
//! original client reads for each server opcode, and a tool that turns every
//! reading path into a frame of exactly that length (`vertrag.py faelle
//! --tsv`). The original asserts on a frame it reads more or less of; openroad
//! reads leniently and survives, which hides a wrong form. So every exact path
//! of an opcode openroad decodes is judged with `packets::judge`: a frame left
//! partly unread (`Short`) is a form that reads too little, a frame that does
//! not decode (`Error`) one that reads too much, and a lossy re-encoding is
//! checked with the frame one byte shorter ([`stops_early`]).
//! Each fails here instead of against the original. (The same frame with one
//! byte more is no test: many types keep a body they do not fully model raw,
//! on purpose.)
//!
//! The cases are the user's local material: set `OPENROAD_CONTRACT_CASES` to
//! the TSV. Without it the test is ignored, and run without the file it says
//! so instead of passing on nothing.

use packets::judge::{judge, Verdict};

#[test]
#[ignore = "needs the contract cases: OPENROAD_CONTRACT_CASES=<vertrag-faelle.tsv> \
            cargo test -p packets --test contract_cases -- --ignored --nocapture"]
fn every_modelled_opcode_reads_as_far_as_the_original() {
    let path = std::env::var("OPENROAD_CONTRACT_CASES")
        .expect("OPENROAD_CONTRACT_CASES names the cases TSV");
    let text = std::fs::read_to_string(&path).expect("the cases TSV is readable");

    let (mut checked, mut unwired, mut skipped) = (0usize, 0usize, 0usize);
    let mut wrong: Vec<String> = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#')).skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        let [opcode, _receiver, _stand, kind, path_, body] = cols[..] else {
            continue;
        };
        // Only paths whose length the contract fixes: `rest` reads to the end,
        // `daten` depends on the client's tables, `U` is unread.
        if kind != "genau" {
            skipped += 1;
            continue;
        }
        let opcode = u16::from_str_radix(opcode.trim_start_matches("0x"), 16).expect("opcode");
        let body: Vec<u8> = (0..body.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&body[i..i + 2], 16).expect("hex body"))
            .collect();
        let exact = judge(opcode, &body);
        if exact == Verdict::Unwired {
            unwired += 1;
            continue;
        }
        checked += 1;
        if stops_early(opcode, &body, exact) {
            wrong.push(format!(
                "{opcode:#06X} {} B [{path_}]: {exact:?}",
                body.len()
            ));
        }
    }
    eprintln!(
        "{checked} exact paths of modelled opcodes, {} read differently; {unwired} unwired, {skipped} not exact",
        wrong.len()
    );
    assert!(checked > 0, "no exact path of a modelled opcode in {path}");
    assert!(
        wrong.is_empty(),
        "openroad reads these differently from the original:\n{}",
        wrong.join("\n")
    );
}

/// Whether openroad stops before the end of a frame the original reads whole.
///
/// `Short` and `Error` say so directly. A `Lossy` re-encoding hides it: a
/// result byte 2 read as `false` and written back as 0 is shorter than the
/// body, just like a tail that was read and dropped on purpose. The frame
/// without its last byte tells the two apart, because a decoder that reads to
/// the end refuses it. (`Differs` is not probed: its re-encoding is as long as
/// the body, and a lenient reader of an optional last byte would pass the
/// probe without having stopped early.)
fn stops_early(opcode: u16, body: &[u8], exact: Verdict) -> bool {
    match exact {
        Verdict::Short(_) | Verdict::Error => true,
        Verdict::Lossy => {
            !body.is_empty() && judge(opcode, &body[..body.len() - 1]) != Verdict::Error
        }
        _ => false,
    }
}
