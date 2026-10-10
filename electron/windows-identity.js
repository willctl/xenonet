// Windows reads notification branding from the AUMID registration. Electron
// separately reuses the activator on its product-name Start menu shortcut.
import { execFileSync } from "node:child_process";
import path from "node:path";

export function windowsToastActivator({ productName, fallback, readShortcut, programs, run = execFileSync }) {
  try {
    // Use the known folder, which can be redirected away from AppData.
    programs ??= run("powershell.exe", ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "[Environment]::GetFolderPath('Programs')"], { windowsHide: true, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 10000 }).trim();
    const existing = readShortcut(path.join(programs, `${productName}.lnk`)).toastActivatorClsid;
    // Electron 44 adopts this value even if the shortcut's AUMID changed.
    // Reading the same value avoids modifying someone else's shortcut.
    if (/^\{?[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}\}?$/i.test(existing)) return existing;
  } catch {}
  return fallback;
}

export function registerWindowsIdentity({ id, icon, activator, platform = process.platform, run = execFileSync }) {
  if (platform !== "win32") return;
  const key = `HKCU\\Software\\Classes\\AppUserModelId\\${id}`;
  for (const [name, value] of [["DisplayName", "XenoNet"], ["IconUri", path.resolve(icon)], ...(activator ? [["CustomActivator", activator]] : [])]) {
    run("reg.exe", ["add", key, "/v", name, "/t", "REG_SZ", "/d", value, "/f"], { windowsHide: true, stdio: "pipe", timeout: 10000 });
  }
}
