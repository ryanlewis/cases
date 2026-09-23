// Preferences for the cases inbox, kept in this browser's localStorage, and
// the desktop notifications they turn on.
//
// Loaded blocking from <head>, so the stored choices are set as data
// attributes on <html> before the stylesheet paints. The first value of each
// option is its default and leaves the attribute off. To add an option, add a
// row here, CSS keyed on its attribute, and a radio group in the options
// dialog in layout.html.
//
// With notifications on and allowed, the page asks /notifications every five
// seconds for cases that landed on the human and shows each as a desktop
// notification. The last id shown is kept per browser, with the server's boot
// id, so tabs share it and a restarted serve starts it again.
//
// A page that shows the inbox or a case follows the store through the event
// stream it names in data-events on the body; see the end of this file.
(function () {
  "use strict";

  var OPTIONS = [
    { name: "theme", values: ["system", "light", "dark"] },
    { name: "face", values: ["mono", "sans", "serif"] },
    { name: "size", values: ["medium", "small", "large"] },
    { name: "links", values: ["new", "same"] },
    { name: "notify", values: ["off", "blocking", "all"] },
  ];
  var NOTIFY = OPTIONS[OPTIONS.length - 1];
  var root = document.documentElement;

  function read(o) {
    var v = null;
    try { v = localStorage.getItem("cases." + o.name); } catch (e) {}
    return o.values.indexOf(v) > 0 ? v : o.values[0];
  }
  function write(o, v) {
    try {
      if (v === o.values[0]) localStorage.removeItem("cases." + o.name);
      else localStorage.setItem("cases." + o.name, v);
    } catch (e) {}
  }
  function apply(o) {
    var v = read(o);
    if (v === o.values[0]) delete root.dataset[o.name];
    else root.dataset[o.name] = v;
  }
  OPTIONS.forEach(apply);

  // Pages already open in other tabs, or restored from the back-forward cache
  // after a change in the options dialog, pick up the stored choices without a reload.
  function applyAll() { OPTIONS.forEach(apply); }
  window.addEventListener("storage", applyAll);
  window.addEventListener("pageshow", function (e) { if (e.persisted) applyAll(); });

  // The server marks external links target=_blank. With links=same, a click
  // drops the target first; listening on the document covers swapped-in
  // content.
  document.addEventListener("click", function (e) {
    if (root.dataset.links !== "same" || !e.target.closest) return;
    var a = e.target.closest('a[target="_blank"]');
    if (a) a.removeAttribute("target");
  }, true);

  // Desktop notifications. Nothing is fetched while they are off or not
  // allowed, and the kept id is dropped then, so turning them on later does
  // not replay what came in meanwhile.
  var CURSOR = "cases.notify-cursor";
  var POLL = 5000;
  var cursor = null; // this page's copy, for when localStorage is unavailable

  function permission() {
    return "Notification" in window ? Notification.permission : "unsupported";
  }
  function readCursor() {
    try {
      var c = JSON.parse(localStorage.getItem(CURSOR));
      if (c && typeof c.boot === "string" && typeof c.id === "number") return c;
    } catch (e) {}
    return cursor;
  }
  function writeCursor(c) {
    cursor = c;
    try {
      if (c) localStorage.setItem(CURSOR, JSON.stringify(c));
      else localStorage.removeItem(CURSOR);
    } catch (e) {}
  }
  function notify(item) {
    if (read(NOTIFY) === "blocking" && !(item.case && item.case.urgency === "blocking")) return;
    // The tag is the same in every tab, so the desktop shows the item once.
    var n = new Notification(item.title, { body: item.body, tag: item.tag });
    n.onclick = function () {
      window.focus();
      location.href = item.url;
      n.close();
    };
  }
  function check() {
    if (read(NOTIFY) === "off" || permission() !== "granted") {
      writeCursor(null);
      setTimeout(check, POLL);
      return;
    }
    var have = readCursor();
    fetch("/notifications" + (have ? "?after=" + have.id : ""), { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (page) {
        if (!page) return false;
        // Read again: another tab may have shown these while this one waited.
        var c = readCursor();
        // A page with no kept id starts at the latest item and shows nothing.
        if (!c) {
          writeCursor({ boot: page.boot, id: page.latest });
          return false;
        }
        // serve restarted. Its feed only holds cases that came in after it
        // started, so start the new boot from 0 and ask again now: the page
        // just fetched asked after an id from the old boot.
        if (c.boot !== page.boot) {
          writeCursor({ boot: page.boot, id: 0 });
          return true;
        }
        // This page was asked after an id the kept one has since moved
        // before (another tab started the new boot), so it can miss items.
        if (have && (have.boot !== c.boot || have.id > c.id)) return true;
        if (c.id >= page.latest) return false;
        page.items.forEach(function (item) { if (item.id > c.id) notify(item); });
        writeCursor({ boot: page.boot, id: page.latest });
        return false;
      })
      .catch(function () { return false; })
      .then(function (again) { setTimeout(check, again ? 0 : POLL); });
  }
  check();

  // Textareas grow with their content. CSS field-sizing does it where the
  // browser has it; elsewhere the height follows scrollHeight on every input
  // and once for anything prefilled, such as a form sent back with a 409.
  var grows = false;
  try { grows = CSS.supports("field-sizing", "content"); } catch (e) {}
  function grow(el) {
    if (grows || !el || el.tagName !== "TEXTAREA") return;
    el.style.height = "auto";
    el.style.height = el.scrollHeight + el.offsetHeight - el.clientHeight + "px";
  }
  document.addEventListener("input", function (e) { grow(e.target); });

  document.addEventListener("DOMContentLoaded", function () {
    Array.prototype.forEach.call(document.getElementsByTagName("textarea"), grow);
    var form = document.getElementById("options");
    if (!form) return;
    var status = document.getElementById("notify-status");
    var allow = document.getElementById("notify-allow");
    var STATUS = {
      granted: "allowed in this browser.",
      denied: "blocked in this browser. allow them in its site settings for this address.",
      "default": "the browser asks before the first one.",
      unsupported: "this browser cannot show them.",
    };
    function show() {
      OPTIONS.forEach(function (o) {
        var input = form.querySelector('input[name="' + o.name + '"][value="' + read(o) + '"]');
        if (input) input.checked = true;
      });
      var p = permission();
      if (status) status.textContent = STATUS[p];
      if (allow) allow.hidden = p !== "default";
    }
    // The browser only asks from a click: the allow button, or choosing to
    // turn notifications on. If the answer is no, they go back to off.
    function ask() {
      Notification.requestPermission().then(function (p) {
        if (p !== "granted") write(NOTIFY, "off");
        show();
      });
    }
    form.addEventListener("change", function (e) {
      OPTIONS.forEach(function (o) {
        if (e.target.name === o.name) {
          write(o, e.target.value);
          apply(o);
        }
      });
      if (e.target.name === NOTIFY.name && e.target.value !== "off" && permission() === "default") ask();
    });
    if (allow) allow.addEventListener("click", ask);
    // A dialog closes itself on Escape and on its close button (a
    // method=dialog form); a click that lands on the dialog element and not
    // its form is a click on the backdrop. The press must start there too, so
    // a drag that begins inside the form and ends outside does not close it.
    // This holds for the options dialog and the keys dialog keys.js opens.
    Array.prototype.forEach.call(document.querySelectorAll("dialog.options"), function (d) {
      var downOnBackdrop = false;
      d.addEventListener("pointerdown", function (e) {
        downOnBackdrop = e.target === d;
      });
      d.addEventListener("click", function (e) {
        if (e.target === d && downOnBackdrop) d.close();
        downOnBackdrop = false;
      });
    });
    var dialog = document.getElementById("options-dialog");
    var open = document.getElementById("options-open");
    if (dialog && open && dialog.showModal) {
      open.addEventListener("click", function () {
        show();
        dialog.showModal();
      });
    }
    var reset = document.getElementById("options-reset");
    if (reset) {
      reset.addEventListener("click", function () {
        OPTIONS.forEach(function (o) {
          write(o, o.values[0]);
          apply(o);
        });
        show();
      });
    }
    show();
  });

  // The event stream sends the store's version each time it changes, and
  // each new version fires store-changed on the body, which the page's htmx
  // regions refresh on. The browser reconnects a dropped stream by itself, as
  // when serve restarts, and says which version it had.
  //
  // A browser allows six connections to one address, shared by all its tabs,
  // and a stream holds one for as long as it is open: six streams would leave
  // none for the pages' own requests. So only a tab that is shown holds a
  // stream. It passes each version on to the other tabs over a broadcast
  // channel, so a tab in the background still follows the store, and a tab
  // shown again opens its stream from the last version it had, which serve
  // answers at once if the store has moved since.
  document.addEventListener("DOMContentLoaded", function () {
    var url = document.body.dataset.events;
    if (!url || !window.EventSource) return;
    var at = new URL(url, location.href);
    var version = at.searchParams.get("after") || "";
    var channel = window.BroadcastChannel ? new BroadcastChannel("cases-store") : null;
    var stream = null;
    function refresh() {
      document.body.dispatchEvent(new CustomEvent("store-changed"));
    }
    function seen(v) {
      if (v === version) return;
      version = v;
      refresh();
    }
    function open() {
      if (stream || (channel && document.hidden)) return;
      at.searchParams.set("after", version);
      stream = new EventSource(at.pathname + at.search);
      var dropped = false;
      stream.onerror = function () { dropped = true; };
      // A refresh set off just before the stream dropped may have failed
      // with it, as when serve restarts, and the version it was for counts
      // as seen: ask again once the stream is back.
      stream.onopen = function () {
        if (dropped) refresh();
        dropped = false;
      };
      stream.onmessage = function (e) {
        if (channel) channel.postMessage(e.data);
        seen(e.data);
      };
    }
    function close() {
      if (stream) stream.close();
      stream = null;
    }
    if (channel) {
      channel.onmessage = function (e) { seen(e.data); };
      document.addEventListener("visibilitychange", function () {
        if (document.hidden) close();
        else open();
      });
    }
    open();

    // A form sent leaves the page, but the write it makes reaches the stream
    // first, and a refresh that sets off could send the browser back to the
    // case instead of where the post redirects. So once a form goes (not the
    // options dialog's), the page's refreshes are dropped, one already on its
    // way included. A page still here a while later, as when the post was
    // opened in another tab or stopped, or shown again from the browser's
    // back-forward cache, follows the store again.
    var leaving = null;
    function stay() {
      clearTimeout(leaving);
      leaving = null;
      refresh();
    }
    document.addEventListener("submit", function (e) {
      if (e.target.method === "dialog") return;
      clearTimeout(leaving);
      leaving = setTimeout(stay, 10000);
    });
    ["htmx:beforeRequest", "htmx:beforeOnLoad"].forEach(function (name) {
      document.addEventListener(name, function (e) {
        if (leaving) e.preventDefault();
      });
    });
    window.addEventListener("pageshow", function (e) {
      if (!e.persisted) return;
      // The browser may have closed the stream while the page was away.
      if (stream && stream.readyState === EventSource.CLOSED) close();
      open();
      if (leaving) stay();
    });
  });

  // A refresh of the case view keeps the form's fields as the human left
  // them (hx-preserve). A browser without moveBefore takes a field out of
  // the page to move it, which loses its focus and caret, so they are put
  // back.
  var kept = null;
  document.addEventListener("htmx:beforeSwap", function () {
    var el = document.activeElement;
    kept = el && el.hasAttribute && el.hasAttribute("hx-preserve") ?
      { el: el, start: el.selectionStart, end: el.selectionEnd, top: el.scrollTop } : null;
  });
  document.addEventListener("htmx:afterSwap", function () {
    var k = kept;
    kept = null;
    if (!k || document.activeElement === k.el || !document.contains(k.el)) return;
    k.el.focus({ preventScroll: true });
    try { k.el.setSelectionRange(k.start, k.end); } catch (e) {}
    k.el.scrollTop = k.top;
  });

  // The tally fragment carries the tab icon's link out of band. Chromium does
  // not load the icon of a link put in place of another, which is what the
  // swap would do, but does when the link's href changes, so the new href is
  // copied onto the link the page has. Safari loads a page's icon once, when
  // the page loads, and follows neither.
  document.addEventListener("htmx:oobBeforeSwap", function (e) {
    var d = e.detail;
    if (!d.target || d.target.id !== "favicon" || !d.fragment.querySelector) return;
    var next = d.fragment.querySelector("#favicon");
    if (!next) return;
    d.shouldSwap = false;
    var href = next.getAttribute("href");
    if (href && d.target.getAttribute("href") !== href) d.target.setAttribute("href", href);
  });
})();
