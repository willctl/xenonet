// project/v1: a project or group as the scan finds it (scanProjects in
// core/projects.js), with the rule that made it one, if any (state.js adds
// it). The map's project nodes (graph/v1) are these with their place and
// state added.
import { define } from "./registry.js";
import { z, obj, ms, nstr } from "./common.js";

export const Rule = obj({
  path: z.string(),
  as: z.enum(["project", "group", "ignore"]),
  by: z.enum(["you", "codex"]),
  at: ms.optional().describe("When you set it; yours only"),
  reason: z.string().optional().describe("Why Codex picked it; Codex's only"),
});

export const Project = define(
  "project",
  1,
  obj({
    name: z.string(),
    path: z.string().describe("The folder, spelled as on disk"),
    key: z.string().describe("The folder's path key (core/paths.js): forward slashes, lowercase, no trailing slash"),
    rel: z.string().describe("The folder inside the projects folder, with forward slashes"),
    role: z.enum(["project", "group"]),
    detected: z
      .enum(["repo", "rule", "submodule", "workspace", "sessions", "folder", "contains"])
      .describe("What made it one: a project's own repo, a rule, being a submodule, a workspace file, sessions run in it, or being a plain top-level folder; a group's projects (contains) or a rule"),
    marker: nstr.describe("The file that marks it: a version control folder (.git, .hg, …) or a workspace file"),
    parent: nstr.describe("The key of the group or project it's inside"),
    git: obj({
      branch: nstr,
      origin: nstr.describe("The origin remote's URL"),
      worktreeOf: nstr.describe("For a linked worktree, the folder of the repo it belongs to"),
      submoduleOf: nstr,
    }).nullable(),
    stack: z.array(z.string()).describe("What it's built with, from marker files: node, typescript, python…"),
    description: nstr,
    modifiedAt: ms.nullable(),
    rule: Rule.nullable().optional().describe("The rule that applies to its folder: yours, or one of Codex's picks"),
  }),
  {
    title: "Project",
    description: "A project, or a group of projects, in the projects folder, as XenoNet's scan finds it.",
  },
);
