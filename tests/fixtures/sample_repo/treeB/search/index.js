// Fixture for issue #706: a same-basename decoy in a SIBLING top-level tree
// (treeB, vs treeA). This file defines `handleSaving`, which
// treeA/save/index.js calls (function-level call site) but does NOT import.
// A false-0-callers / wrong-edge resolver bug would either (a) create a
// bogus treeA -> treeB import edge from the call-site name match, or
// (b) miss the real treeA/save -> treeA/core/button edge by matching this
// file's basename instead. Neither must happen: this file must receive
// ZERO import edges from treeA.
export function handleSaving() {
    return 'saved';
}
