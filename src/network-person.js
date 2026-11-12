// Your devices and this machine's services, in the Network panel
// (network.js; docs/design/authorization.md):
//
//   - Your devices: each device your identity holds (network/state/v1's
//     person.devices), how it asks you (Touch ID, or something weaker),
//     which is this machine's, and Revoke for another one, as for a lost
//     machine: POST /api/net/devices/revoke, from the window, which asks
//     you once. Whether your identity's root key is sealed here, or in a
//     file, which the doctor warns of too.
//   - Services: what this machine publishes to the network (GET
//     /api/net/svc), with Take off (POST /api/net/svc/unpublish, which asks
//     nothing), and Publish a service: its name, address, the paths other
//     machines may reach and which machines may call. Publishing widens
//     what the network can reach, so it asks you (POST
//     /api/net/svc/publish, from the window).
//
// Markup and the lenient reads only: the panel holds the state and answers
// the buttons.
import { escapeHtml as h } from "./format.js";

const KIND_WORDS = { biometric: "asks with Touch ID", weaker: "asks with Windows Hello, polkit or a passphrase, which is weaker" };

const list = (v) => (Array.isArray(v) ? v : []);
const word = (v) => typeof v === "string" && v.length > 0;

// The person as network/state/v1 has them, read leniently: null before an
// identity is set up here.
export function personOf(state) {
  const p = state?.person;
  if (!p || typeof p !== "object" || !word(p.id)) return null;
  return {
    id: p.id,
    owner: !!p.owner,
    device: word(p.device) ? p.device : null,
    kind: p.kind === "biometric" || p.kind === "weaker" ? p.kind : null,
    acts: !!p.acts,
    rootSealed: typeof p.rootSealed === "boolean" ? p.rootSealed : null,
    devices: list(p.devices)
      .filter((d) => d && word(d.id))
      .map((d) => ({ id: d.id, label: word(d.label) ? d.label : d.id, kind: d.kind === "biometric" ? "biometric" : "weaker", active: d.active !== false, here: !!d.here })),
  };
}

// The services as GET /api/net/svc gives them, read leniently.
export const servicesOf = (raw) =>
  list(raw?.services)
    .filter((x) => x && word(x.name))
    .map((x) => ({ name: x.name, target: word(x.target) ? x.target : "", paths: list(x.paths).filter(word), hosts: list(x.hosts).filter(word) }));

// A device's line: how it asks, and whether it can sign.
export function deviceLine(d) {
  if (!d.active) return "revoked: nothing it signs counts";
  return KIND_WORDS[d.kind] ?? KIND_WORDS.weaker;
}

// Your devices, with Revoke on another active one, and its confirmation
// when `confirm` names it.
export function devicesHtml(person, confirm = null) {
  if (!person) return "";
  const row = (d) => {
    const tags = `${d.here ? `<span class="ac-badge on">this machine</span>` : ""}${d.active ? "" : `<span class="ac-badge">revoked</span>`}`;
    const can = person.acts && d.active && !d.here;
    const acts = !can
      ? ""
      : confirm === d.id
        ? `<span class="net-ask">Revoke ${h(d.label)}'s device?</span><button class="act" data-net-keep>Keep it</button><button class="act primary ac-danger" data-net-revoke-yes="${h(d.id)}" data-yours>Revoke</button>`
        : `<button class="act" data-net-revoke="${h(d.id)}" title="For a lost machine: nothing this device signs counts again">Revoke…</button>`;
    return `<li class="ac-row" data-net-device="${h(d.id)}"><span class="ac-dot"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">${h(d.label)}</span>${tags}</div><div class="ac-what">${h(deviceLine(d))}</div></div><div class="ac-acts">${acts}</div></li>`;
  };
  const root =
    person.rootSealed === false
      ? `<p class="net-warn" data-net-root>Your identity's root key is in an owner-only file on this machine, since it can't seal it to its device key. A program running as you could read it. Run only programs you trust under this account.</p>`
      : "";
  return `<div class="panel-title sub"><span>Your devices</span><span class="tick">${person.devices.length}</span></div>
      <ol class="ac-list" data-net-devices>${person.devices.map(row).join("")}</ol>
      ${root}
      <p class="pj-note">Each machine you act from holds a device key of yours. Revoke one when its machine is lost: every machine in the network then refuses what it signs. It asks you once. Remove that machine from the network too.</p>`;
}

// What this machine publishes, with Take off, and Publish a service.
export function servicesHtml(services) {
  const who = (x) => (x.hosts.length ? `for ${x.hosts.join(", ")}` : "for every machine");
  const rows = services
    .map(
      (x) =>
        `<li class="ac-row" data-net-service="${h(x.name)}"><span class="ac-dot"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">${h(x.name)}</span></div><div class="ac-what">${h(`${x.target} ${x.paths.join(" ")} · ${who(x)}`)}</div></div><div class="ac-acts"><button class="act" data-net-unpublish="${h(x.name)}" title="Other machines can't reach it from now on">Take off</button></div></li>`,
    )
    .join("");
  return `<div class="panel-title sub"><span>Services</span><span class="tick">${services.length}</span></div>
      ${services.length ? `<ol class="ac-list" data-net-services>${rows}</ol>` : `<p class="pj-note" data-net-services>This machine publishes no services.</p>`}
      <div class="m-foot"><span class="grow"></span><button class="act" data-net-go="publish">Publish a service…</button></div>`;
}

// The Publish a service form, with what was typed (`draft`).
export function publishHtml(draft = {}, error = null) {
  const field = (k, attr, value, placeholder) => `<label class="ac-field"><span class="ac-k">${k}</span><input class="ac-input" ${attr} value="${h(value ?? "")}" placeholder="${h(placeholder)}" spellcheck="false" autocomplete="off"></label>`;
  return `<p class="ac-lead">Publish a service</p>
      <div class="ac-form">
        ${field("Its name", "data-net-svc-name", draft.name, "notes")}
        ${field("Its address", "data-net-svc-target", draft.target, "http://127.0.0.1:8080")}
        ${field("Paths", "data-net-svc-paths", draft.paths, "/api/v1/")}
        ${field("Machines", "data-net-svc-hosts", draft.hosts, "every machine")}
      </div>
      <p class="pj-note">A program on another machine in the network reaches it through that machine's own XenoNet, at /api/svc/&lt;this machine&gt;/&lt;name&gt;/&lt;path&gt;, and only under the paths you list, each starting and ending with /. Leave Machines empty for every machine, or name them, separated by spaces. A service with no sign-in of its own is open to every program on the machines that may call it. Publishing asks you once.</p>
      <p class="ac-error bad" data-net-error ${error ? "" : "hidden"}>${h(error ?? "")}</p>
      <div class="m-foot"><button class="act" data-net-back>Cancel</button><span class="grow"></span><button class="act primary" data-net-publish data-yours>Publish</button></div>`;
}

const words = (text) =>
  String(text ?? "")
    .split(/[\s,]+/)
    .map((w) => w.trim())
    .filter(Boolean);

// POST /api/net/svc/publish's body from the form's fields.
export const publishBody = ({ name, target, paths, hosts }) => ({ name: String(name ?? "").trim(), target: String(target ?? "").trim(), paths: words(paths), hosts: words(hosts) });
