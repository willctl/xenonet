# Adytum, sealed cells on your machine

Adytum makes cells on your machine. A cell is a VM with one purpose, such as holding one identity's browser, and every cell sits behind a wall, a label that keeps cells with different labels apart. XenoNet decides who may act across machines, and Adytum only works for you on the machine you run it on.

This page is for using Adytum with XenoNet. [The README](../../adytum/README.md) has the command list and the crates.

## Setting up

Adytum is built from this repository and needs [Lima](https://lima-vm.io) 2.1.2 for its VMs.

```text
npm run cargo -- build -p adytum-cli -p eidolon-cli
adytum doctor
```

`adytum doctor` checks Lima's version, Adytum's own Lima folder, the pinned image in Lima's cache, the free disk (20 GiB for a VM) and the ledger. Each problem comes with the command that fixes it. It exits 1 when a check fails. `xenonet doctor` reports whether Adytum and Eidolon are present too.

Adytum keeps everything under `~/.adytum`, or under `ADYTUM_HOME` if you set it. Lima runs from `~/.adytum/lima`, so the VMs you keep in Lima's default folder are not listed and can't be reached by a typo.

## A first cell

```text
adytum create adytum-web --sealed --wall wall_work
adytum start adytum-web
adytum exec adytum-web -- uname -a
adytum ls
```

- The name starts with `adytum-`. Adytum touches no other name.
- `--sealed` means no folder of your machine is mounted into the cell. Files get in and out with `adytum cp`, where one side is `<name>:<absolute path>`.
- `--wall` is the label. Without it, the cell gets `wall_default`.
- `adytum exec` takes a program and its arguments after `--`. Nothing runs through a shell on your machine.

Stop and remove it when you are done.

```text
adytum stop adytum-web
adytum rm adytum-web
```

## Ports

`--port 18080:80` publishes the cell's port 80 on your port 18080, on 127.0.0.1 only, so nothing outside your machine can reach it. `--automation-port N` names a port that a program inside the cell uses for automation. It is never published.

## Copies

`adytum clone adytum-web adytum-web2` copies a stopped cell. The copy keeps the wall. A clone into a different wall is refused, and so is a clone of a cell that holds an open eidolon. A published host port belongs to the original, so give the copy its own with `--port`.

## If Adytum is stopped halfway

Adytum writes a record before it makes or removes a cell and settles it after. Every command compares those records with what Lima reports, finishes or removes the difference, and says so on standard error. A cell it never recorded is listed by `adytum ls` and never touched.

## Security

What Adytum protects.

- A sealed cell can't read your files. It has no mount from your machine, and `adytum create` refuses to add one. Files cross only through `adytum cp`, which you run.
- A cell's ports can't be reached from the network. Every published port is bound to 127.0.0.1, and the automation port is not published, so a program that drives a browser in a cell does so over a stream through `adytum exec`.
- Two walls stay apart. Cells with different walls share no network, no mount source, no clone source and no secret, and Adytum refuses a create or a clone that would break that.
- Adytum can't harm your other VMs. It touches only names that start with `adytum-`, and it runs Lima in its own folder.
- A command runs as a program with a list of arguments and an empty environment plus the variables named for it. Nothing you pass is read as shell syntax, and the whole process group is stopped when the deadline passes.

What it doesn't protect.

- A VM is a strong wall, not a perfect one. A flaw in the hypervisor reaches your machine.
- A container is a weaker wall than a VM, because it shares your machine's kernel. Adytum can make podman and docker cells in its library, but the `adytum` command makes Lima cells only for now.
- A wall is a label. Adytum never learns what it means, so who may use which wall is XenoNet's decision, not Adytum's.
- Anyone who can run commands as you can run `adytum` as you.

There is no Windows or WSL driver yet.

## Related

- [Eidolon](eidolon.md), which keeps a browser profile or a folder in a cell and moves it between machines.
- [Release notes](../../adytum/RELEASES.md).
- [The design](../design/cells.md).
