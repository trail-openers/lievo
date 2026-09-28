// TypeScript/TSX fixture for issue #701: wrapped/call-expression exports in
// .ts/.tsx must yield CodeUnits named after the binding, matching the .js
// behavior (acceptance criterion 1 covers both .js and .ts/.tsx).
//
// NOTE: the repo's TypeScript parser is the tree-sitter JavaScript grammar
// (see TreeSitterExtractor::get_language — .ts/.tsx use the JavaScript
// grammar), so constructs that need the TypeScript grammar (type-annotated
// function parameters, generic call arguments) are NOT used here; the
// untyped wrapped forms below are the valid, real-world subset.

import { sumNumbers } from './js_sample';

// (a) Tagged template on a member expression
export const StyledBadge = styled.span`font-size: 12px;`;

// (b) Tagged template on a call — styled(X)`...`
export const StyledLink = styled(StyledBadge)`color: green`;

// (c) forwardRef with an inline arrow
export const ForwardLink = forwardRef((props, ref) => {
    return props.href;
});

// (d) observer with an inline arrow
export const ObservedList = observer((props) => {
    const total = sumNumbers(props.items);
    return total;
});

// (e) memo with an inline function expression (not arrow)
export const MemoList = memo(function (props) {
    return props.items.length;
});

// (f) HOC wrap (with[A-Z] pattern) over an identifier
export const ThemedBadge = withTheme(StyledBadge);

// (g) Nested wrappers — exactly one unit named after the binding
export const NestedLink = memo(forwardRef((props, ref) => {
    return props.deep;
}));

// ---------------------------------------------------------------------------
// Separate-export form (issue #791): declaration and export are separate
// statements. Must yield one unit per binding, named after the binding.
// ---------------------------------------------------------------------------

// (h) observer + export default (separate statement)
const ArticlePanel = observer((props) => {
    const total = sumNumbers(props.items);
    return total;
});
export default ArticlePanel;

// (i) memo + export
const Counter = memo((props) => {
    return props.count;
});
export { Counter };

// (j) forwardRef + export
const SearchBar = forwardRef((props, ref) => {
    return props.query;
});
export { SearchBar };

// (k) Nested memo(forwardRef(...)) in separate form — exactly ONE unit
const Modal = memo(forwardRef((props, ref) => {
    return props.visible;
}));
export { Modal };

// (l) Wrapped component assigned but never exported — still extracted
const InternalWidget = observer((props) => {
    return props.label;
});

// (m) Negative: an inline callback assigned to a variable is NOT a wrapped
// component. `.map` is a member expression, not a bare-identifier callee, so
// the member-callee narrowing of the inline-fn rule excludes it (issue #791).
// The inner arrow IS extracted as a plain-arrow unit under the #677 rule
// (documented caveat; see the PR for the follow-up).
const doubled = [1, 2, 3].map((n) => {
    return n * 2;
});

// (n) Negative: the wrapped-component gate (issue #791) is module-level
// only, so a function-local inline event handler is NOT extracted as a
// wrapped component. (The handler arrow does still appear as a plain-arrow
// unit under the pre-existing #677 rule, which is out of scope here — see
// the PR for the follow-up.)
function Host(props) {
    const clickHandler = () => {
        return props.onClick;
    };
    return clickHandler;
}
export { Host };
