# What XenoNet is for

Will's statement of the project's goals, written down on 2026-11-08 so every
design, plan and PR can be checked against it. Change this file only when
Will changes the goals. `direction.md` beside it says how the current code
measures up and what to build next.

## The one-sentence version

XenoNet is a system, a data model and a set of protocols that let actors
(people and bots) collaborate to take an intent to an outcome, securely,
across machines. To get there they use three kinds of thing, each within
what they are allowed:

- **Hosts**: the machines and other resources work runs on.
- **Identities**: the accounts work is done as.
- **Running sessions**: the agents and harnesses doing the work.

The console, the CLI and the MCP servers are surfaces onto that system. They
matter, but they are not the core. Xenolith, the shared context, is one of the
system's layers and follows the same model.

## Goals

1. **Collaboration across machines is the core.** A session on one machine
   and an actor on another can see each other, message each other, share
   context and hand off work, with the same tools and the same guarantees as
   two sessions on one machine.
2. **Many-to-many everywhere.** An identity (a person, an account, an
   organization) can own or belong to many projects, machines and context
   layers, and each of those can belong to many identities. Nothing in the
   model assumes one person, one machine, one network, one projects folder or
   one shared context.
3. **Secure by construction.** Every actor, agent, machine and identity has a
   key or a credential of its own. Every action names who did it and under
   what authority. Authority is granted explicitly, is as narrow as the task,
   can be delegated no wider than it was given, and can be revoked. A relay
   can carry things it cannot read or forge. The person stays in charge of
   what admits, widens or publishes.
4. **High performance.** The hot paths are push, not poll. A message between
   two agents on one LAN arrives in well under a second. Reading state is
   cheap enough that agents can do it on every turn.
5. **Distributed and scalable.** Each machine works alone and catches up when
   it reconnects. No part is a single point of failure unless the person
   chooses one. The design holds at 50 machines, 1,000 live sessions and
   10,000 messages an hour, and degrades gracefully past that.
6. **Modular and composable.** The core is a set of crates with one job each,
   typed interfaces, and no knowledge of the surfaces or the vendors. Another
   project can take the network crate, the authorization crate or the data
   model alone. Vendor adapters (Claude Code, Codex, Grok Build, Hermes and
   the rest) are plugins to the core, never conditions inside it.
7. **Seamless to set up and integrate.** Installing XenoNet on a machine and
   connecting a platform to it is one command each, with the doctor saying
   exactly what is wrong when it is not. Joining a network is one command.
   The same goes for a new platform: adding one means writing an adapter
   against one documented interface.
8. **Reuse before reinvent.** Where a protocol, a format or a library already
   solves a problem well (agent interop, identity, authorization,
   transport, sync, verification), XenoNet adopts it or borrows its design
   and says so. XenoNet's own protocols cover only what nothing else does.
9. **Verified where a bug costs most.** The network protocol, the
   authorization rules and the coordination guarantees (claims, delivery,
   joining) have formal models or machine-checked properties, kept in step
   with the code, runnable in CI.
10. **Xenolith follows the same model.** Shared context has the same
    identities, spaces and grants as the rest of XenoNet, and syncs over the
    same network.

## Non-goals, for now

- **Backwards compatibility.** XenoNet is a prototype used by one person.
  Major redesigns and refactors are welcome. Migrations are nice to have, not
  required.
- **The console as the product.** The map is a surface. Its needs do not
  shape the data model.
- **Will's own setup.** Nothing assumes Tailscale, macOS, a particular
  folder or a particular account. Every such choice is the user's, with
  options for anyone ([xenonet-public-defaults](../../AGENTS.md)).

## The alignment check

Every design doc, plan and non-trivial PR answers these before it is built:

- Which goal does this serve? If none, why build it?
- How does it help actors take an intent to an outcome, and which hosts,
  identities and sessions does it use?
- Does it add a one-of-anything assumption (one person, one machine, one
  network, one gateway, one folder)? If so, goal 2 says no.
- Who can do this, to whom, under what authority, and how is that authority
  named and checked? (Goal 3.)
- Does it poll where it could push, or scan where it could index? (Goals 4
  and 5.)
- Does it put vendor or surface knowledge inside a core crate? (Goal 6.)
- Is there an existing protocol or library for this? If we reinvent, say why.
  (Goal 8.)
- Which property should hold, and how will we know it does: a test, a
  property test, a model? (Goal 9.)

## Related

- [direction.md](direction.md): where the code stands against these goals,
  the target architecture and the order of work.
- [network.md](network.md), [contracts.md](contracts.md),
  [rust-core.md](rust-core.md): the designs built so far.
- [../guide/related-work.md](../guide/related-work.md): protocols and
  projects XenoNet already looked at.
