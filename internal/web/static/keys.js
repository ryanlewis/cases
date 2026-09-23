// Keyboard shortcuts for the cases inbox.
//
// Cmd+Enter or Ctrl+Enter sends an open case's answer, from any of its
// fields. It is the same submit as the send button: the browser checks the
// required fields first, and the form carries the revision it was drawn at,
// so a case that has moved on is refused with 409 as from a click.
//
// Other keys do nothing while a text field has the focus, a dialog is open, or
// Cmd, Ctrl or Alt is held:
//
//   1-9  choose that option, where the form has one set of choices (decision,
//        sign-off, stuck); an approval's rows have a set each, and are left to
//        the arrow keys
//   j k  open the next or previous case in the inbox list
//   ?    show these keys
//
// Listening on the document covers a case view swapped in by a refresh.
(function () {
  "use strict";

  // Set once a form is sent, so a held or repeated key does not send it
  // again and have the second post refused as stale. A page still here 10s
  // later, as when the post was stopped or failed, can send again, as
  // prefs.js follows the store again then.
  var sent = false;
  var unsend = null;
  document.addEventListener("submit", function (e) {
    if (e.target.method === "dialog") return;
    sent = true;
    clearTimeout(unsend);
    unsend = setTimeout(function () { sent = false; }, 10000);
  });
  window.addEventListener("pageshow", function (e) {
    if (!e.persisted) return;
    clearTimeout(unsend);
    sent = false;
  });

  function typing(el) {
    if (!el || !el.tagName) return false;
    if (el.isContentEditable || el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
    return el.tagName === "INPUT" && ["radio", "checkbox", "button", "submit", "reset"].indexOf(el.type) < 0;
  }

  // The open case's answer form, if the page shows one. On a narrow window
  // the inbox page hides its case, form and all, and the keys leave it alone.
  function caseForm() {
    var form = document.querySelector('#case form.respond[action$="/answer"]');
    return form && form.getClientRects().length ? form : null;
  }

  // The radios of the form's one set of choices, in page order, or none on
  // an approval, whose rows have a set each (even when it has one row).
  function choices(form) {
    if (form.querySelector("fieldset.row")) return [];
    return Array.prototype.slice.call(form.querySelectorAll('input[type="radio"]'));
  }

  function step(by) {
    var cards = Array.prototype.slice.call(document.querySelectorAll("#inbox a.card"));
    if (!cards.length) return;
    var at = cards.findIndex(function (a) { return a.getAttribute("aria-current") === "page"; });
    var next = at < 0 ? (by > 0 ? 0 : cards.length - 1) : at + by;
    if (next >= 0 && next < cards.length && next !== at) location.href = cards[next].href;
  }

  document.addEventListener("keydown", function (e) {
    if (e.defaultPrevented || e.isComposing) return;
    var dialogOpen = document.querySelector("dialog[open]");

    if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) {
      var form = caseForm();
      if (!form || dialogOpen || e.repeat || sent) return;
      // A field in some other form keeps the key to itself, and so does a
      // link, which the browser opens in a new tab on Cmd or Ctrl+Enter.
      if (e.target.form && e.target.form !== form) return;
      if (e.target.closest && e.target.closest("a[href]")) return;
      e.preventDefault();
      form.requestSubmit(form.querySelector("#respond-send"));
      return;
    }

    if (e.metaKey || e.ctrlKey || e.altKey || typing(e.target)) return;
    if (dialogOpen) return;

    if (e.key === "?") {
      var keys = document.getElementById("keys-dialog");
      if (keys && keys.showModal) {
        e.preventDefault();
        keys.showModal();
      }
      return;
    }
    if (e.key === "j" || e.key === "k") {
      e.preventDefault();
      step(e.key === "j" ? 1 : -1);
      return;
    }
    if (/^[1-9]$/.test(e.key)) {
      var f = caseForm();
      var radio = f && choices(f)[Number(e.key) - 1];
      if (!radio) return;
      e.preventDefault();
      radio.checked = true;
      radio.focus();
      // As a click would, so anything listening for the choice hears it.
      radio.dispatchEvent(new Event("change", { bubbles: true }));
    }
  });
})();
