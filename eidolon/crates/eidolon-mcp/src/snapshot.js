/* eidolon:snapshot */
function () {
  // Reads the page and writes nothing into it: no attributes, no classes,
  // no styles. It returns the elements it found as DOM nodes (the browser
  // names them with a sharedId) and a tree that points at them by index.
  var MAX_ITEMS = 1500;
  var MAX_TEXT = 160;
  var nodes = [];
  var budget = MAX_ITEMS;
  var truncated = false;
  var SKIP = { script: 1, style: 1, noscript: 1, template: 1, head: 1, svg: 1, canvas: 1, iframe: 1, object: 1, embed: 1 };
  var ACTIVE = { button: 1, link: 1, textbox: 1, searchbox: 1, checkbox: 1, radio: 1, combobox: 1, slider: 1, file: 1, menuitem: 1, tab: 1, "switch": 1, option: 1, spinbutton: 1, menuitemcheckbox: 1, menuitemradio: 1 };
  var GROUPS = { nav: 1, main: 1, header: 1, footer: 1, aside: 1, form: 1, dialog: 1, ul: 1, ol: 1, table: 1, fieldset: 1 };
  var GROUP_ROLES = { dialog: 1, navigation: 1, main: 1, banner: 1, form: 1, list: 1, table: 1, alert: 1, status: 1, menu: 1, toolbar: 1 };

  function clip(s) {
    s = String(s == null ? "" : s).replace(/\s+/g, " ").trim();
    return s.length > MAX_TEXT ? s.slice(0, MAX_TEXT) + "..." : s;
  }
  function shown(el) {
    if (el.getAttribute("aria-hidden") === "true") return false;
    var st = getComputedStyle(el);
    return st.display !== "none" && st.visibility !== "hidden";
  }
  function roleOf(el) {
    var tag = el.tagName.toLowerCase();
    var explicit = el.getAttribute("role");
    if (explicit) return explicit.split(" ")[0];
    if (tag === "a") return el.hasAttribute("href") ? "link" : null;
    if (tag === "button" || tag === "summary") return "button";
    if (tag === "select") return "combobox";
    if (tag === "textarea") return "textbox";
    if (tag === "img") return "img";
    if (/^h[1-6]$/.test(tag)) return "heading";
    if (tag === "input") {
      var t = (el.type || "text").toLowerCase();
      if (t === "hidden") return null;
      if (t === "button" || t === "submit" || t === "reset" || t === "image") return "button";
      if (t === "checkbox") return "checkbox";
      if (t === "radio") return "radio";
      if (t === "range") return "slider";
      if (t === "file") return "file";
      if (t === "number") return "spinbutton";
      if (t === "search") return "searchbox";
      return "textbox";
    }
    if (el.isContentEditable && el.getAttribute("contenteditable") !== null) return "textbox";
    return null;
  }
  function nameOf(el, role) {
    var tag = el.tagName.toLowerCase();
    var label = el.getAttribute("aria-label");
    if (label && clip(label)) return clip(label);
    var by = el.getAttribute("aria-labelledby");
    if (by) {
      var doc = el.ownerDocument;
      var joined = by.split(/\s+/).map(function (id) { var n = doc.getElementById(id); return n ? n.textContent : ""; }).join(" ");
      if (clip(joined)) return clip(joined);
    }
    if (el.labels && el.labels.length) {
      var l = clip(Array.prototype.map.call(el.labels, function (x) { return x.textContent; }).join(" "));
      if (l) return l;
    }
    if (tag === "img") return clip(el.getAttribute("alt") || el.getAttribute("title"));
    if (tag === "input") {
      var t = (el.type || "text").toLowerCase();
      if (t === "button" || t === "submit" || t === "reset") return clip(el.value);
      return clip(el.getAttribute("title") || el.getAttribute("placeholder"));
    }
    if (tag === "textarea" || tag === "select") return clip(el.getAttribute("title") || el.getAttribute("placeholder"));
    if (role === "textbox" || role === "searchbox") return clip(el.getAttribute("title") || el.getAttribute("placeholder"));
    return clip(el.innerText || el.textContent) || clip(el.getAttribute("title"));
  }
  function valueOf(el) {
    var tag = el.tagName.toLowerCase();
    if (tag === "input") {
      var t = (el.type || "text").toLowerCase();
      // A password is never copied out of the page.
      if (t === "password") return el.value ? "(hidden)" : "";
      if (t === "checkbox" || t === "radio" || t === "button" || t === "submit" || t === "reset" || t === "file") return "";
      return clip(el.value);
    }
    if (tag === "textarea") return clip(el.value);
    if (tag === "select") {
      var o = el.selectedOptions && el.selectedOptions[0];
      return o ? clip(o.textContent) : "";
    }
    if (el.isContentEditable) return clip(el.textContent);
    return "";
  }
  function attrsOf(el) {
    var out = {};
    var names = ["id", "name", "type", "href", "placeholder", "aria-expanded", "aria-checked", "aria-selected"];
    for (var i = 0; i < names.length; i++) {
      var v = el.getAttribute(names[i]);
      if (v !== null && v !== "") out[names[i]] = clip(v);
    }
    if (el.checked === true) out.checked = "true";
    if (el.disabled === true) out.disabled = "true";
    if (el.required === true) out.required = "true";
    if (el.tagName.toLowerCase() === "input" && (el.type || "").toLowerCase() === "file" && el.multiple) out.multiple = "true";
    return out;
  }
  function describe(el, role) {
    var item = { n: nodes.length, role: role, tag: el.tagName.toLowerCase(), name: nameOf(el, role) };
    nodes.push(el);
    var value = valueOf(el);
    if (value) item.value = value;
    var attrs = attrsOf(el);
    for (var k in attrs) { item.attrs = attrs; break; }
    if (role === "heading") {
      var m = /^h([1-6])$/.exec(item.tag);
      if (m) item.level = Number(m[1]);
    }
    return item;
  }
  function walk(parent, out) {
    var kids = parent.childNodes;
    for (var i = 0; i < kids.length; i++) {
      if (budget <= 0) { truncated = true; return; }
      var child = kids[i];
      if (child.nodeType === 3) {
        var text = clip(child.nodeValue);
        if (text) { out.push({ t: text }); budget--; }
        continue;
      }
      if (child.nodeType !== 1) continue;
      var tag = child.tagName.toLowerCase();
      if (SKIP[tag]) continue;
      // File inputs are often hidden behind a styled button and still take files.
      var isFile = tag === "input" && (child.type || "").toLowerCase() === "file";
      if (!isFile && !shown(child)) continue;
      var role = roleOf(child);
      if (role && (ACTIVE[role] || role === "heading" || role === "img")) {
        out.push(describe(child, role));
        budget--;
        continue;
      }
      var inner = [];
      walk(child, inner);
      if (child.shadowRoot) walk(child.shadowRoot, inner);
      if (inner.length && (GROUPS[tag] || (role && GROUP_ROLES[role]))) {
        out.push({ c: role || tag, name: clip(child.getAttribute("aria-label")), children: inner });
      } else {
        for (var j = 0; j < inner.length; j++) out.push(inner[j]);
      }
    }
  }
  var items = [];
  walk(document.body || document.documentElement, items);
  return { tree: JSON.stringify({ url: location.href, title: document.title, truncated: truncated, items: items }), nodes: nodes };
}
