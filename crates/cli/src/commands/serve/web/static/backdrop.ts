// A dialog's backdrop, shared by the dialogs: closes the dialog on a click on
// its backdrop, a press and release on the dialog itself, whose content fills
// it, so that a drag that starts inside the form and ends outside, or outside and
// ends inside, does not close it. The click alone cannot tell: the browser
// sends it to the nearest element both the press and the release were in, the
// dialog. Whether a press started on the backdrop is kept with the dialog, not
// the handlers, so that a redraw between the press and the click, which gives
// new ones, does not lose it.
const pressed = new WeakSet<EventTarget>();

// The handlers for a dialog's backdrop: `close` is what a click on it does,
// by default closing the dialog.
export function backdrop(close: (dialog: HTMLDialogElement) => void = (d) => d.close()) {
  return {
    onPointerDown: (e: PointerEvent) => {
      if (e.target === e.currentTarget) pressed.add(e.currentTarget!);
      else pressed.delete(e.currentTarget!);
    },
    // Released off the backdrop, it is not a click on it.
    onPointerUp: (e: PointerEvent) => {
      if (e.target !== e.currentTarget) pressed.delete(e.currentTarget!);
    },
    // A press the browser cancels (a touch that turns into a scroll) gives
    // no click, and is not one.
    onPointerCancel: (e: PointerEvent) => pressed.delete(e.currentTarget!),
    onClick: (e: MouseEvent) => {
      if (e.target === e.currentTarget && pressed.has(e.currentTarget!)) close(e.currentTarget as HTMLDialogElement);
      pressed.delete(e.currentTarget!);
    },
  };
}
