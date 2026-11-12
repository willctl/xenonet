const REVIEW_LIMITS={"files":512,"fileBytes":262144,"totalBytes":4194304,"depth":24}; const excluded=function excludedPath(name) {
  return name.split("/").some((part) => /^(?:\.git|node_modules|\.ssh|\.aws|\.azure|\.gnupg|\.venv|venv|__pycache__|\.cache|\.xenonet)$/i.test(part)
    || /^\.env(?:\..*)?$/i.test(part) || /^(?:\.npmrc|\.pypirc|\.netrc|\.git-credentials|credentials(?:\.json)?|secrets?(?:\..*)?|id_rsa|id_ed25519)$/i.test(part)
    || /\.(?:pem|key|p12|pfx|keystore)$/i.test(part));
}; const portable=function reviewPath(name) {
  if (typeof name !== "string" || !name || name.length > 240 || !/^[a-zA-Z0-9_.@ +\/-]+$/.test(name)) return false;
  const parts = name.split("/");
  return parts.length <= REVIEW_LIMITS.depth && parts.every((part) => part && part !== "." && part !== ".." && !/[. ]$/.test(part) && !/^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(part));
}; process.stdout.write(JSON.stringify((function collectText(root, limits, excluded, portable) {
  const fs = require("node:fs"), path = require("node:path");
  const files = [], omissions = [], problems = [], stamps = [], directories = []; let bytes = 0, entries = 0;
  const bad = (name, reason) => { if (problems.length < 30) problems.push({ path: name.slice(0, 240), reason }); };
  const mark = (s) => `${s.dev}:${s.ino}:${s.size}:${s.mtimeMs}:${s.ctimeMs}`;
  const base = fs.realpathSync(root);
  if (base !== root || !fs.lstatSync(root).isDirectory()) throw new Error("Review root is not a real directory.");
  function walk(dir, prefix = "", depth = 0) {
    if (depth > limits.depth) { bad(prefix, "Directory depth exceeded"); return; }
    const names = fs.readdirSync(dir).sort(); directories.push([dir, JSON.stringify(names)]);
    for (const name of names) {
      if (++entries > 2048) throw new Error("Review entry limit exceeded.");
      const rel = prefix ? `${prefix}/${name}` : name, file = path.join(dir, name);
      if (excluded(rel)) { if (omissions.length >= limits.files) throw new Error("Too many exclusions."); omissions.push({ path: rel.slice(0, 240), reason: "Outside review scope" }); continue; }
      if (!portable(rel)) { bad(rel, "Unsupported portable path"); continue; }
      const st = fs.lstatSync(file);
      if (st.isSymbolicLink()) { bad(rel, "Links cannot be retrieved"); continue; }
      if (st.isDirectory()) { walk(file, rel, depth + 1); continue; }
      if (!st.isFile() || st.size > limits.fileBytes) { bad(rel, "Non-regular or oversized file"); continue; }
      bytes += st.size;
      if (bytes > limits.totalBytes || files.length >= limits.files) throw new Error("Review size limit exceeded.");
      const fd = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
      let data;
      try {
        const actual = fs.realpathSync(`/proc/self/fd/${fd}`);
        if (!actual.startsWith(`${base}/`) || mark(fs.fstatSync(fd)) !== mark(st)) throw new Error("File changed or escaped review root.");
        data = Buffer.alloc(st.size); let read = 0;
        while (read < data.length) { const n = fs.readSync(fd, data, read, data.length - read, read); if (!n) break; read += n; }
        if (read !== st.size || mark(fs.fstatSync(fd)) !== mark(st)) throw new Error("File changed during retrieval.");
      } finally { fs.closeSync(fd); }
      stamps.push([file, mark(st)]);
      try { new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(data); if (data.includes(0)) throw new Error("binary"); }
      catch { bad(rel, "Binary or invalid UTF-8 file"); continue; }
      files.push({ path: rel, content: data.toString("base64") });
    }
  }
  walk(root);
  for (const [file, before] of stamps) if (mark(fs.lstatSync(file)) !== before) throw new Error("Files changed during retrieval.");
  for (const [dir, before] of directories) if (JSON.stringify(fs.readdirSync(dir).sort()) !== before) throw new Error("Files changed during retrieval.");
  return { files, omissions, problems };
})("/workspace/project", REVIEW_LIMITS, excluded, portable)));