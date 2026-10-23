// Shared by the scripts and the tests: where an installed Xenolith keeps its files, and the
// Windows helpers the tray's tests use to look at Task Scheduler and the registry. The installer
// is the Rust program (`xenolith install`, rust/crates/cli/src/install/).

import { spawnSync } from 'node:child_process';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { appPaths } from './paths.mjs';

export const REPO = path.resolve(fileURLToPath(new URL('..', import.meta.url)));
export const HOME = os.homedir();

// The standard locations, whatever XENOLITH_* overrides the calling shell has: those are for the
// programs and the tests.
const AP = appPaths({ env: Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^XENOLITH_/.test(k))) });

export const P = {
  config: AP.configFile,
  defaultWiki: path.join(HOME, 'Xenolith'),
};

export async function readJson(file) {
  return JSON.parse((await fsp.readFile(file, 'utf8')).replace(/^\uFEFF/, ''));
}

/** Runs a program and waits: {ok, code, stdout, stderr}. */
export function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { encoding: 'utf8', windowsHide: true, timeout: 120_000, ...opts });
  return { ok: r.status === 0, code: r.status, stdout: r.stdout || '', stderr: r.stderr || (r.error ? String(r.error.message) : '') };
}

// ---------------------------------------------------------------- Windows: the tray's sign-in task

const xmlText = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/**
 * A Task Scheduler definition like the one `xenolith install` registers to start the tray when you
 * sign in: as you, at your logon, normal priority, no run-time limit, one instance. `enabled: false`
 * is for tests.
 */
export function trayTaskXml({ exe, sid, enabled = true, delaySec = 10 }) {
  if (!/^S-1-\d+(-\d+)+$/.test(String(sid))) throw new Error(`not a SID: ${sid}`);
  const win = path.win32.normalize(exe);
  return [
    '<?xml version="1.0" encoding="UTF-16"?>',
    '<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">',
    '  <RegistrationInfo>',
    '    <Author>xenolith tests</Author>',
    '    <Description>Starts the Xenolith tray when you sign in.</Description>',
    '  </RegistrationInfo>',
    '  <Triggers>',
    '    <LogonTrigger>',
    '      <Enabled>true</Enabled>',
    `      <UserId>${sid}</UserId>`,
    `      <Delay>PT${delaySec}S</Delay>`,
    '    </LogonTrigger>',
    '  </Triggers>',
    '  <Principals>',
    '    <Principal id="Author">',
    `      <UserId>${sid}</UserId>`,
    '      <LogonType>InteractiveToken</LogonType>',
    '      <RunLevel>LeastPrivilege</RunLevel>',
    '    </Principal>',
    '  </Principals>',
    '  <Settings>',
    '    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>',
    '    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>',
    '    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>',
    '    <AllowHardTerminate>true</AllowHardTerminate>',
    '    <StartWhenAvailable>false</StartWhenAvailable>',
    '    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>',
    '    <IdleSettings>',
    '      <StopOnIdleEnd>false</StopOnIdleEnd>',
    '      <RestartOnIdle>false</RestartOnIdle>',
    '    </IdleSettings>',
    '    <AllowStartOnDemand>true</AllowStartOnDemand>',
    `    <Enabled>${enabled}</Enabled>`,
    '    <Hidden>false</Hidden>',
    '    <RunOnlyIfIdle>false</RunOnlyIfIdle>',
    '    <WakeToRun>false</WakeToRun>',
    '    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>',
    '    <Priority>4</Priority>',
    '  </Settings>',
    '  <Actions Context="Author">',
    '    <Exec>',
    `      <Command>"${xmlText(win)}"</Command>`,
    '      <Arguments>--from task</Arguments>',
    `      <WorkingDirectory>${xmlText(path.win32.dirname(win))}</WorkingDirectory>`,
    '    </Exec>',
    '  </Actions>',
    '</Task>',
    '',
  ].join('\n');
}

/** schtasks /xml reads UTF-16 only ("unable to switch the encoding" for UTF-8). */
export const taskXmlBytes = (xml) => Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(xml, 'utf16le')]);

/** {exists, enabled, command} of a task (command without quotes). */
export function queryTask(name) {
  const r = run('schtasks.exe', ['/query', '/tn', name, '/xml', 'ONE']);
  if (!r.ok) return { exists: false, enabled: false, command: null };
  const settings = r.stdout.match(/<Settings>[\s\S]*?<\/Settings>/)?.[0] ?? '';
  const command = r.stdout.match(/<Command>([^<]*)<\/Command>/)?.[1].replace(/&amp;/g, '&').replace(/^"|"$/g, '') ?? null;
  return { exists: true, enabled: !/<Enabled>\s*false\s*<\/Enabled>/.test(settings), command };
}

export function deleteTask(name) {
  return run('schtasks.exe', ['/delete', '/tn', name, '/f']).ok;
}

/** The data of a registry value (REG_SZ), or null. `name` null reads the key's default value. */
export function regValue(key, name) {
  const r = run('reg.exe', ['query', key, ...(name === null ? ['/ve'] : ['/v', name])]);
  if (!r.ok) return null;
  const rows = r.stdout.split(/\r?\n/).map((l) => l.match(/^\s+(.+?)\s+REG_\w+\s+(.*)$/)).filter(Boolean);
  // The default value's name is localized ("(Default)", "(Standard)", ...): with /ve it is the only row.
  return (name === null ? rows[0] : rows.find((m) => m[1] === name))?.[2] ?? null;
}

/** Your account's SID (Windows), or null. */
export function currentUserSid() {
  if (process.platform !== 'win32') return null;
  const whoami = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'whoami.exe');
  const r = run(whoami, ['/user', '/fo', 'csv', '/nh']);
  return r.stdout.match(/"(S-1-[\d-]+)"/)?.[1] ?? null;
}
