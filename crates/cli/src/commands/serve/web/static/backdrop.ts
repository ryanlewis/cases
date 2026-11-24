// A dialog's backdrop, shared by the dialogs: closes the dialog on a click on
// its backdrop, a press and release on the dialog itself, whose content fills
// it, so that a drag that starts inside the form and ends outside, or outside and
// ends inside, does not close it. The click alone cannot tell: the browser
// sends it to the nearest element both the press and the release were in, the
// dialog. Whether a press started on the backdrop is kept with the dialog, not
// the handlers, so that a redraw between the press and the click, which gives
// new ones, does not lose it.
//
// A touch or a pen is captured by what it pressed: its pointerup goes to the
// dialog wherever it is let go, so where that was takes a hit test. A press is
// that of one pointer, a primary one: a second finger, down or let go
// elsewhere, does not change it.
const pressed = new WeakMap<EventTarget, number>();

// The handlers for a dialog's backdrop: `close` is what a click on it does,
// by default closing the dialog.
export function backdrop(close: (dialog: HTMLDialogElement) => void = (d) => d.close()) {
  return {
    onPointerDown: (e: PointerEvent) => {
      if (e.isPrimary === false) return;
      if (e.target === e.currentTarget) pressed.set(e.currentTarget!, e.pointerId);
      else pressed.delete(e.currentTarget!);
    },
    // Released off the backdrop, it is not a click on it.
    onPointerUp: (e: PointerEvent) => {
      if (pressed.get(e.currentTarget!) !== e.pointerId) return;
      const captured = e.pointerType === "touch" || e.pointerType === "pen";
      const at = captured ? document.elementFromPoint(e.clientX, e.clientY) : e.target;
      if (at !== e.currentTarget) pressed.delete(e.currentTarget!);
    },
    // A press the browser cancels (a touch that turns into a scroll) gives
    // no click, and is not one.
    onPointerCancel: (e: PointerEvent) => {
      if (pressed.get(e.currentTarget!) === e.pointerId) pressed.delete(e.currentTarget!);
    },
    onClick: (e: MouseEvent) => {
      if (e.target === e.currentTarget && pressed.has(e.currentTarget!)) close(e.currentTarget as HTMLDialogElement);
      pressed.delete(e.currentTarget!);
    },
  };
}
