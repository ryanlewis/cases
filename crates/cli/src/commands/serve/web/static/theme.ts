// The inbox's display preferences (IN-10), kept in this browser's
// localStorage. The look is the set of tokens style.css draws with.
//
// Loaded as a plain script from <head>, before the stylesheet, so the stored
// choices are set as data attributes on <html> before the page paints. The
// first value of each option is its default and leaves the attribute off.
// The SPA's options dialog reads and writes them through window.casesPrefs.
//
// The face's default is the look's: its --prose token, mono in the mono
// look and sans in daylight and keel. So a face chosen in the dialog is
// stored even when it is the look's own, and reads the same in every look;
// only reset goes back to the look's default. FACES says which choice that
// default shows as.
//
// The key hints' default, auto, is the pointer's: shown where it is fine,
// as a mouse's, and hidden where it is coarse, as a finger's on a phone,
// by the stylesheet's (pointer: coarse). Show or hide chosen in the dialog
// is stored and set either way, as a face is.
//
// With approve on, every row of an open approval starts with approve
// chosen (case.ts); nothing is sent until the form is.
//
// Wrap is how every code block draws a long line: scrolled sideways, the
// default, or wrapped. The button on each block (code.ts) flips it for all
// of them, and the dialog sets it too.
(function () {
  "use strict";

  var OPTIONS: import("./types.js").Option[] = [
    { name: "look", values: ["daylight", "mono", "keel"] },
    { name: "theme", values: ["system", "light", "dark"] },
    { name: "face", values: ["look", "mono", "sans", "serif"] },
    { name: "size", values: ["medium", "small", "large"] },
    { name: "links", values: ["new", "same"] },
    { name: "notify", values: ["off", "blocking", "all"] },
    { name: "hints", values: ["auto", "show", "hide"] },
    { name: "approve", values: ["off", "on"] },
    { name: "wrap", values: ["scroll", "wrap"] },
  ];
  var root = document.documentElement;
  // The face each look reads in when none is chosen, as its --prose; mono
  // for a look not named.
  var FACES: Record<string, string> = { mono: "mono", daylight: "sans", keel: "sans" };

  function option(name: string): import("./types.js").Option | null {
    for (var i = 0; i < OPTIONS.length; i++) {
      if (OPTIONS[i]!.name === name) return OPTIONS[i]!;
    }
    return null;
  }
  // The value stored for option `name`, or its default.
  function stored(name: string): string {
    var o = option(name)!;
    var v: string | null = null;
    try { v = localStorage.getItem("cases." + name); } catch (e) {}
    return o.values.indexOf(v!) > 0 ? v! : o.values[0]!;
  }
  // Whether the pointer is coarse, so that the key hints are hidden when
  // none is chosen.
  function coarse(): boolean {
    return !!window.matchMedia && window.matchMedia("(pointer: coarse)").matches;
  }
  // The value of option `name` as the dialog shows it: the face the look
  // reads in when no face is stored, and whether the key hints show when
  // that is not chosen.
  function read(name: string): string {
    var v = stored(name);
    if (name === "face" && v === "look") return FACES[stored("look")] || "mono";
    if (name === "hints" && v === "auto") return coarse() ? "hide" : "show";
    return v;
  }
  function write(name: string, value: string): void {
    var o = option(name)!;
    try {
      if (value === o.values[0]) localStorage.removeItem("cases." + name);
      else localStorage.setItem("cases." + name, value);
    } catch (e) {}
  }
  function apply(): void {
    OPTIONS.forEach(function (o) {
      var v = stored(o.name);
      if (v === o.values[0]) delete root.dataset[o.name];
      else root.dataset[o.name] = v;
    });
    // Each block's wrap button says whether long lines wrap.
    if (document.querySelectorAll) {
      var wrapped = String(stored("wrap") === "wrap");
      document.querySelectorAll(".wrap-toggle").forEach(function (b) { b.setAttribute("aria-pressed", wrapped); });
    }
  }
  apply();

  // Tabs already open, or restored from the back-forward cache, take up a
  // change made in another tab without a reload.
  window.addEventListener("storage", apply);
  window.addEventListener("pageshow", function (e) { if (e.persisted) apply(); });

  // Links to other sites open in a new tab. With links=same, a click drops
  // the target first; listening on the document covers every link drawn.
  document.addEventListener("click", function (e) {
    if (root.dataset.links !== "same" || !(e.target as Element).closest) return;
    var a = (e.target as Element).closest('a[target="_blank"]');
    if (a) a.removeAttribute("target");
  }, true);

  // A code block's wrap button flips wrap for every block.
  document.addEventListener("click", function (e) {
    if (!(e.target as Element).closest || !(e.target as Element).closest(".wrap-toggle")) return;
    write("wrap", stored("wrap") === "wrap" ? "scroll" : "wrap");
    apply();
  });

  window.casesPrefs = { options: OPTIONS, read: read, write: write, apply: apply };
})();
