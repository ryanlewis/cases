// A dialog's backdrop (static/backdrop.ts) under touch and pen, and with more
// than one pointer down. The dialogs' own tests pin it for a mouse. A touch or
// a pen is captured by what it pressed, so its pointerup goes to the dialog
// wherever it is let go, and only a hit test says where that was.
import { test } from "node:test";
import assert from "node:assert/strict";
import { backdrop } from "../static/backdrop.ts";

// A dialog with the handlers on it, and `under`, what is at the point a
// pointer is let go.
function dialog(t) {
  const el = {
    closed: 0,
    close() {
      this.closed++;
    },
  };
  const inside = {};
  const on = backdrop();
  const was = globalThis.document;
  t.after(() => {
    globalThis.document = was;
  });
  const d = {
    el,
    inside,
    under: el,
    hits: 0,
    down: (target, e = {}) => on.onPointerDown({ target, currentTarget: el, ...e }),
    up: (target, e = {}) => on.onPointerUp({ target, currentTarget: el, ...e }),
    cancel: (e = {}) => on.onPointerCancel({ target: el, currentTarget: el, ...e }),
    click: (target = el) => on.onClick({ target, currentTarget: el }),
  };
  globalThis.document = {
    elementFromPoint: () => {
      d.hits++;
      return d.under;
    },
  };
  return d;
}

for (const pointerType of ["touch", "pen"]) {
  const finger = { pointerType, pointerId: 1, clientX: 10, clientY: 20 };

  test(`a ${pointerType} pressed on the backdrop that slides into the form does not close the dialog`, (t) => {
    const d = dialog(t);
    d.down(d.el, finger);
    d.under = d.inside;
    // Captured, its release goes to the dialog, wherever it was.
    d.up(d.el, finger);
    d.click();
    assert.equal(d.el.closed, 0);
  });

  test(`a ${pointerType} pressed and let go on the backdrop closes the dialog`, (t) => {
    const d = dialog(t);
    d.down(d.el, finger);
    d.up(d.el, finger);
    d.click();
    assert.equal(d.el.closed, 1);
  });

  test(`a ${pointerType} let go where nothing is does not close the dialog`, (t) => {
    const d = dialog(t);
    d.down(d.el, finger);
    d.under = null;
    d.up(d.el, finger);
    d.click();
    assert.equal(d.el.closed, 0);
  });
}

test("a mouse, which nothing captures, is not hit tested", (t) => {
  const d = dialog(t);
  const mouse = { pointerType: "mouse", pointerId: 1, clientX: 10, clientY: 20 };
  d.down(d.el, mouse);
  d.under = d.inside;
  d.up(d.el, mouse);
  d.click();
  assert.equal(d.hits, 0);
  assert.equal(d.el.closed, 1);
});

const one = { pointerType: "touch", pointerId: 1 };
const two = { pointerType: "touch", pointerId: 2, isPrimary: false };

test("another finger pressed and let go inside the form leaves the press on the backdrop", (t) => {
  const d = dialog(t);
  d.down(d.el, one);
  d.down(d.inside, two);
  d.up(d.inside, two);
  d.up(d.el, one);
  d.click();
  assert.equal(d.el.closed, 1);
});

test("another finger on the backdrop does not take the press over from the first", (t) => {
  const d = dialog(t);
  d.down(d.el, one);
  d.down(d.el, two);
  // The first slides into the form: its release is still the press's.
  d.under = d.inside;
  d.up(d.el, one);
  d.click();
  assert.equal(d.el.closed, 0);
});

test("a press inside the form replaces a press on the backdrop no click followed, from another pointer", (t) => {
  const d = dialog(t);
  d.down(d.el, { pointerType: "touch", pointerId: 7, isPrimary: true });
  d.up(d.el, { pointerType: "touch", pointerId: 7, isPrimary: true });
  const mouse = { pointerType: "mouse", pointerId: 1, isPrimary: true };
  d.down(d.inside, mouse);
  d.up(d.el, mouse);
  d.click();
  assert.equal(d.el.closed, 0);
});

test("another finger cancelled leaves the press on the backdrop", (t) => {
  const d = dialog(t);
  d.down(d.el, one);
  d.cancel(two);
  d.up(d.el, one);
  d.click();
  assert.equal(d.el.closed, 1);
});

test("the finger that pressed the backdrop, cancelled, is no click on it", (t) => {
  const d = dialog(t);
  d.down(d.el, one);
  d.cancel(one);
  d.click();
  assert.equal(d.el.closed, 0);
});
