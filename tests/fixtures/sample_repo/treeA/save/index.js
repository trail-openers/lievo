// Fixture for issue #706: a relative import must resolve by normalised path
// ONLY, never by basename/suffix matching across unrelated top-level trees.
//
// This file imports "../core/button" (resolves within treeA, to
// treeA/core/button/index.js) and separately CALLS `handleSaving`, a
// function defined only in treeB/search/index.js — a same-basename decoy
// tree. The call site must never manufacture a file->file imports edge
// (call evidence is function-level, not a module dependency), and the
// resolved import must land on the treeA target, not the treeB decoy.
import Button from '../core/button';

export function save() {
    handleSaving();
    return Button;
}
