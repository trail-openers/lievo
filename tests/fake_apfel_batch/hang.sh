#!/usr/bin/env bash
# Fake apfel binary for the batch-timeout salvage integration test (issue #673).
#
# Simulates a wedged LLM binary: it blocks (sleep 999) before writing its marker
# file, so if the pipeline kills it on the timeout deadline, the marker is never
# written. If the child were merely abandoned (not killed), the marker would
# appear and the test would fail.
#
# The marker path is read from FAKE_APFEL_MARKER (set by the test) so the test
# can verify the child was killed, not abandoned.

if [ -n "${FAKE_APFEL_MARKER:-}" ]; then
    touch "$FAKE_APFEL_MARKER"
fi

sleep 999
