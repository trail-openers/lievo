// JavaScript fixture for issue #701's independent zero-unit-file defect:
// a file with ZERO extractable function units must still get a file entity.
//
// Every declaration here is a non-wrapped binding (issue #791: the trigger
// is the assignment to a variable binding whose initializer is a wrapped form
// — styled template, forwardRef/memo/observer/connect/with[X] call, or a call
// taking an inline function as its first argument). These bindings do NOT
// match that pattern: object literals, plain identifiers, and arrays are not
// wrapped forms — so the extractor produces no code units from this file.
// Yet grouping must still seed a file entity from the scanned-path list.
const config = { theme: 'dark', version: 1 };
const timeoutId = setTimeout;
const items = [1, 2, 3];
// `timeoutId` above is deliberately a bare identifier (no initializer): a
// `setTimeout(() => {}, 100)` call form WOULD be extracted under the
// widened #791 assignment-keyed rule (call taking an inline function), so
// the initializer was removed rather than left as a false positive.
