// Repo keys, which a project's own host computes so that checkouts of one
// repo on different machines can be bridged (contracts, section 2), and
// the ids hosts get. Everything here is made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { repoKey, repoId, isHostId } from "../src/shared/ids.js";
const KEY = "github.com/willctl/xenonet";

test("a repo key is the same for every way of writing the remote", () => {
  for (const url of [
    "git@github.com:willctl/xenonet.git",
    "git@github.com:willctl/xenonet",
    "https://github.com/willctl/xenonet",
    "https://github.com/willctl/xenonet.git",
    "https://github.com/willctl/xenonet/",
    "https://nova@github.com/willctl/xenonet",
    "https://nova:not-a-token@github.com/willctl/xenonet.git",
    "ssh://git@github.com/willctl/xenonet.git",
    "ssh://git@github.com:22/willctl/xenonet.git",
    "git+ssh://git@github.com:2222/willctl/xenonet",
    "GIT@GitHub.com:WillCtl/XenoNet.GIT",
    "  git@github.com:willctl/xenonet.git\n",
  ])
    assert.equal(repoKey(url), KEY, url);
  assert.equal(repoId(`https://${KEY}`), `repo:${KEY}`, "a merged view's id is repo:<key>");
});

test("whole paths are kept, and remotes that are folders on disk have no key", () => {
  assert.equal(repoKey("https://gitlab.example/group/sub/repo.git"), "gitlab.example/group/sub/repo");
  assert.equal(repoKey("git@gitlab.example:group/sub/repo.git"), "gitlab.example/group/sub/repo");
  for (const url of ["file:///srv/git/atlas.git", "/srv/git/atlas.git", "../atlas", "C:\\repos\\atlas", "C:/repos/atlas", "", null, undefined, "https://github.com/", "git@github.com:"]) assert.equal(repoKey(url), null, String(url));
});

test("host ids are names people choose, never a machine's own", () => {
  for (const ok of ["vm1", "studio", "demo-b"]) assert.ok(isHostId(ok), ok);
  for (const bad of ["Will-MacBook-Pro", "will's mac", "vm1.local", "x"]) assert.ok(!isHostId(bad), bad);
});
