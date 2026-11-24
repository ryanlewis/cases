// What app.ts imports from preact and preact/hooks, in the app tests
// (app-page.js gives app.ts this module in their place). The hooks are
// hooks-stand-in.js's, with a layout effect run as an effect is: the stand-in
// draws nothing, so there is no paint for it to come before. `render` puts
// nothing in a page either: it keeps what app.ts's start() asked it to
// draw, from which the tests take App and the build the page says it is.

export { useEffect, useEffect as useLayoutEffect, useRef, useState } from "./hooks-stand-in.js";

export const rendered = [];

export function render(vnode, parent) {
  rendered.push({ vnode, parent });
}
