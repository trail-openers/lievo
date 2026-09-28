#!/usr/bin/env bash
# Fake apfel binary for the context-overflow offline test (issue #649).
#
# Simulates apfel's observed context-overflow failure: it prints the exact
# overflow message on stderr and exits non-zero. This lets the pipeline test
# build the same `SummarizationFailed` error the real pipeline would construct
# from a failed apfel call, and assert it classifies as `skipped_oversized` —
# without invoking a real (unavailable) apfel binary.
#
# The observed apfel overflow message (from issue #649):
#   [context overflow] Input exceeds the 4096-token context window

echo "[context overflow] Input exceeds the 4096-token context window" >&2
exit 1
