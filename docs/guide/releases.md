# Releasing XenoNet

XenoNet ships a first-install bundle and a Desktop-only update, plus separate Service and Tools archives. Desktop runs on Windows x64 and macOS arm64/x64. Service and Tools also ship for Linux x64. See [components](components.md) for installation and migration. GitHub Releases holds the installers, ZIP update payloads, blockmaps and update manifests. The app reads published stable releases from `willctl/xenonet`. It ignores drafts and prereleases and does not downgrade.

## Prepare a version

1. Choose a new version. Use a patch for fixes and a minor version for new behavior while XenoNet is below 1.0.
2. Run `npm version 0.1.1 --no-git-tag-version`, with the Desktop version you chose. Update `components/versions.json` for each changed Service or Tools package. Keep unchanged component versions unchanged; changing their runtime or shared code requires a new component version. This updates both package files.
3. Write `docs/releases/0.1.1.md`. Describe what changed for the person using XenoNet, any install limits and anything they must do. Review public copy with the [unslop guidance](https://github.com/cursor/plugins/blob/main/pstack/skills/unslop/SKILL.md).
4. Run `npm run release:check`, `npm test` and `npm run test:e2e`. Check the desktop UI in the demo instance when it changed.
5. Commit the changes, push them, then create and push an annotated tag that matches the package version.

```sh
git tag -a v0.1.1 -m "XenoNet 0.1.1"
git push origin v0.1.1
```

The Release workflow runs unit tests on Windows, macOS and Linux and the updater's desktop UI tests on Windows. Run the full `npm run test:e2e` suite on a machine with a GPU before tagging. The component lifecycle test also runs against an unpacked Windows build by setting `XENONET_TEST_PACKAGED_EXE` to its executable. This verifies a persistent shell and a live console MCP connection across Desktop restarts. Software rendering on standard hosted runners does not reliably meet the 3D interaction tests' settling and screenshot expectations; those tests stay in the local release checklist.

Installer builds run alongside the hosted checks on Windows and macOS. Vite produces the production page; electron-builder bundles the runtime and production dependencies, with maximum archive compression and only the required terminal module for each CPU. Existing package checks reject the builder's home path in bundled text.

Each Mac architecture has a bundle disk image and a Desktop-only ZIP. Windows has a bundle installer and a Desktop-only installer; `latest.yml` must point to the latter. Each component archive includes a pinned Node runtime whose download is checked against Node's official SHA-256 list. The component build uses fixed archive timestamps. Assembly checks the manifest's version, file size and SHA-512 hash before it combines the Mac manifests. It verifies every Service and Tools archive for all supported targets, writes `components.json`, then writes SHA-256 checksums. No build job publishes files directly.

## Publish

When every job passes, the workflow creates a draft named `XenoNet <version>` with the notes and all downloads attached. Download the installers and check launch, terminal startup and the Updates panel on the target operating systems. Publish the draft from GitHub Releases when it is ready. The draft keeps an incomplete build out of installed apps' update checks.

You can rerun a failed workflow against the same tag. It may replace files on a draft. It refuses to replace a published release. Fix a published build with a new version and tag; do not move tags or overwrite downloads.

To try the release build before tagging, run the Release workflow manually on a branch. It uploads the installers as workflow artifacts and creates no release. This lets you check packaging before choosing a permanent version tag.

For a local build of the same installers, run `npm run release:build` on Windows or macOS. It never publishes. Set `XENONET_RELEASE_DIR` to a fresh output directory if an unpacked app in `release/` is running. `npm run dist` remains available for local installers, including the Mac PKG.

The build compares component versions with the latest published manifest. An unchanged version must produce the same archive hash. If Service or Tools code changes, bump that component in `components/versions.json`; changing only Desktop does not require a component version bump.

## Signing

Unsigned builds are the current default. XenoNet does not choose a certificate from the build machine without an explicit opt-in. A certificate can publish its owner's legal name, so choose the public publisher identity before setting this up.

For Windows, add the repository secrets `WIN_CSC_LINK` and `WIN_CSC_KEY_PASSWORD`, then set repository variable `WIN_SIGN` to `1`. electron-builder embeds the signing publisher in the update configuration and electron-updater checks it on subsequent downloads.

For macOS, add `MAC_CSC_LINK`, `MAC_CSC_KEY_PASSWORD`, `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD` and `APPLE_TEAM_ID`. Use a Developer ID Application certificate, then set repository variable `MAC_SIGN` to `1`. The workflow requires signing and notarization and marks that build as able to install updates. It fails if the signing setup is incomplete. Never put certificate contents or passwords in the repo.

The current unsigned Windows installer can update, but has no publisher signature to verify. It checks the download against GitHub's SHA-512 manifest over HTTPS. Unsigned Mac builds only check for releases; they never ask Apple's updater to install an ad-hoc signed bundle. Moving to signed Mac builds needs a one-time manual install.

## App behavior

Installed copies check 30 seconds after startup and every six hours. Supported builds download in the background. Updates never install just because the app quits. The user chooses Restart and update. Desktop restarts while the independent service, terminals and connected MCP processes continue. The installer preserves settings and project data.

Updates in the command palette and native menus shows the installed version, check result, download progress and restart action. Turning off automatic checks also stops automatic downloads. Manual checks still work. Service and Tools updates are listed separately and installed on request. If a service update finds running terminals, it stages the download; close them and run the update again. Source, demo and test copies never contact the release feed or install updates.

## If a release is broken

Stop further distribution by marking it as a prerelease in GitHub, then publish a fixed version with a higher number. Already downloaded updates stay on the user's machine, and installed copies do not roll back automatically. Keep the older installers available for a deliberate manual reinstall. Check data compatibility before recommending an older app.
