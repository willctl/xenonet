// Time-lapse replay of the last 24 hours: sessions appear as they were
// created, sparks fly as tools ran, arcs form, and actors (actors.js) beam
// into what they acted on and react as they act, in about a minute. Can
// record the canvas to a WebM clip.
import { THEME, onTheme, withAlpha } from "./theme.js";

const SPEEDS = [
  { label: "60 s", seconds: 60 },
  { label: "2 min", seconds: 120 },
  { label: "20 s", seconds: 20 },
];
const MAX_EVENTS_PER_FRAME = 30;

export function createReplay({ scene, hud, sfx, onEnter, onExit, getCanvas }) {
  const bar = document.getElementById("replay");
  let events = [];
  // The actors' events over the same span, and how far they've played.
  let actorEvents = [];
  let actorPlayed = 0;
  let start = 0, end = 0, cursor = 0, played = 0;
  let playing = false;
  let speed = 0;
  let raf = 0;
  let lastFrame = 0;
  let recorder = null;
  let chunks = [];
  let active = false;
  let lastVisibleKey = "";
  // A film's own length (see film.js), instead of the speed button's.
  let fixedSeconds = null;

  bar.innerHTML = `
    <button class="rp-btn" data-rp-play title="play/pause (space)">❚❚</button>
    <button class="rp-btn" data-rp-speed title="Replay length">60 s</button>
    <div class="rp-track"><canvas class="rp-hist"></canvas><div class="rp-head"></div></div>
    <div class="rp-time">--:--</div>
    <button class="rp-btn rec" data-rp-rec title="Record a clip">● rec</button>
    <button class="rp-btn" data-rp-exit title="Leave replay · Esc">✕</button>`;
  const hist = bar.querySelector(".rp-hist");
  const head = bar.querySelector(".rp-head");
  const timeEl = bar.querySelector(".rp-time");
  const playBtn = bar.querySelector("[data-rp-play]");
  const speedBtn = bar.querySelector("[data-rp-speed]");
  const recBtn = bar.querySelector("[data-rp-rec]");
  const track = bar.querySelector(".rp-track");

  function drawHistogram() {
    const r = hist.getBoundingClientRect();
    hist.width = Math.max(1, Math.round(r.width * devicePixelRatio));
    hist.height = Math.max(1, Math.round(r.height * devicePixelRatio));
    const g = hist.getContext("2d");
    const bins = new Array(120).fill(0);
    for (const e of events) if (e.kind === "tool") bins[Math.min(119, Math.floor(((e.ts - start) / (end - start)) * 120))]++;
    const max = Math.max(1, ...bins);
    const w = hist.width / bins.length;
    bins.forEach((v, i) => {
      const bh = (v / max) * hist.height * 0.9;
      const grd = g.createLinearGradient(0, hist.height - bh, 0, hist.height);
      grd.addColorStop(0, withAlpha(THEME.colors.accent, 0.9));
      grd.addColorStop(1, withAlpha(THEME.colors.accent, 0.15));
      g.fillStyle = grd;
      g.fillRect(i * w + 0.5, hist.height - bh, Math.max(1, w - 1), bh);
    });
  }
  onTheme(() => active && drawHistogram());

  // Sessions exist from the moment they were created.
  function applyVisibility() {
    const vis = new Set();
    for (const n of scene.nodes()) {
      if (n.kind !== "session") vis.add(n.id);
      else if ((n.createdAt ?? 0) <= cursor) vis.add(n.id);
    }
    const key = [...vis].length;
    if (key !== lastVisibleKey) {
      lastVisibleKey = key;
      scene.setVisible(vis);
    }
  }

  function fmt(ts) {
    return new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function frame(now) {
    raf = requestAnimationFrame(frame);
    const dt = Math.min(0.1, (now - lastFrame) / 1000);
    lastFrame = now;
    if (playing) {
      const span = end - start;
      const next = Math.min(end, cursor + (span / (fixedSeconds ?? SPEEDS[speed].seconds)) * dt);
      let fired = 0;
      while (played < events.length && events[played].ts <= next) {
        const ev = events[played++];
        if (fired++ < MAX_EVENTS_PER_FRAME) {
          scene.activity(ev);
          if (ev.kind !== "message") hud.activity(ev);
        }
      }
      while (actorPlayed < actorEvents.length && actorEvents[actorPlayed].at <= next) scene.actorEvent(actorEvents[actorPlayed++]);
      cursor = next;
      applyVisibility();
      if (cursor >= end) {
        playing = false;
        playBtn.textContent = "▶";
        if (recorder) stopRecording();
      }
    }
    head.style.left = `${((cursor - start) / Math.max(1, end - start)) * 100}%`;
    timeEl.textContent = fmt(cursor);
    hud.setClock?.(`REPLAY ${fmt(cursor)}`);
  }

  function seek(ts) {
    cursor = Math.max(start, Math.min(end, ts));
    played = events.findIndex((e) => e.ts > cursor);
    if (played < 0) played = events.length;
    actorPlayed = actorEvents.findIndex((e) => e.at > cursor);
    if (actorPlayed < 0) actorPlayed = actorEvents.length;
    applyVisibility();
  }

  // What actors did over the day: the history, where XenoNet keeps one
  // (GET /api/actors/events), else each actor's recent events.
  async function loadActorEvents(since) {
    const res = await fetch(`/api/actors/events?since=${since}`).catch(() => null);
    const body = res?.ok ? await res.json().catch(() => null) : null;
    const list = body?.events ?? scene.actors().flatMap((a) => a.snapshot.recent ?? []);
    return list.filter((e) => e?.actor && Number.isFinite(e.at)).sort((a, b) => a.at - b.at);
  }

  async function enter({ seconds = null } = {}) {
    if (active) return;
    fixedSeconds = seconds;
    const res = await fetch("/api/history").then((r) => r.json());
    events = res.events;
    if (!events.length) return hud.toast?.("Nothing to replay yet");
    active = true;
    start = Math.min(events[0].ts, ...scene.nodes().filter((n) => n.kind === "session" && n.createdAt > res.now - 24 * 3600e3).map((n) => n.createdAt));
    end = res.now;
    actorEvents = await loadActorEvents(start);
    // The actors' beams follow the replay's clock while it plays.
    scene.actorLayer.setClock(() => cursor);
    bar.hidden = false;
    document.body.classList.add("replaying");
    onEnter?.();
    sfx("rewind");
    lastFrame = performance.now();
    seek(start);
    requestAnimationFrame(drawHistogram);
    playing = true;
    playBtn.textContent = "❚❚";
    raf = requestAnimationFrame(frame);
  }

  function exit() {
    if (!active) return;
    if (recorder) stopRecording();
    active = false;
    playing = false;
    fixedSeconds = null;
    cancelAnimationFrame(raf);
    scene.actorLayer.setClock(null);
    bar.hidden = true;
    document.body.classList.remove("replaying");
    hud.setClock?.(null);
    lastVisibleKey = "";
    onExit?.();
    sfx("close");
  }

  function startRecording() {
    const canvas = getCanvas();
    const stream = canvas.captureStream(30);
    const type = ["video/webm;codecs=vp9", "video/webm;codecs=vp8", "video/webm"].find((t) => MediaRecorder.isTypeSupported(t));
    recorder = new MediaRecorder(stream, { mimeType: type, videoBitsPerSecond: 12e6 });
    chunks = [];
    recorder.ondataavailable = (e) => e.data.size && chunks.push(e.data);
    recorder.onstop = () => {
      const blob = new Blob(chunks, { type: "video/webm" });
      lastClip = blob;
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = `xenonet-replay-${new Date().toLocaleDateString("en-CA")}.webm`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 60000);
      hud.toast?.(`Saved a ${(blob.size / 1e6).toFixed(1)} MB clip`);
      sfx("success");
    };
    recorder.start(500);
    recBtn.classList.add("on");
    recBtn.textContent = "■ stop";
    seek(start);
    playing = true;
    playBtn.textContent = "❚❚";
  }

  let lastClip = null;
  function stopRecording() {
    recorder?.stop();
    recorder = null;
    recBtn.classList.remove("on");
    recBtn.textContent = "● rec";
  }

  bar.addEventListener("click", (e) => {
    if (e.target.closest("[data-rp-play]")) {
      if (cursor >= end) seek(start);
      playing = !playing;
      playBtn.textContent = playing ? "❚❚" : "▶";
      sfx("toggle");
    } else if (e.target.closest("[data-rp-speed]")) {
      speed = (speed + 1) % SPEEDS.length;
      speedBtn.textContent = SPEEDS[speed].label;
      sfx("toggle");
    } else if (e.target.closest("[data-rp-rec]")) {
      recorder ? stopRecording() : startRecording();
      sfx("select");
    } else if (e.target.closest("[data-rp-exit]")) exit();
  });
  track.addEventListener("pointerdown", (e) => {
    const move = (ev) => {
      const r = track.getBoundingClientRect();
      seek(start + ((ev.clientX - r.left) / r.width) * (end - start));
    };
    move(e);
    track.setPointerCapture(e.pointerId);
    track.onpointermove = move;
    track.onpointerup = () => (track.onpointermove = null);
  });
  addEventListener("keydown", (e) => {
    if (!active || e.target.matches?.("input, textarea, .xterm-helper-textarea")) return;
    if (e.key === "Escape") exit();
    if (e.key === " ") {
      e.preventDefault();
      bar.querySelector("[data-rp-play]").click();
    }
  });

  return {
    enter,
    exit,
    get active() {
      return active;
    },
    get debug() {
      return { events: events.length, start, end, cursor, played, playing, recording: !!recorder, clipBytes: lastClip?.size ?? 0 };
    },
    seekFraction: (f) => seek(start + f * (end - start)),
    get cursor() {
      return cursor;
    },
    get range() {
      return { start, end };
    },
    get playing() {
      return playing;
    },
    setSpeedSeconds(s) {
      SPEEDS[speed].seconds = s;
    },
  };
}
