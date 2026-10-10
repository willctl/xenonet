// gh's hosts.yml, as core/identities.js reads it: the hosts gh is signed in
// to, and the accounts on each. XenoNet reads the file a line at a time and
// never a value but a host or a login name, so a token in it stays there.
import { vendor, open, str, z } from "../common.js";

export const Hosts = vendor(
  "github/gh/hosts",
  1,
  z
    .record(
      str.describe("A host, such as github.com"),
      open({
        user: str.optional().describe("The active account"),
        users: z.record(str.describe("An account's login"), z.unknown()).optional().describe("Every account signed in on the host (gh 2.40 and later)"),
      }),
    )
    .describe("The file's YAML as JSON, keys only: each value XenoNet doesn't read stands as an empty string"),
  {
    title: "gh's hosts.yml",
    description: "gh's hosts.yml ($GH_CONFIG_DIR, or the XDG or platform config folder), hosts and logins only. The file carries no version, so the newest schema reads it.",
    versions: "*",
  },
);
