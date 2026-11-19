# Adytum release notes

## 0.1.0 (unreleased)

What you can do.

- Make, start, stop, clone and remove sealed cells on your machine with `adytum`, each behind a wall you name.
- Run a program in a cell with `adytum exec` and copy files in and out with `adytum cp`.
- Ask `adytum doctor` what is wrong with the setup. Each problem comes with its fix.
- Use the cells for [Eidolon](../eidolon/RELEASES.md), which opens an identity's state in one.

What to know before you install.

- Cells are Lima VMs, so Lima 2.1.2 has to be installed. Adytum won't install it.
- A VM needs 20 GiB free where `~/.adytum` lives. Set `ADYTUM_HOME` to a short absolute path to keep it elsewhere.
- Adytum builds from this repository with `npm run cargo -- build -p adytum-cli`. There is no installer yet.
- There is no Windows or WSL driver. Container cells (podman and docker) exist in the library but the `adytum` command doesn't make them, and a container is a weaker wall than a VM.

What you must do.

- Run `adytum doctor` once and follow what it says.
- Name cells starting with `adytum-`. Adytum refuses any other name, and it doesn't see the VMs in Lima's default folder.
- Pick a wall for each cell. Cells without one share `wall_default`.
