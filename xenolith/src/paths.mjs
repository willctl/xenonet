// Where Xenolith keeps its own files (not the wiki): the platform's standard
// locations, never a dot folder in the home directory.
//
//                Windows                        Linux (XDG)                       macOS
//   config       %APPDATA%\Xenolith            $XDG_CONFIG_HOME/xenolith       ~/Library/Application Support/Xenolith
//   data         %LOCALAPPDATA%\Xenolith       $XDG_DATA_HOME/xenolith         ~/Library/Application Support/Xenolith
//   state        <data>\state                   $XDG_STATE_HOME/xenolith        <data>/state
//   logs         <data>\logs                    <state>/logs                      ~/Library/Logs/Xenolith
//   cache        <data>                         $XDG_CACHE_HOME/xenolith        ~/Library/Caches/Xenolith
//
// On macOS an XDG_* variable that is set wins for its kind. Overrides, strongest first:
// XENOLITH_{CONFIG,DATA,STATE,LOG,CACHE}_DIR for one kind (the service passes these, since it
// runs as another account with another profile), then XENOLITH_HOME for "everything in this one
// folder" (portable mode: the tests and ui:preview; the layout of the old ~/.xenolith).

import os from 'node:os';
import path from 'node:path';

/** The folder every version before 1.3 used, which `install-local` moves away from. */
export const legacyHome = (home = os.homedir()) => path.join(home, '.xenolith');

const OVERRIDES = { configDir: 'XENOLITH_CONFIG_DIR', dataDir: 'XENOLITH_DATA_DIR', stateDir: 'XENOLITH_STATE_DIR', logDir: 'XENOLITH_LOG_DIR', cacheDir: 'XENOLITH_CACHE_DIR' };

// The XDG spec says to ignore a relative path in these variables.
const xdg = (env, name, fallback) => (env[name] && path.isAbsolute(env[name]) ? env[name] : fallback);

function standardDirs(env, platform, home) {
  if (platform === 'win32') {
    const roaming = env.APPDATA || path.join(home, 'AppData', 'Roaming');
    const local = env.LOCALAPPDATA || path.join(home, 'AppData', 'Local');
    const data = path.join(local, 'Xenolith');
    return { configDir: path.join(roaming, 'Xenolith'), dataDir: data, stateDir: path.join(data, 'state'), logDir: path.join(data, 'logs'), cacheDir: data };
  }
  if (platform === 'darwin') {
    const support = path.join(home, 'Library', 'Application Support', 'Xenolith');
    const state = env.XDG_STATE_HOME && path.isAbsolute(env.XDG_STATE_HOME) ? path.join(env.XDG_STATE_HOME, 'xenolith') : path.join(support, 'state');
    return {
      configDir: env.XDG_CONFIG_HOME && path.isAbsolute(env.XDG_CONFIG_HOME) ? path.join(env.XDG_CONFIG_HOME, 'xenolith') : support,
      dataDir: env.XDG_DATA_HOME && path.isAbsolute(env.XDG_DATA_HOME) ? path.join(env.XDG_DATA_HOME, 'xenolith') : support,
      stateDir: state,
      logDir: env.XDG_STATE_HOME && path.isAbsolute(env.XDG_STATE_HOME) ? path.join(state, 'logs') : path.join(home, 'Library', 'Logs', 'Xenolith'),
      cacheDir: env.XDG_CACHE_HOME && path.isAbsolute(env.XDG_CACHE_HOME) ? path.join(env.XDG_CACHE_HOME, 'xenolith') : path.join(home, 'Library', 'Caches', 'Xenolith'),
    };
  }
  const state = path.join(xdg(env, 'XDG_STATE_HOME', path.join(home, '.local', 'state')), 'xenolith');
  return {
    configDir: path.join(xdg(env, 'XDG_CONFIG_HOME', path.join(home, '.config')), 'xenolith'),
    dataDir: path.join(xdg(env, 'XDG_DATA_HOME', path.join(home, '.local', 'share')), 'xenolith'),
    stateDir: state,
    logDir: path.join(state, 'logs'),
    cacheDir: path.join(xdg(env, 'XDG_CACHE_HOME', path.join(home, '.cache')), 'xenolith'),
  };
}

/**
 * Every location Xenolith uses besides the wiki itself. `mode` is "portable" when
 * XENOLITH_HOME puts everything in one folder, otherwise "standard".
 */
export function appPaths({ env = process.env, platform = process.platform, home = os.homedir() } = {}) {
  const portable = env.XENOLITH_HOME ? path.resolve(env.XENOLITH_HOME) : null;
  const dirs = portable
    ? { configDir: portable, dataDir: portable, stateDir: portable, logDir: path.join(portable, 'logs'), cacheDir: portable }
    : standardDirs(env, platform, home);
  for (const [k, name] of Object.entries(OVERRIDES)) if (env[name]) dirs[k] = path.resolve(env[name]);
  return {
    mode: portable ? 'portable' : 'standard',
    ...dirs,
    configFile: path.join(dirs.configDir, 'config.json'),
    installState: path.join(dirs.stateDir, 'install-state.json'),
    migrationJournal: path.join(dirs.stateDir, 'migration.json'),
    // In logs/, not state/: the service may write logs/ and nothing else of ours.
    httpSessions: path.join(dirs.logDir, 'http-sessions.json'),
    runtimeDir: path.join(dirs.dataDir, 'runtime'),
    serviceDir: path.join(dirs.dataDir, 'service'),
    trayDir: path.join(dirs.dataDir, 'tray'),
    marketDir: path.join(dirs.dataDir, 'marketplace'),
    curatorDir: path.join(dirs.dataDir, 'curator'),
    curatorCodexHome: path.join(dirs.dataDir, 'curator', 'codex-home'),
    pasteFile: path.join(dirs.dataDir, 'paste-into-app-settings.md'),
    uiProfile: path.join(dirs.cacheDir, 'ui-profile'),
    legacyHome: legacyHome(home),
  };
}

/** Environment that makes a child process (another server, the read-only Ask server) use the same locations. */
export function pathsEnv(p = appPaths()) {
  return Object.fromEntries(Object.entries(OVERRIDES).map(([k, name]) => [name, p[k]]));
}
