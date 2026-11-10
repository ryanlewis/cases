// The hooks case.ts gets in respond.test.js: hooks-stand-in.js's inside a
// component the test mounts, so that its state and effects can be driven,
// and a still drawing elsewhere, as render-to-string draws the components
// in the tree it gives: their state is the initial one, and no effect runs.
import * as standIn from "./hooks-stand-in.js";

// `hook` from the stand-in, or `still` when no mounted component is drawing.
function either(hook, still) {
  return (...args) => {
    try {
      return hook(...args);
    } catch (e) {
      if (e.message !== "a hook called outside a component") throw e;
      return still(...args);
    }
  };
}

export const useState = either(standIn.useState, (initial) => [typeof initial === "function" ? initial() : initial, () => {}]);
export const useRef = either(standIn.useRef, (initial) => ({ current: initial }));
export const useEffect = either(standIn.useEffect, () => {});
// A layout effect needs a page, which the test has not: none runs.
export const useLayoutEffect = () => {};
