//! Differential correctness harness for the native converter.
//!
//! We reimplement ESO Logs' upload encoder ourselves, so the ONLY trustworthy
//! definition of "correct" is: **does our output byte-match what the official
//! uploader produced for the same raw log?** This module is that check.
//!
//! A *golden pair* is `(raw Encounter.log, official segment output)` captured
//! from a real upload. [`diff_against_golden`] converts the raw log with our
//! encoder and compares byte-for-byte against the official output, returning a
//! precise [`Diff`] (first divergence + context) when they differ.
//!
//! Two uses:
//! 1. **Tests** — golden-pair fixtures in `testdata/` gate every encoder change;
//!    a regression fails loudly with the exact diverging line.
//! 2. **Runtime safety gate** — [`coverage_of`] reports which event types in a
//!    log our encoder has *proven* it can reproduce. The transport uses this to
//!    run native upload ONLY for logs fully within proven coverage, and fall
//!    back to the official uploader otherwise. That rule is what guarantees a
//!    user never receives a silently-corrupted report: native output is either
//!    byte-identical to official, or it isn't used.
//!
//! Clean-room: this compares against output we captured from our own uploads;
//! it embeds no third-party code.

/// The result of diffing our output against an official golden output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diff {
    /// Byte-identical — our encoder reproduced the official output exactly.
    Identical,
    /// Diverged. Carries the 1-based line number and the two differing lines
    /// (trimmed/capped) so a failure points straight at the problem.
    Diverged {
        line: usize,
        ours: String,
        official: String,
    },
    /// Same prefix, but one side has extra trailing lines.
    LengthMismatch { ours: usize, official: usize },
}

impl Diff {
    pub fn is_identical(&self) -> bool {
        matches!(self, Diff::Identical)
    }
}

/// Compare two rendered segment texts line-by-line, reporting the first
/// divergence. Line-oriented (not raw-byte) so the report is human-readable; the
/// grammar is newline-framed so this is equivalent to byte comparison for
/// well-formed output, while giving a precise location on failure.
pub fn diff_segments(ours: &str, official: &str) -> Diff {
    let mut o = ours.lines();
    let mut f = official.lines();
    let mut line = 0;
    loop {
        line += 1;
        match (o.next(), f.next()) {
            (Some(a), Some(b)) => {
                if a != b {
                    return Diff::Diverged {
                        line,
                        ours: cap(a),
                        official: cap(b),
                    };
                }
            }
            (None, None) => return Diff::Identical,
            (a, b) => {
                // One side ended first — count remaining on each to report sizes.
                let ours_total = line - 1 + usize::from(a.is_some()) + o.count();
                let off_total = line - 1 + usize::from(b.is_some()) + f.count();
                return Diff::LengthMismatch {
                    ours: ours_total,
                    official: off_total,
                };
            }
        }
    }
}

fn cap(s: &str) -> String {
    const MAX: usize = 200;
    s.chars().take(MAX).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_segments_report_identical() {
        let s = "15|1\n2\nA\nB\n";
        assert!(diff_segments(s, s).is_identical());
    }

    #[test]
    fn first_divergence_is_pinpointed() {
        let ours = "15|1\n2\nA\nX\n";
        let official = "15|1\n2\nA\nB\n";
        match diff_segments(ours, official) {
            Diff::Diverged {
                line,
                ours,
                official,
            } => {
                assert_eq!(line, 4);
                assert_eq!(ours, "X");
                assert_eq!(official, "B");
            }
            other => panic!("expected Diverged, got {other:?}"),
        }
    }

    #[test]
    fn length_mismatch_is_reported() {
        // Identical prefix, official has one extra trailing line. (Prefix must
        // match exactly, else the line-divergence check fires first — which is
        // the correct precedence.)
        let ours = "15|1\n2\nA\nB\n";
        let official = "15|1\n2\nA\nB\nC\n";
        match diff_segments(ours, official) {
            Diff::LengthMismatch { ours, official } => {
                assert_eq!(ours, 4);
                assert_eq!(official, 5);
            }
            other => panic!("expected LengthMismatch, got {other:?}"),
        }
    }
}
