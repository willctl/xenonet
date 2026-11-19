/* eidolon:prepare-fill */
function (el, value, phase) {
  // phase "probe": a select takes the option whose value or text matches;
  // anything else is left alone. phase "focus": the field gets focus with its
  // content selected, so the typing that follows replaces it.
  if (phase === "probe") {
    if (el.tagName.toLowerCase() !== "select") return "typing";
    var options = Array.prototype.slice.call(el.options);
    var hit = options.filter(function (o) { return o.value === value; })[0] || options.filter(function (o) { return o.textContent.trim() === value; })[0];
    if (!hit) return "no-option";
    el.value = hit.value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return "selected";
  }
  el.focus();
  if (typeof el.select === "function") {
    el.select();
  } else if (el.isContentEditable) {
    var range = document.createRange();
    range.selectNodeContents(el);
    var sel = getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }
  return "typing";
}
