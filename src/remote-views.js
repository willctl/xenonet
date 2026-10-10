// The remote connections' views and buttons in the Actors panel
// (actors-panel.js): pairing a remote actor, what would leave, paired,
// revoking, and the tunnel XenoNet runs for a connection. The markup is
// remote-panel.js's; this keeps a pairing's choices and answers the
// buttons. It's apart from remote-panel.js, which the page loads at the
// start for createRemoteConnections, so that it loads with the panel.
import { newDraft, pairHtml, readPairForm, pairBody, previewHtml, pairedHtml, revokeHtml, tunnelHtml } from "./remote-panel.js";

// The panel lends `ui`: the view it shows (view()), and how to move to
// another (go), draw it again (render) or only its list (drawList), its
// sounds and toasts, and act, which makes a call and says how it went.
export function createRemoteViews(remote, ui) {
  // A pairing's choices, kept between its form and its preview.
  let pairing = newDraft();
  let pairChoices = null;

  // A remote view's markup (pair, preview, paired, unpair or tunnel);
  // `actors()` are the actors beside the map, which a pairing can join.
  function html(v, actors) {
    if (v.name === "pair") return pairHtml(pairing, { products: remote.view().products ?? [], actors: actors(), ...(pairChoices ?? {}) });
    if (v.name === "preview") return previewHtml(v.preview, pairing);
    if (v.name === "paired") return pairedHtml(v.connection);
    if (v.name === "unpair") return revokeHtml(remote.of(v.grant) ?? { label: v.grant, actor_id: "", grant_id: v.grant });
    return tunnelHtml(remote.of(v.grant) ?? { label: v.grant, actor_id: "" }, remote.tunnelOf(v.grant), { ...(remote.view().tunnels ?? {}), replacing: !!v.replacing });
  }

  // The pairing form, once drawn, takes the typing where it starts.
  const focus = (body) => body.querySelector(pairing.actor ? "[data-rc-label]" : "[data-rc-name]")?.focus();

  // The remote connections' buttons: whether one of them was clicked.
  async function click(t, d, body) {
    // A choice in the pairing form keeps what's typed so far.
    const choose = (patch) => {
      pairing = { ...readPairForm(body, pairing), ...patch };
      ui.sfx("toggle");
      ui.render();
      return true;
    };
    const say = (text) => (Object.assign(body.querySelector("[data-rc-error]"), { hidden: false, textContent: text }), true);
    if (d.rcPair !== undefined) {
      pairing = newDraft();
      pairChoices = await remote.places();
      ui.go({ name: "pair" });
      return true;
    }
    if (d.rcProduct !== undefined) {
      const product = remote.view().products.find((p) => (p.product ?? "custom") === d.rcProduct);
      return choose({ product: d.rcProduct, actor: "", transport: product?.transports[0]?.id ?? pairing.transport });
    }
    if (d.rcActor !== undefined) return choose({ actor: d.rcActor });
    if (d.rcTransport !== undefined) return choose({ transport: d.rcTransport });
    if (d.rcScope !== undefined) return choose({ scope: d.rcScope });
    if (d.rcVerified !== undefined) return choose({ verified: !pairing.verified });
    if (d.rcLabels !== undefined) return choose({ labels: !pairing.labels });
    if (d.rcDays !== undefined) return choose({ days: Number(d.rcDays) });
    if (d.rcLink !== undefined) return choose({ link: "unverified", identity: "" });
    if (d.rcIdentity !== undefined) return choose({ link: "owner_asserted", identity: d.rcIdentity });
    if (d.rcPlace !== undefined) {
      const had = pairing.projects.includes(d.rcPlace);
      return choose({ projects: had ? pairing.projects.filter((p) => p !== d.rcPlace) : [...pairing.projects, d.rcPlace] });
    }
    if (d.rcPreview !== undefined) {
      pairing = readPairForm(body, pairing);
      if (!pairing.label.trim()) return say("Give the connection a name");
      if (!pairing.audience.trim()) return say("Say who can call its endpoint");
      t.disabled = true;
      const r = await remote.preview({ projects: pairing.projects, labels: pairing.labels });
      if (!r.ok) {
        t.disabled = false;
        ui.sfx("error");
        return say(r.error ?? "XenoNet couldn't show what would leave");
      }
      ui.go({ name: "preview", preview: r.data });
      return true;
    }
    if (d.rcEdit !== undefined) return ui.go({ name: "pair" }), true;
    if (d.rcApprove !== undefined) {
      t.disabled = true;
      const r = await remote.pair(pairBody(pairing));
      if (!r.ok) {
        ui.sfx("error");
        ui.toast(r.error ?? "That didn't work", true);
        ui.go({ name: "pair" });
        return true;
      }
      ui.sfx("success");
      pairing = newDraft();
      ui.go({ name: "paired", connection: r.data });
      return true;
    }
    if (d.rcCopy !== undefined && ui.view().connection) {
      const text = [ui.view().connection.launch.command, ...ui.view().connection.launch.args].join(" ");
      const copied = await Promise.resolve(navigator.clipboard?.writeText(text)).then(() => !!navigator.clipboard, () => false);
      ui.sfx(copied ? "select" : "error");
      ui.toast(copied ? "Command copied" : "Couldn't copy it; select it and copy instead", !copied);
      return true;
    }
    if (d.rcTunnel) {
      await remote.refresh();
      ui.go({ name: "tunnel", grant: d.rcTunnel });
      return true;
    }
    if (await onTunnel(t, d, body, say)) return true;
    if (d.rcRevoke) return ui.go({ name: "unpair", grant: d.rcRevoke }), true;
    if (d.rcRevokeYes) {
      t.disabled = true;
      const label = remote.of(d.rcRevokeYes)?.label ?? "it";
      await ui.act(() => remote.revoke(d.rcRevokeYes), `Revoked “${label}”`);
      ui.go({ name: "list" });
      return true;
    }
    if (d.rcAccess !== undefined) {
      const on = remote.view().disabled;
      t.disabled = true;
      await ui.act(() => remote.setAccess(on), on ? "Cloud access is on again" : "Cloud access is off: every remote connection stopped");
      ui.render();
      return true;
    }
    return false;
  }

  // A tunnel's buttons, in its row or its view. The key is read from its
  // field once, sent, and the field cleared; nothing here keeps it.
  async function onTunnel(t, d, body, say) {
    const grant = ui.view().name === "tunnel" ? ui.view().grant : (d.rcTstart ?? d.rcTstop);
    if (!grant) return false;
    const field = (sel) => body.querySelector(sel)?.value.trim() ?? "";
    const done = async (r, words) => {
      if (!r.ok) {
        ui.sfx("error");
        if (ui.view().name === "tunnel" && body.querySelector("[data-rc-error]")) say(r.error ?? "That didn't work");
        else ui.toast(r.error ?? "That didn't work", true);
        return true;
      }
      ui.sfx("success");
      if (words) ui.toast(words);
      if (ui.view().name === "tunnel") ui.render();
      else ui.drawList();
      return true;
    };
    if (d.rcTstart !== undefined) {
      t.disabled = true;
      return done(await remote.tunnel.start(grant), "The tunnel is starting");
    }
    if (d.rcTstop !== undefined) {
      t.disabled = true;
      return done(await remote.tunnel.stop(grant), "The tunnel stopped");
    }
    if (ui.view().name !== "tunnel") return false;
    if (d.rcTuse) {
      body.querySelector("[data-rc-tid]").value = d.rcTuse;
      ui.sfx("toggle");
      return true;
    }
    if (d.rcTsave !== undefined) {
      if (!field("[data-rc-tid]")) return say("Give the tunnel's id, from OpenAI's Tunnels page");
      t.disabled = true;
      return done(await remote.tunnel.set(grant, { tunnelId: field("[data-rc-tid]"), keyTarget: field("[data-rc-ktarget]") || null }), "Saved");
    }
    if (d.rcTauto !== undefined) {
      t.disabled = true;
      const on = !remote.tunnelOf(grant)?.autostart;
      return done(await remote.tunnel.set(grant, { autostart: on }), on ? "It starts when XenoNet starts" : "It runs only when you start it");
    }
    if (d.rcTkey !== undefined) {
      const input = body.querySelector("[data-rc-key]");
      const key = input?.value.trim() ?? "";
      input.value = "";
      if (!key) return say("Paste the tunnel's runtime key first");
      if (!remote.tunnelOf(grant)?.tunnel_id) {
        if (!field("[data-rc-tid]")) return say("Give the tunnel's id first");
        const r = await remote.tunnel.set(grant, { tunnelId: field("[data-rc-tid]"), keyTarget: field("[data-rc-ktarget]") || null });
        if (!r.ok) return done(r);
      }
      t.disabled = true;
      ui.view().replacing = false;
      return done(await remote.tunnel.key(grant, key), "The key is kept; XenoNet won't show it again");
    }
    if (d.rcTreplace !== undefined) {
      ui.view().replacing = true;
      ui.render();
      return true;
    }
    if (d.rcTforget !== undefined) {
      t.disabled = true;
      return done(await remote.tunnel.forget(grant), "XenoNet deleted the key it kept");
    }
    if (d.rcTcheck !== undefined) {
      t.disabled = true;
      const out = body.querySelector("[data-rc-tout]");
      Object.assign(out, { hidden: false, textContent: "tunnel-client doctor --explain …" });
      const r = await remote.tunnel.check(grant);
      t.disabled = false;
      if (!r.ok) return (out.hidden = true), done(r);
      out.textContent = r.data.lines.join("\n") || (r.data.ok ? "No problems found" : "It found problems, and said nothing more");
      ui.sfx(r.data.ok ? "success" : "error");
      return true;
    }
    return false;
  }

  return { html, focus, click };
}
