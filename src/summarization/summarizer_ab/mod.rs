// Summarizer A/B harness support module (issue #773).
//
// Provides the fixed, checked-in entity sample and the parse-success scorer
// that the A/B harness example (`examples/summarizer_ab.rs`) uses to measure
// candidate summarizer backends on lievo's own source.
//
// This module is intentionally self-contained: it has no dependency on
// `pack_by_char_budget`, `parse_batch_response`, or any other existing pub fn
// in `src/summarization/`. It only depends on `std`.

pub mod sample;
pub mod scoring;

#[cfg(test)]
mod sample_tests;

#[cfg(test)]
mod scoring_tests;
