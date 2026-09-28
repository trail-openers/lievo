// JavaScript fixture for issue #701: a file whose exports are ALL
// wrapped/call-expression forms. Before #701 this file produced zero code
// units and no file entity; after the fix each exported binding yields one
// CodeUnit named after the binding, and the file itself gets a file entity.

import { sumNumbers } from './js_sample';

// (a) Tagged template on a member expression — styled.div`...`
export const StyledButton = styled.div`color: red;`;

// (b) Tagged template on a call — styled(X)`...`
export const StyledIcon = styled(StyledButton)`color: blue;`;

// (c) forwardRef with an inline arrow function
export const ForwardButton = forwardRef((props, ref) => {
    console.log(props.label);
    ref.current.click();
    return props.label;
});

// (d) observer (react MobX) with an inline arrow
export const ObservedCard = observer((props) => {
    const total = sumNumbers(props.items);
    return total;
});

// (e) memo with an inline arrow
export const MemoButton = memo((props) => {
    return props.count;
});

// (f) HOC wrap (with[A-Z] pattern) over an identifier
export const ThemedButton = withTheme(StyledButton);

// (g) Nested wrappers must yield exactly ONE unit named after the binding
export const NestedButton = memo(forwardRef((props, ref) => {
    return props.nested;
}));

// ---------------------------------------------------------------------------
// Separate-export form (issue #791): declaration and export are separate
// statements. The trigger is the assignment to a variable binding, not the
// export — these must yield exactly one unit each, named after the binding.
// ---------------------------------------------------------------------------

// (h) observer + export default (separate statement)
const ArticleCarousel = observer((props) => {
    const total = sumNumbers(props.items);
    return total;
});
export default ArticleCarousel;

// (i) React.memo + export default
const TimerWidget = memo((props) => {
    return props.seconds;
});
export { TimerWidget };

// (j) React.forwardRef + export default
const InputField = forwardRef((props, ref) => {
    return props.placeholder;
});
export { InputField };

// (k) Nested memo(forwardRef(...)) in separate form — exactly ONE unit
const ComplexDialog = memo(forwardRef((props, ref) => {
    return props.title;
}));
export { ComplexDialog };

// (l) Wrapped component assigned but never exported — still extracted
const InternalHelper = observer((props) => {
    return props.value;
});

// (m) Negative: an inline callback assigned to a variable is NOT a wrapped
// component. `.map` is a member expression, not a bare-identifier callee, so
// the member-callee narrowing of the inline-fn rule excludes it (issue #791).
// The inner arrow IS extracted as a plain-arrow unit under the #677 rule
// (documented caveat; see the PR for the follow-up).
const processedItems = [1, 2, 3].map((item) => {
    return item * 2;
});

// (n) Negative: the wrapped-component gate (issue #791) is module-level
// only, so a function-local inline event handler is NOT extracted as a
// wrapped component. (The handler arrow does still appear as a plain-arrow
// unit under the pre-existing #677 rule, which is out of scope here — see
// the PR for the follow-up.)
function HandlerHost(props) {
    const clickHandler = () => {
        return props.onClick;
    };
    return clickHandler;
}
export { HandlerHost };
