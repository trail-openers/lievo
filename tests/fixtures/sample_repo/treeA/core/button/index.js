// Fixture for issue #706: the correct sibling-tree import target for
// treeA/save/index.js's "../core/button" specifier. Same basename
// (index.js) as treeB/search/index.js, so a wrong-edge resolver that
// matches by suffix/basename instead of normalised path could confuse
// the two — the fix must land the edge here, not on the treeB decoy.
export default function Button() {
    return 'button';
}
