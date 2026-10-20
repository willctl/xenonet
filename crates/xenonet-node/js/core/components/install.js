// core/components/install.js in Rust (crates/xenonet-components/src/install.rs):
// installing and switching components. A bundle installs what's missing and
// never upgrades what's there; a runtime that fails its startup check is
// never selected; a service update keeps a record until the new version
// answers, and rolls back when it doesn't. What it waits on (the service,
// starting a runtime, the clock) is done here, with the operations given.
import fs from "node:fs";
import os from "node:os";
import { native } from "../../index.js";
import { settle } from "../../settle.js";
import { drive } from "../../task.js";
import { envOf, machine } from "../../machine.js";
import { lifecycleOps, stopTimeout } from "../../lifecycle.js";
import { ensureService, connectService } from "./runtime.js";

// The node launcher script this XenoNet ships, which entries an earlier
// XenoNet wrote run; the Rust has it built in.
export const LAUNCHER_SOURCE = new URL("../../../../../core/components/launcher.cjs", import.meta.url);

const ctx = () => ({ ...machine(), localAppData: process.env.LOCALAPPDATA ?? null, home: os.homedir() });

export const launcherVersion = (text) => native.compLauncherVersion(text == null ? null : String(text));

export const writeLauncher = (entry, { source = fs.readFileSync(LAUNCHER_SOURCE, "utf8") } = {}) => settle(native.compWriteLauncher(String(entry), String(source)));

export const installLauncher = (env = process.env) => settle(native.compInstallLauncher(envOf(env), ctx()));

export const checkRuntime = (staged) => settle(native.compCheckRuntime(staged, process.platform));

export const installedRuntime = (env = process.env) => settle(native.compInstalledRuntime(envOf(env), ctx()));

const defaults = { connect: connectService, start: ensureService, check: checkRuntime };

export const recoverServiceUpdate = ({ env = process.env, operations } = {}) => drive(native.compRecover(envOf(env), ctx(), stopTimeout(operations)), lifecycleOps(defaults, operations, env));

export const installBundle = (bundle, { env = process.env, command = true } = {}) => drive(native.compInstallBundle(String(bundle), envOf(env), !!command, ctx(), stopTimeout()), lifecycleOps(defaults, undefined, env));

export const activatePackage = (file, item, { env = process.env, operations } = {}) => drive(native.compActivate(String(file), item, envOf(env), ctx(), stopTimeout(operations)), lifecycleOps(defaults, operations, env));
