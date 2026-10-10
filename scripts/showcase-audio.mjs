// Capture XenoNet's own post-compressor mix, without recording other apps or a mic.
// This runs only in the isolated showcase page through its development channel.
import fs from "node:fs";

export async function prepareAudio(s) {
  await s.eval(`return (() => {
    if (window.__xenonet.ambient.debug.ctxState !== null)
      throw new Error('Quit the showcase instance before recording so its audio graph is fresh.');
    const connect = AudioNode.prototype.connect;
    AudioNode.prototype.connect = function(destination, ...args) {
      const result = connect.call(this, destination, ...args);
      if (destination === this.context.destination) {
        const tap = this.context.createMediaStreamDestination();
        connect.call(this, tap);
        AudioNode.prototype.connect = connect;
        window.__showcaseAudio = {
          start() {
            const chunks = [];
            const recorder = new MediaRecorder(tap.stream, {mimeType: 'audio/webm;codecs=opus', audioBitsPerSecond: 192000});
            recorder.ondataavailable = e => chunks.push(e.data);
            this.finished = new Promise((resolve, reject) => {
              recorder.onerror = e => reject(new Error(e.error?.message || 'Audio capture failed'));
              recorder.onstop = () => {
                const reader = new FileReader();
                reader.onerror = () => reject(new Error('Could not read recorded audio'));
                reader.onload = () => resolve(reader.result.split(',')[1]);
                reader.readAsDataURL(new Blob(chunks, {type: recorder.mimeType}));
              };
            });
            this.recorder = recorder;
            recorder.start();
            return Date.now();
          },
          async stop() { this.recorder.stop(); return this.finished; }
        };
      }
      return result;
    };
    return true;
  })()`);
  await s.driver("command", "mute");
  const state = await s.eval("return { ...window.__xenonet.ambient.debug, capture: !!window.__showcaseAudio }");
  if (state.state !== "playing" || !state.capture) throw new Error("XenoNet's audio did not start for the recording.");
}

export const startAudio = (s) => s.eval("return window.__showcaseAudio.start()");

export async function stopAudio(s, file) {
  const encoded = await s.eval("return await window.__showcaseAudio.stop()");
  fs.writeFileSync(file, Buffer.from(encoded, "base64"));
}
