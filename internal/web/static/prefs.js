// Preferences for the cases inbox, kept in this browser's localStorage.
//
// Loaded blocking from <head>, so the stored choices are set as data
// attributes on <html> before the stylesheet paints. The first value of each
// option is its default and leaves the attribute off. To add an option, add a
// row here, CSS keyed on its attribute, and a radio group on /options.
(function () {
  "use strict";

  var OPTIONS = [
    { name: "theme", values: ["system", "light", "dark"] },
    { name: "face", values: ["mono", "sans", "serif"] },
    { name: "links", values: ["new", "same"] },
  ];
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
  // after a change on /options, pick up the stored choices without a reload.
  function applyAll() { OPTIONS.forEach(apply); }
  window.addEventListener("storage", applyAll);
  window.addEventListener("pageshow", function (e) { if (e.persisted) applyAll(); });

  // The server marks external links target=_blank. With links=same, a click
  // drops the target first; listening on the document covers polled swaps.
  document.addEventListener("click", function (e) {
    if (root.dataset.links !== "same" || !e.target.closest) return;
    var a = e.target.closest('a[target="_blank"]');
    if (a) a.removeAttribute("target");
  }, true);

  document.addEventListener("DOMContentLoaded", function () {
    var form = document.getElementById("options");
    if (!form) return;
    function show() {
      OPTIONS.forEach(function (o) {
        var input = form.querySelector('input[name="' + o.name + '"][value="' + read(o) + '"]');
        if (input) input.checked = true;
      });
    }
    form.addEventListener("change", function (e) {
      OPTIONS.forEach(function (o) {
        if (e.target.name === o.name) {
          write(o, e.target.value);
          apply(o);
        }
      });
    });
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
})();
